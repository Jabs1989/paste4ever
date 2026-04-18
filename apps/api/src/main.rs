//! Paste4Ever API
//!
//! Thin Rust gateway between the Next.js frontend and the local `antd` daemon
//! (the Autonomi network gateway). All P2P / wallet / payment logic lives in
//! `antd`; this service just proxies pastes to/from it and adds the Paste4Ever-
//! specific business logic (size limits, future rate limiting, etc).

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use parking_lot::Mutex;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

#[derive(Clone)]
struct AppState {
    /// Base URL of the local antd daemon, e.g. http://localhost:8082
    antd_url: String,
    http: reqwest::Client,
    /// Local SQLite index of pastes made through this API. The index is just
    /// a cache for the /recent wall — the actual paste bytes live on Autonomi
    /// and remain retrievable by address even if this DB is wiped.
    db: Arc<Mutex<Connection>>,
    /// Rolling count of consecutive *fully-failed* paste uploads (all retries
    /// exhausted). Reset to 0 on any successful upload. Used by /health so
    /// the external watchdog can restart antd when its DHT rots, and by the
    /// frontend to warn users before they waste a click.
    consecutive_failures: Arc<AtomicU32>,
}

// ── SQLite schema ─────────────────────────────────────────────────────────
/// Returns an 80-char preview with newlines collapsed, plus an ellipsis if
/// the original was longer. The preview lives in our DB so the wall can
/// render instantly without round-tripping back to Autonomi for every card.
fn make_preview(content: &str) -> String {
    const MAX: usize = 80;
    let flat: String = content
        .chars()
        .map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c })
        .collect();
    let trimmed = flat.trim();
    if trimmed.chars().count() <= MAX {
        trimmed.to_string()
    } else {
        let head: String = trimmed.chars().take(MAX).collect();
        format!("{}…", head)
    }
}

fn init_db(path: &str) -> anyhow::Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS pastes (
            id          TEXT PRIMARY KEY,
            created_at  INTEGER NOT NULL,
            size_bytes  INTEGER NOT NULL,
            preview     TEXT NOT NULL,
            listed      INTEGER NOT NULL DEFAULT 1
        );
        CREATE INDEX IF NOT EXISTS idx_pastes_listed_created
            ON pastes(listed, created_at DESC);
        "#,
    )?;
    Ok(conn)
}

#[derive(Deserialize)]
struct CreatePasteRequest {
    content: String,
}

#[derive(Serialize)]
struct CreatePasteResponse {
    /// Autonomi data address (hex). Doubles as the paste ID — visible in the URL.
    id: String,
}

#[derive(Serialize)]
struct GetPasteResponse {
    content: String,
}

// ── antd request/response shapes ──────────────────────────────────────────
#[derive(Serialize)]
struct AntdPutRequest {
    data: String, // base64-encoded
}

#[derive(Deserialize)]
struct AntdPutResponse {
    address: String,
    #[allow(dead_code)]
    chunks_stored: Option<u64>,
    #[allow(dead_code)]
    payment_mode_used: Option<String>,
}

#[derive(Deserialize)]
struct AntdGetResponse {
    data: String, // base64-encoded
}

// ── Handlers ──────────────────────────────────────────────────────────────

/// Health signal for the homepage badge and the external antd watchdog.
///
/// Two things are checked, cheaply:
/// 1. TCP/HTTP reachability of antd — any HTTP response (even a 404 on /) is
///    enough to prove the daemon process is alive and listening.
/// 2. The rolling count of *fully-failed* paste uploads since the last success.
///    This is the real signal that the DHT has rotted: antd is still up and
///    accepting HTTP, but K-buckets are full of stale peers and no upload can
///    land. When `consecutive_failures` reaches 2+, the watchdog should
///    restart antd.
#[derive(Serialize)]
struct HealthResponse {
    status: &'static str, // "healthy" | "degraded"
    antd_reachable: bool,
    consecutive_failures: u32,
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    // Probe antd with a short timeout. We don't care about the status code,
    // only that the TCP handshake + HTTP response completed quickly.
    let probe = state
        .http
        .get(&state.antd_url)
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await;
    let antd_reachable = probe.is_ok();
    let consecutive_failures = state.consecutive_failures.load(Ordering::Relaxed);
    let status = if antd_reachable && consecutive_failures < 2 {
        "healthy"
    } else {
        "degraded"
    };
    Json(HealthResponse {
        status,
        antd_reachable,
        consecutive_failures,
    })
}

/// Max number of attempts for a paste upload.
///
/// Autonomi in "early days" (dixit storage_guy on the #ant Discord) sometimes
/// returns `partial upload: N stored, M failed: chunk storage failed after retries`
/// or just silently stalls after payment went through. Retries ARE safe: the
/// network is idempotent on content — if the quote was already paid, resubmitting
/// the same bytes does NOT charge again, it just completes the stuck storage step.
const UPLOAD_MAX_ATTEMPTS: u32 = 3;
/// Seconds to wait between retry attempts (applied before attempts 2..=N).
/// Exponential-ish: 15s, 45s. Short enough to stay within a reasonable UX window,
/// long enough that a transient chunk-storage flap has time to clear.
const UPLOAD_RETRY_DELAYS_SECS: &[u64] = &[15, 45];

