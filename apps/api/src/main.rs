//! Paste4Ever API
//!
//! Thin Rust gateway between the Next.js frontend and the local `antd` daemon
//! (the Autonomi network gateway). All P2P / wallet / payment logic lives in
//! `antd`; this service just proxies pastes to/from it and adds the Paste4Ever-
//! specific business logic (size limits, future rate limiting, etc).

use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use parking_lot::Mutex;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use axum::http::{HeaderName, Method};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

#[derive(Clone)]
struct AppState {
    /// Base URL of the local antd daemon, e.g. http://localhost:8082
    antd_url: String,
    http: reqwest::Client,
    /// Cloudflare Turnstile secret key. When set, every /paste request
    /// must include a `turnstile_token` that verifies successfully with
    /// Cloudflare. When empty (local dev), verification is skipped.
    turnstile_secret: Option<String>,
    /// Local SQLite index of pastes made through this API. The index is just
    /// a cache for the /recent wall — the actual paste bytes live on Autonomi
    /// and remain retrievable by address even if this DB is wiped.
    db: Arc<Mutex<Connection>>,
    /// Rolling count of consecutive *fully-failed* paste uploads (all retries
    /// exhausted). Reset to 0 on any successful upload. Used by /health so
    /// the external watchdog can restart antd when its DHT rots, and by the
    /// frontend to warn users before they waste a click.
    consecutive_failures: Arc<AtomicU32>,
    /// Sliding-window rate limiter keyed by client IP + a global bucket.
    /// Protects the hot wallet: each paste costs ~$0.50 in ANT, so an
    /// unthrottled public endpoint is an open drain.
    rate_limiter: Arc<Mutex<RateLimiter>>,
    /// In-memory store of in-flight + recently-completed paste jobs. The
    /// POST /paste endpoint returns immediately with a job_id; the real
    /// antd work runs in a spawned task and updates the job status here.
    /// The browser polls GET /paste/status/<job_id> to find out when it
    /// lands. Keyed by job_id; jobs age out after 10 minutes.
    jobs: Arc<Mutex<HashMap<String, PasteJob>>>,
    /// Atomic counter used as part of job_id generation. Combined with
    /// a nanosecond timestamp this gives us unique ids without pulling
    /// in a uuid dependency for a single call-site.
    job_counter: Arc<AtomicU64>,
}

// ── Async paste jobs ──────────────────────────────────────────────────────

/// Current state of a paste upload job. Serialized directly to the wire,
/// hence the `#[serde(tag)]` / `rename_all` — the browser reads it as
/// `{"status":"success","address":"..."}` etc.
#[derive(Clone, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
enum PasteJobStatus {
    /// Upload is still running on the server. Browser should keep polling.
    Pending,
    /// Upload landed; `address` is the Autonomi data address (hex).
    Success { address: String },
    /// Upload failed after all retries. `error` is a short user-facing
    /// message; `detail` carries the raw server error for debugging.
    Failed {
        error: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
}

#[derive(Clone)]
struct PasteJob {
    status: PasteJobStatus,
    /// Wall-clock create time. Used to age out completed/abandoned jobs
    /// so the in-memory map doesn't grow unboundedly.
    created_at: Instant,
}

/// Max age a job is kept in memory. After this, GET /paste/status returns
/// 404. The browser polls at most 10 min so this matches the client's
/// own deadline.
const JOB_TTL: Duration = Duration::from_secs(600);

// ── Rate limiting ─────────────────────────────────────────────────────────

/// Max pastes per IP per hour. Generous for legitimate human use
/// (5 pastes/hour is way more than any real user needs), tight enough to
/// make griefing uneconomical: ~5 × $0.50 = $2.50/hour per source IP.
const PER_IP_LIMIT: usize = 5;
/// Max pastes from the whole service per hour. Hard global cap so even a
/// distributed botnet can't drain the wallet faster than this. Pick a
/// number that matches what you're comfortable losing per hour in the
/// absolute worst case. $10/hour cap at current ANT price.
const GLOBAL_LIMIT: usize = 20;
/// Window length for both buckets. One hour.
const RL_WINDOW: std::time::Duration = std::time::Duration::from_secs(3600);

/// In-memory sliding-window rate limiter. For the MVP single-instance
/// deploy this is fine; swap for Redis if we ever horizontally scale.
struct RateLimiter {
    per_ip: HashMap<String, VecDeque<Instant>>,
    global: VecDeque<Instant>,
}

impl RateLimiter {
    fn new() -> Self {
        Self {
            per_ip: HashMap::new(),
            global: VecDeque::new(),
        }
    }

