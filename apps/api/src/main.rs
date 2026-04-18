//! Paste4Ever API
//!
//! Thin Rust gateway between the Next.js frontend and the local `antd` daemon
//! (the Autonomi network gateway). All P2P / wallet / payment logic lives in
//! `antd`; this service just proxies pastes to/from it and adds the Paste4Ever-
//! specific business logic (size limits, future rate limiting, etc).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

#[derive(Clone)]
struct AppState {
    /// Base URL of the local antd daemon, e.g. http://localhost:8082
    antd_url: String,
    http: reqwest::Client,
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
async fn health() -> &'static str {
    "Paste4Ever API — OK"
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
                "⏳ retry {}/{} in {}s (autonomi idempotent — no double charge)",
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
        return (
            StatusCode::OK,
            Json(CreatePasteResponse { id: parsed.address }),
        )
            .into_response();
    }

    // All attempts exhausted.
    tracing::error!(
        "❌ upload failed after {} attempts — last error: {:?}",
        UPLOAD_MAX_ATTEMPTS,
        last_error_detail
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
    // can legitimately take 2-4 minutes on first-seen content. 300s gives us
    // a comfortable margin without hiding real stalls forever.
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    let state = AppState { antd_url, http };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/", get(health))
        .route("/paste", post(create_paste))
        .route("/paste/:id", get(get_paste))
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
