use axum::{
    extract::Path,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

// Temporary in-memory store. Wave 3b replaces this with Autonomi.
type Store = Arc<Mutex<HashMap<String, String>>>;

#[derive(Deserialize)]
struct CreatePasteRequest {
    content: String,
}

#[derive(Serialize)]
struct CreatePasteResponse {
    id: String,
}

#[derive(Serialize)]
struct GetPasteResponse {
    content: String,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

async fn health() -> &'static str {
    "Paste4Ever API — OK"
}

async fn create_paste(
    axum::extract::State(store): axum::extract::State<Store>,
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

    // Generate a short random ID (Wave 3b replaces this with Autonomi chunk address)
    let id: String = (0..8)
        .map(|_| {
            let chars = b"abcdefghijklmnopqrstuvwxyz0123456789";
            let idx = rand_index(chars.len());
            chars[idx] as char
        })
        .collect();

    store.lock().unwrap().insert(id.clone(), req.content);

    (StatusCode::OK, Json(CreatePasteResponse { id })).into_response()
}

async fn get_paste(
    axum::extract::State(store): axum::extract::State<Store>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match store.lock().unwrap().get(&id) {
        Some(content) => (
            StatusCode::OK,
            Json(GetPasteResponse {
                content: content.clone(),
            }),
        )
            .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": "Not found" })),
        )
            .into_response(),
    }
}

// Cheap pseudo-random since we haven't added the `rand` crate yet
fn rand_index(max: usize) -> usize {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as usize;
    nanos % max
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter("paste4ever_api=debug,tower_http=debug")
        .init();

    let store: Store = Arc::new(Mutex::new(HashMap::new()));

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
        .with_state(store);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080")
        .await
        .unwrap();

    println!("🚀 Paste4Ever API listening on http://localhost:8080");
    axum::serve(listener, app).await.unwrap();
}