    /// Check whether this IP can do another paste right now. Returns
    /// Ok(()) on success (and records the attempt) or Err with a
    /// human-readable reason.
    fn try_acquire(&mut self, ip: &str) -> Result<(), RateLimitError> {
        let now = Instant::now();
        let cutoff = now - RL_WINDOW;

        // Prune the global window before checking.
        while self.global.front().map(|t| *t < cutoff).unwrap_or(false) {
            self.global.pop_front();
        }
        if self.global.len() >= GLOBAL_LIMIT {
            return Err(RateLimitError::Global {
                retry_after_secs: self
                    .global
                    .front()
                    .map(|t| RL_WINDOW.as_secs().saturating_sub(now.duration_since(*t).as_secs()))
                    .unwrap_or(RL_WINDOW.as_secs()),
            });
        }

        let bucket = self.per_ip.entry(ip.to_string()).or_default();
        while bucket.front().map(|t| *t < cutoff).unwrap_or(false) {
            bucket.pop_front();
        }
        if bucket.len() >= PER_IP_LIMIT {
            return Err(RateLimitError::PerIp {
                retry_after_secs: bucket
                    .front()
                    .map(|t| RL_WINDOW.as_secs().saturating_sub(now.duration_since(*t).as_secs()))
                    .unwrap_or(RL_WINDOW.as_secs()),
            });
        }

        bucket.push_back(now);
        self.global.push_back(now);
        Ok(())
    }

    /// Periodically called to stop per_ip from growing unboundedly for
    /// IPs we haven't seen in a while. Called from the size-limit check
    /// in create_paste so we don't need a separate background task.
    fn gc(&mut self) {
        let cutoff = Instant::now() - RL_WINDOW;
        self.per_ip.retain(|_, bucket| {
            while bucket.front().map(|t| *t < cutoff).unwrap_or(false) {
                bucket.pop_front();
            }
            !bucket.is_empty()
        });
    }
}

#[derive(Debug)]
enum RateLimitError {
    PerIp { retry_after_secs: u64 },
    Global { retry_after_secs: u64 },
}

// ── Turnstile ──────────────────────────────────────────────────────────────

/// Cloudflare's verify response. We only care about the `success` flag;
/// action/hostname/cdata are available for future tightening.
#[derive(Deserialize)]
struct TurnstileVerifyResponse {
    success: bool,
    #[serde(rename = "error-codes", default)]
    error_codes: Vec<String>,
}

/// Verify a Turnstile token against Cloudflare's siteverify endpoint.
/// `ip` is optional but recommended — Cloudflare uses it as a tamper check.
/// Returns Ok(()) on success or Err with a short reason suitable for logs.
async fn verify_turnstile(
    http: &reqwest::Client,
    secret: &str,
    token: &str,
    ip: Option<&str>,
) -> Result<(), String> {
    // Short timeout — this call is in the hot path of every paste and we'd
    // rather fail closed than hang if Cloudflare is flaky.
    let mut form = vec![("secret", secret), ("response", token)];
    if let Some(ip) = ip {
        form.push(("remoteip", ip));
    }
    let res = http
        .post("https://challenges.cloudflare.com/turnstile/v0/siteverify")
        .form(&form)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| format!("turnstile network error: {}", e))?;
    let parsed: TurnstileVerifyResponse = res
        .json()
        .await
        .map_err(|e| format!("turnstile parse error: {}", e))?;
    if parsed.success {
        Ok(())
    } else {
        Err(format!("turnstile failed: {:?}", parsed.error_codes))
    }
}