async fn create_paste(
    State(state): State<AppState>,
    Json(req): Json<CreatePasteRequest>,
) -> impl IntoResponse {
    if req.content.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Content required" })),
        )
            .into_response();
    }
    if req.content.len() > 100_000 {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({ "error": "Content too large (max 100KB)" })),
        )
            .into_response();
    }

    let body = AntdPutRequest {
        data: B64.encode(req.content.as_bytes()),
    };

    let url = format!("{}/v1/data/public", state.antd_url);
    let mut last_error_detail: Option<String> = None;

    for attempt in 1..=UPLOAD_MAX_ATTEMPTS {
        if attempt > 1 {
            let delay = UPLOAD_RETRY_DELAYS_SECS
                .get((attempt - 2) as usize)
                .copied()
                .unwrap_or(60);
            tracing::warn!(
                "⏳ retry {}/{} in {}s (warning: antd may re-pay for chunks on retry)",
                attempt,
                UPLOAD_MAX_ATTEMPTS,
                delay
            );
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        }
        tracing::info!(
            "→ POST {} ({} bytes raw) — attempt {}/{}",
            url,
            req.content.len(),
            attempt,
            UPLOAD_MAX_ATTEMPTS
        );

        let res = match state.http.post(&url).json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
                // Connection/timeout error — transient, retry.
                tracing::warn!("antd unreachable on attempt {}: {}", attempt, e);
                last_error_detail = Some(format!("transport: {}", e));
                continue;
            }
        };

        let status = res.status();
        if !status.is_success() {
            let detail = res.text().await.unwrap_or_default();
            tracing::warn!("antd {} on attempt {}: {}", status, attempt, detail);
            last_error_detail = Some(format!("{}: {}", status, detail));
            // 5xx = server/network problem → retry. 4xx = client problem → give up.
            if status.is_server_error() {
                continue;
            }
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "Upload rejected by storage daemon",
                    "detail": detail
                })),
            )
                .into_response();
        }

        let parsed: AntdPutResponse = match res.json().await {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("antd response parse error on attempt {}: {}", attempt, e);
                last_error_detail = Some(format!("parse: {}", e));
                continue;
            }
        };

        tracing::info!(
            "✅ stored at {} (attempt {}/{})",
            parsed.address,
            attempt,
            UPLOAD_MAX_ATTEMPTS
        );
        // Reset the health counter — any success means the DHT is usable again.
        state.consecutive_failures.store(0, Ordering::Relaxed);

        // Record the paste in our local index so it shows up on the public wall.
        // Best-effort: a DB error here must NOT fail the user's paste, so we
        // just log and move on — the bytes are already safely on Autonomi.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let preview = make_preview(&req.content);
        let size = req.content.len() as i64;
        let id_for_db = parsed.address.clone();
        let db = state.db.clone();
        if let Err(e) = (|| -> rusqlite::Result<()> {
            let conn = db.lock();
            conn.execute(
                "INSERT OR IGNORE INTO pastes (id, created_at, size_bytes, preview, listed)
                 VALUES (?1, ?2, ?3, ?4, 1)",
                rusqlite::params![id_for_db, now, size, preview],
            )?;
            Ok(())
        })() {
            tracing::warn!("failed to index paste {} in wall DB: {}", parsed.address, e);
        }

        return (
            StatusCode::OK,
            Json(CreatePasteResponse { id: parsed.address }),
        )
            .into_response();
    }

    // All attempts exhausted — flip the health counter so the watchdog and
    // the frontend badge both see the degraded state.
    let new_count = state.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
    tracing::error!(
        "❌ upload failed after {} attempts — last error: {:?} (consecutive_failures={})",
        UPLOAD_MAX_ATTEMPTS,
        last_error_detail,
        new_count
    );
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({
            "error": "Autonomi network is currently congested. Your paste could not be stored after multiple attempts — please try again in a few minutes.",
            "detail": last_error_detail,
            "attempts": UPLOAD_MAX_ATTEMPTS,
        })),
    )
        .into_response()
}

/// Reads can also flake (peer holding the chunk briefly unreachable). Shorter
/// backoff than uploads — no payment involved, just retry fast.
const GET_MAX_ATTEMPTS: u32 = 3;
const GET_RETRY_DELAYS_SECS: &[u64] = &[5, 15];

