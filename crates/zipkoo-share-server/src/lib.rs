//! Zipkoo share server. Stores ciphertext blobs with an expiry and a download limit.
//! Keys travel only in the link's `#` fragment, so this server cannot read what it stores.
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct Config {
    pub dir: PathBuf,
    pub max_bytes: u64,
    pub max_hours: u32,
}

#[derive(Clone)]
pub struct AppState {
    cfg: Arc<Config>,
    /// Serializes metadata updates (download counters).
    lock: Arc<Mutex<()>>,
}

#[derive(Serialize, Deserialize)]
struct Record {
    size: u64,
    created: u64,
    expires_at: u64,
    max_downloads: Option<u32>,
    downloads: u32,
    delete_token_sha256: String,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn new_id(len: usize) -> String {
    use rand::Rng;
    const A: &[u8] = b"abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut r = rand::thread_rng();
    (0..len).map(|_| A[r.gen_range(0..A.len())] as char).collect()
}

fn sha(s: &str) -> String {
    Sha256::digest(s.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn valid_id(id: &str) -> bool {
    (8..=32).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric())
}

impl AppState {
    fn blob(&self, id: &str) -> PathBuf {
        self.cfg.dir.join(format!("{id}.bin"))
    }
    fn meta(&self, id: &str) -> PathBuf {
        self.cfg.dir.join(format!("{id}.json"))
    }
    async fn load(&self, id: &str) -> Option<Record> {
        if !valid_id(id) {
            return None;
        }
        serde_json::from_slice(&tokio::fs::read(self.meta(id)).await.ok()?).ok()
    }
    async fn save(&self, id: &str, r: &Record) -> std::io::Result<()> {
        tokio::fs::write(self.meta(id), serde_json::to_vec(r).unwrap()).await
    }
    async fn remove(&self, id: &str) {
        let _ = tokio::fs::remove_file(self.blob(id)).await;
        let _ = tokio::fs::remove_file(self.meta(id)).await;
    }
    fn alive(r: &Record) -> bool {
        r.expires_at > now() && r.max_downloads.map(|m| r.downloads < m).unwrap_or(true)
    }
}

fn err(code: StatusCode, msg: &str) -> Response {
    (code, Json(serde_json::json!({ "error": msg }))).into_response()
}

async fn create(State(st): State<AppState>, headers: HeaderMap, body: Body) -> Response {
    let hdr = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    let hours = hdr("x-expires-hours").unwrap_or(24).clamp(1, st.cfg.max_hours as u64);
    let max_downloads = hdr("x-max-downloads").map(|m| m.clamp(1, 10_000) as u32);
    if let Some(len) = hdr("content-length") {
        if len > st.cfg.max_bytes {
            return err(StatusCode::PAYLOAD_TOO_LARGE, "too large");
        }
    }
    let id = new_id(16);
    let token = new_id(32);
    let tmp = st.cfg.dir.join(format!("{id}.part"));
    let mut file = match tokio::fs::File::create(&tmp).await {
        Ok(f) => f,
        Err(_) => return err(StatusCode::INTERNAL_SERVER_ERROR, "storage unavailable"),
    };
    let mut size = 0u64;
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            let _ = tokio::fs::remove_file(&tmp).await;
            return err(StatusCode::BAD_REQUEST, "upload interrupted");
        };
        size += chunk.len() as u64;
        if size > st.cfg.max_bytes {
            drop(file);
            let _ = tokio::fs::remove_file(&tmp).await;
            return err(StatusCode::PAYLOAD_TOO_LARGE, "too large");
        }
        if file.write_all(&chunk).await.is_err() {
            let _ = tokio::fs::remove_file(&tmp).await;
            return err(StatusCode::INSUFFICIENT_STORAGE, "storage full");
        }
    }
    let _ = file.flush().await;
    drop(file);
    let rec = Record { size, created: now(), expires_at: now() + hours * 3600, max_downloads, downloads: 0, delete_token_sha256: sha(&token) };
    if tokio::fs::rename(&tmp, st.blob(&id)).await.is_err() || st.save(&id, &rec).await.is_err() {
        st.remove(&id).await;
        return err(StatusCode::INTERNAL_SERVER_ERROR, "storage unavailable");
    }
    (StatusCode::CREATED, Json(serde_json::json!({ "id": id, "deleteToken": token, "expiresAt": rec.expires_at, "size": size }))).into_response()
}