/// Extract the real client IP. Precedence matches what Cloudflare Tunnel
/// and standard reverse proxies put on the wire:
///   1. CF-Connecting-IP  (Cloudflare set this; trust it when we're behind CF)
///   2. X-Forwarded-For   (first hop, which is the original client)
///   3. The direct TCP peer address (localhost/dev case)
fn client_ip(headers: &HeaderMap, peer: SocketAddr) -> String {
    if let Some(v) = headers.get("cf-connecting-ip").and_then(|v| v.to_str().ok()) {
        return v.trim().to_string();
    }
    if let Some(v) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()) {
        if let Some(first) = v.split(',').next() {
            return first.trim().to_string();
        }
    }
    peer.ip().to_string()
}

// ── SQLite schema ─────────────────────────────────────────────────────────
/// Returns an 80-char preview with newlines collapsed, plus an ellipsis if
/// the original was longer. The preview lives in our DB so the wall can
/// render instantly without round-tripping back to Autonomi for every card.
fn make_preview(content: &str) -> String {
    // Pastes are already capped at 280 chars server-side, so the preview
    // can be the full content (with newlines collapsed). No ellipsis is
    // needed at the paste-length limit, but we keep a defensive truncate
    // at 300 just in case older pastes somehow slipped past the cap.
    const MAX: usize = 300;
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
    /// Cloudflare Turnstile response token. Required in production (when
    /// the server has TURNSTILE_SECRET_KEY set), ignored in local dev.
    #[serde(default)]
    turnstile_token: Option<String>,
}

// NOTE: the synchronous CreatePasteResponse type (returning the final
// address) was removed when POST /paste switched to 202 + job polling.
// The inline serde_json::json!() block in create_paste returns
// `{ job_id, status: "pending" }`; on success the client then reads
// `{ status: "success", address }` from GET /paste/status/:job_id.

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
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<CreatePasteRequest>,
) -> impl IntoResponse {
    if req.content.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "Content required" })),
        )
            .into_response();
    }
    // Hard cap at tweet length. Paste4Ever is positioned as "permanent
    // tweets, forever" — a one-thought-per-paste wall. Short posts keep
    // the wall readable and remove abuse vectors (no 99KB spam walls, no
    // dumped credential files, no pasted malware). Counted in chars
    // (code points) so CJK and emoji users aren't penalized vs ASCII.
    const MAX_CHARS: usize = 280;
    if req.content.chars().count() > MAX_CHARS {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({
                "error": format!("Too long. Pastes are capped at {} characters.", MAX_CHARS)
            })),
        )
            .into_response();
    }

    // Rate limit BEFORE any antd contact — we never want a throttled
    // request to cost us a chunk-payment. Also GC the per_ip map opportunistically
    // while we hold the lock, to keep memory bounded.
    let ip = client_ip(&headers, peer);
    {
        let mut rl = state.rate_limiter.lock();
        rl.gc();
        if let Err(e) = rl.try_acquire(&ip) {
            let (reason, retry) = match e {
                RateLimitError::PerIp { retry_after_secs } => (
                    "You've hit the per-IP paste limit. Try again later.",
                    retry_after_secs,
                ),
                RateLimitError::Global { retry_after_secs } => (
                    "Paste4Ever is busy right now. Please try again shortly.",
                    retry_after_secs,
                ),
            };
            tracing::warn!("rate-limited ip={} reason={}", ip, reason);
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [("retry-after", retry.to_string())],
                Json(serde_json::json!({
                    "error": reason,
                    "retry_after_secs": retry,
                })),
            )
                .into_response();
        }
    }

    // Turnstile verification. Only enforced when the server has a secret
    // configured (production). Local dev leaves TURNSTILE_SECRET_KEY unset
    // so curl + localhost work without a token.
    if let Some(secret) = state.turnstile_secret.as_deref() {
        let token = match req.turnstile_token.as_deref() {
            Some(t) if !t.is_empty() => t,
            _ => {
                tracing::warn!("turnstile: missing token ip={}", ip);
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "error": "Human verification required. Please complete the challenge and try again.",
                    })),
                )
                    .into_response();
            }
        };
        if let Err(e) = verify_turnstile(&state.http, secret, token, Some(&ip)).await {
            tracing::warn!("turnstile: verification failed ip={} err={}", ip, e);
            return (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": "Human verification failed. Please refresh and try again.",
                })),
            )
                .into_response();
        }
    }

    // All the fast, synchronous checks passed. Time to create a job
    // record and hand the slow antd work off to a background task so
    // the browser gets a response in milliseconds — avoiding the
    // Cloudflare edge's ~100s read-timeout on idle connections.
    let job_id = {
        let now_nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let n = state.job_counter.fetch_add(1, Ordering::Relaxed);
        format!("{:x}{:x}", now_nanos, n)
    };
    {
        let mut jobs = state.jobs.lock();
        // GC expired jobs while we hold the lock. Keeps the map bounded
        // without a separate tokio background task.
        let cutoff = Instant::now() - JOB_TTL;
        jobs.retain(|_, j| j.created_at > cutoff);
        jobs.insert(
            job_id.clone(),
            PasteJob {
                status: PasteJobStatus::Pending,
                created_at: Instant::now(),
            },
        );
    }

    let task_state = state.clone();
    let task_content = req.content;
    let task_job_id = job_id.clone();
    tokio::spawn(async move {
        run_paste_upload(task_state, task_content, task_job_id).await;
    });

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "job_id": job_id,
            "status": "pending",
        })),
    )
        .into_response()
}