async fn get_paste(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let url = format!("{}/v1/data/public/{}", state.antd_url, id);
    let mut last_error_detail: Option<String> = None;

    for attempt in 1..=GET_MAX_ATTEMPTS {
        if attempt > 1 {
            let delay = GET_RETRY_DELAYS_SECS
                .get((attempt - 2) as usize)
                .copied()
                .unwrap_or(30);
            tracing::warn!("⏳ GET retry {}/{} in {}s", attempt, GET_MAX_ATTEMPTS, delay);
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        }
        tracing::info!("→ GET {} — attempt {}/{}", url, attempt, GET_MAX_ATTEMPTS);

        let res = match state.http.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("antd unreachable on attempt {}: {}", attempt, e);
                last_error_detail = Some(format!("transport: {}", e));
                continue;
            }
        };

        let status = res.status();

        // Definitive 404 — paste doesn't exist. Don't retry.
        if status == StatusCode::NOT_FOUND {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Paste not found" })),
            )
                .into_response();
        }

        if !status.is_success() {
            let detail = res.text().await.unwrap_or_default();
            tracing::warn!("antd {} on attempt {}: {}", status, attempt, detail);
            last_error_detail = Some(format!("{}: {}", status, detail));
            if status.is_server_error() {
                continue;
            }
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({ "error": "Paste not found", "detail": detail })),
            )
                .into_response();
        }

        let parsed: AntdGetResponse = match res.json().await {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("antd response parse error on attempt {}: {}", attempt, e);
                last_error_detail = Some(format!("parse: {}", e));
                continue;
            }
        };

        let bytes = match B64.decode(&parsed.data) {
            Ok(b) => b,
            Err(e) => {
                tracing::error!("base64 decode error: {}", e);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": "Corrupt paste data" })),
                )
                    .into_response();
            }
        };
        let content = String::from_utf8_lossy(&bytes).into_owned();

        return (StatusCode::OK, Json(GetPasteResponse { content })).into_response();
    }

    tracing::error!(
        "❌ fetch failed after {} attempts — last error: {:?}",
        GET_MAX_ATTEMPTS,
        last_error_detail
    );
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({
            "error": "Could not retrieve paste from the Autonomi network after multiple attempts — please try again shortly.",
            "detail": last_error_detail,
            "attempts": GET_MAX_ATTEMPTS,
        })),
    )
        .into_response()
}

// ── Wall of pastes ────────────────────────────────────────────────────────
#[derive(Serialize)]
struct RecentPaste {
    id: String,
    created_at: i64,
    size_bytes: i64,
    preview: String,
}

#[derive(Deserialize)]
struct RecentQuery {
    limit: Option<u32>,
}

async fn recent_pastes(
    State(state): State<AppState>,
    Query(q): Query<RecentQuery>,
) -> impl IntoResponse {
    // Clamp to a sane range. Homepage shows ~20; max 100 for future API consumers.
    let limit = q.limit.unwrap_or(20).clamp(1, 100) as i64;

    let rows = {
        let conn = state.db.lock();
        let result = (|| -> Result<Vec<RecentPaste>, rusqlite::Error> {
            let mut stmt = conn.prepare(
                "SELECT id, created_at, size_bytes, preview
                 FROM pastes
                 WHERE listed = 1
                 ORDER BY created_at DESC
                 LIMIT ?1",
            )?;
            let iter = stmt.query_map([limit], |row| {
                Ok(RecentPaste {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    size_bytes: row.get(2)?,
                    preview: row.get(3)?,
                })
            })?;
            iter.collect::<Result<Vec<_>, _>>()
        })();
        drop(conn);
        result
    };

    match rows {
        Ok(list) => (StatusCode::OK, Json(list)).into_response(),
        Err(e) => {
            tracing::error!("recent_pastes DB error: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": "Could not load recent pastes" })),
            )
                .into_response()
        }
    }
}

// ── main ──────────────────────────────────────────────────────────────────
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "paste4ever_api=debug,tower_http=info".into()),
        )
        .init();

    let antd_url = std::env::var("ANTD_URL").unwrap_or_else(|_| "http://localhost:8082".to_string());
    tracing::info!("🌐 antd gateway: {}", antd_url);

    // Autonomi uploads (quote → pay on Arbitrum → store chunks across peers)
    // can legitimately take 2-4 minutes on first-seen content, but when the
    // DHT has stale peers a successful upload can stretch to 10-15 minutes
    // while antd retries internally. We set a generous 20-minute ceiling so
    // we don't cut off antd mid-DataMap and trigger a *second* payment on our
    // own retry path. Real stalls are caught by antd-side timeouts before us.
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(1200))
        .build()?;

    let db_path = std::env::var("PASTES_DB").unwrap_or_else(|_| "pastes.db".to_string());
    tracing::info!("📒 wall index DB: {}", db_path);
    let db = Arc::new(Mutex::new(init_db(&db_path)?));

    let state = AppState {
        antd_url,
        http,
        db,
        consecutive_failures: Arc::new(AtomicU32::new(0)),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/", get(health))
        .route("/health", get(health))
        .route("/paste", post(create_paste))
        .route("/paste/:id", get(get_paste))
        .route("/recent", get(recent_pastes))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    println!("🚀 Paste4Ever API listening on http://localhost:{}", port);
    axum::serve(listener, app).await?;
    Ok(())
}