async fn meta(State(st): State<AppState>, Path(id): Path<String>) -> Response {
    match st.load(&id).await {
        Some(r) if AppState::alive(&r) => Json(serde_json::json!({
            "size": r.size, "expiresAt": r.expires_at,
            "downloadsLeft": r.max_downloads.map(|m| m - r.downloads),
        }))
        .into_response(),
        _ => err(StatusCode::NOT_FOUND, "gone"),
    }
}

async fn blob(State(st): State<AppState>, Path(id): Path<String>) -> Response {
    let rec = {
        let _g = st.lock.lock().await;
        let Some(mut r) = st.load(&id).await else { return err(StatusCode::NOT_FOUND, "gone") };
        if !AppState::alive(&r) {
            return err(StatusCode::GONE, "gone");
        }
        r.downloads += 1;
        if st.save(&id, &r).await.is_err() {
            return err(StatusCode::INTERNAL_SERVER_ERROR, "storage unavailable");
        }
        r
    };
    let Ok(file) = tokio::fs::File::open(st.blob(&id)).await else { return err(StatusCode::NOT_FOUND, "gone") };
    let stream = tokio_util::io::ReaderStream::with_capacity(file, 1 << 16);
    Response::builder()
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, rec.size)
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from_stream(stream))
        .unwrap()
}

/// The first bytes of a share (header + encrypted name), so clients can check the password
/// before downloading. Does not count as a download.
async fn head(State(st): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(r) = st.load(&id).await else { return err(StatusCode::NOT_FOUND, "gone") };
    if !AppState::alive(&r) {
        return err(StatusCode::GONE, "gone");
    }
    let Ok(f) = tokio::fs::File::open(st.blob(&id)).await else { return err(StatusCode::NOT_FOUND, "gone") };
    let mut buf = Vec::new();
    use tokio::io::AsyncReadExt;
    if f.take(45 + 65_536 + 16).read_to_end(&mut buf).await.is_err() {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "storage unavailable");
    }
    Response::builder().header(header::CONTENT_TYPE, "application/octet-stream").header(header::CACHE_CONTROL, "no-store").body(Body::from(buf)).unwrap()
}

async fn delete(State(st): State<AppState>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    let token = headers.get("x-delete-token").and_then(|v| v.to_str().ok()).unwrap_or("");
    match st.load(&id).await {
        Some(r) if r.delete_token_sha256 == sha(token) => {
            st.remove(&id).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Some(_) => err(StatusCode::FORBIDDEN, "wrong token"),
        None => err(StatusCode::NOT_FOUND, "gone"),
    }
}

const PAGE: &str = include_str!("../static/download.html");

async fn page() -> impl IntoResponse {
    (
        [
            (header::CONTENT_SECURITY_POLICY, "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'; img-src data:"),
            (header::REFERRER_POLICY, "no-referrer"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        Html(PAGE),
    )
}

async fn home() -> &'static str {
    "Zipkoo share server. Files here are end-to-end encrypted."
}

/// Delete expired and used-up shares.
pub async fn sweep(st: &AppState) -> usize {
    let mut n = 0;
    let Ok(mut rd) = tokio::fs::read_dir(&st.cfg.dir).await else { return 0 };
    while let Ok(Some(e)) = rd.next_entry().await {
        let name = e.file_name().to_string_lossy().into_owned();
        if let Some(id) = name.strip_suffix(".json") {
            if let Some(r) = st.load(id).await {
                // Used-up shares get an hour of grace so an in-flight download can finish.
                let used_up = r.max_downloads.map(|m| r.downloads >= m).unwrap_or(false);
                if r.expires_at <= now() || (used_up && now() > r.created + 3600) {
                    st.remove(id).await;
                    n += 1;
                }
            }
        }
    }
    n
}

pub fn app(cfg: Config) -> (Router, AppState) {
    let st = AppState { cfg: Arc::new(cfg), lock: Arc::new(Mutex::new(())) };
    let r = Router::new()
        .route("/", get(home))
        .route("/api/shares", post(create))
        .route("/api/shares/{id}", get(meta).delete(delete))
        .route("/api/shares/{id}/blob", get(blob))
        .route("/api/shares/{id}/head", get(head))
        .route("/s/{id}", get(page))
        .layer(axum::extract::DefaultBodyLimit::disable())
        .with_state(st.clone());
    (r, st)
}