/// Run the actual upload + SQLite-index work for a paste job. Called from
/// a spawned tokio task so the HTTP handler can return 202 in milliseconds.
/// Updates `state.jobs[job_id]` with the final status when done.
async fn run_paste_upload(state: AppState, content: String, job_id: String) {
    let body = AntdPutRequest {
        data: B64.encode(content.as_bytes()),
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
            tokio::time::sleep(Duration::from_secs(delay)).await;
        }
        tracing::info!(
            "→ POST {} ({} bytes raw) — attempt {}/{} [job {}]",
            url,
            content.len(),
            attempt,
            UPLOAD_MAX_ATTEMPTS,
            job_id
        );

        let res = match state.http.post(&url).json(&body).send().await {
            Ok(r) => r,
            Err(e) => {
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
            if status.is_server_error() {
                if detail.contains("partial upload") || detail.contains("chunk storage failed")
                {
                    let n = state.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
                    tracing::warn!("DHT-rot signal detected (consecutive_failures={})", n);
                }
                continue;
            }
            // 4xx — permanent client error. Mark the job failed and stop.
            finish_job(
                &state,
                &job_id,
                PasteJobStatus::Failed {
                    error: "Upload rejected by storage daemon.".to_string(),
                    detail: Some(detail),
                },
            );
            return;
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
            "✅ stored at {} (attempt {}/{}) [job {}]",
            parsed.address,
            attempt,
            UPLOAD_MAX_ATTEMPTS,
            job_id
        );
        state.consecutive_failures.store(0, Ordering::Relaxed);

        // Index the paste on our local wall. Best-effort — the bytes are
        // already safely on Autonomi regardless of DB success.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let preview = make_preview(&content);
        let size = content.len() as i64;
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

        finish_job(
            &state,
            &job_id,
            PasteJobStatus::Success {
                address: parsed.address,
            },
        );
        return;
    }

    // All attempts exhausted.
    let current = state.consecutive_failures.load(Ordering::Relaxed);
    let new_count = if current == 0 {
        state.consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1
    } else {
        current
    };
    tracing::error!(
        "❌ upload failed after {} attempts — last error: {:?} (consecutive_failures={}) [job {}]",
        UPLOAD_MAX_ATTEMPTS,
        last_error_detail,
        new_count,
        job_id
    );
    finish_job(
        &state,
        &job_id,
        PasteJobStatus::Failed {
            error: "Autonomi network is congested. Your paste could not be stored after multiple attempts — please try again in a few minutes.".to_string(),
            detail: last_error_detail,
        },
    );
}

/// Update a job's terminal status in the shared map. No-op if the job
/// has already been garbage-collected (browser gave up polling).
fn finish_job(state: &AppState, job_id: &str, status: PasteJobStatus) {
    let mut jobs = state.jobs.lock();
    if let Some(j) = jobs.get_mut(job_id) {
        j.status = status;
    }
}

/// GET /paste/status/:job_id — the browser polls this while an upload is
/// in flight. Returns the current `PasteJobStatus`, or 404 if the job id
/// is unknown (never existed, or aged out after JOB_TTL).
async fn get_paste_status(
    State(state): State<AppState>,
    Path(job_id): Path<String>,
) -> impl IntoResponse {
    let jobs = state.jobs.lock();
    match jobs.get(&job_id) {
        Some(j) => (StatusCode::OK, Json(j.status.clone())).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Job not found or expired" })),
        )
            .into_response(),
    }
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

    // TURNSTILE_SECRET_KEY: when present, every /paste requires a token that
    // verifies against Cloudflare. Empty / unset = dev mode (no challenge).
    let turnstile_secret = std::env::var("TURNSTILE_SECRET_KEY")
        .ok()
        .filter(|s| !s.trim().is_empty());
    match &turnstile_secret {
        Some(_) => tracing::info!("🛡  Turnstile enabled (human verification required)"),
        None => tracing::warn!("⚠  Turnstile disabled — /paste is open to any caller"),
    }

    let state = AppState {
        antd_url,
        http,
        turnstile_secret,
        db,
        consecutive_failures: Arc::new(AtomicU32::new(0)),
        rate_limiter: Arc::new(Mutex::new(RateLimiter::new())),
        jobs: Arc::new(Mutex::new(HashMap::new())),
        job_counter: Arc::new(AtomicU64::new(0)),
    };

    // CORS. The frontend lives on paste4ever.com / www.paste4ever.com and
    // calls this API directly from the browser (bypassing the Worker proxy
    // to avoid same-zone fetch issues). We need to spell methods/headers
    // explicitly: axum-cors's `Any` sometimes omits preflight responses
    // for POST-with-JSON requests, which would break paste creation with
    // a misleading 'Failed to fetch' in the browser.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::OPTIONS,
        ])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::ACCEPT,
            // X-Requested-With is sometimes sent by fetch libs as a hint;
            // easier to just accept it than debug a missing-header rejection.
            HeaderName::from_static("x-requested-with"),
        ])
        .expose_headers([
            // retry-after is set on 429 responses; browsers strip it by
            // default unless explicitly exposed.
            axum::http::header::RETRY_AFTER,
        ])
        .max_age(std::time::Duration::from_secs(3600));

    let app = Router::new()
        .route("/", get(health))
        .route("/health", get(health))
        .route("/paste", post(create_paste))
        .route("/paste/status/:job_id", get(get_paste_status))
        .route("/paste/:id", get(get_paste))
        .route("/recent", get(recent_pastes))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    println!("🚀 Paste4Ever API listening on http://localhost:{}", port);
    // `into_make_service_with_connect_info` gives handlers access to the TCP
    // peer address via ConnectInfo<SocketAddr>. The rate limiter uses it as
    // a fallback IP key when CF-Connecting-IP / X-Forwarded-For are absent
    // (i.e. in local dev and curl).
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}
