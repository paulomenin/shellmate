//! Test helpers: a fake provider and a tiny mock HTTP server. Dev-dependency only.

mod services;
mod store;

pub use services::{output, FakeServices};
pub use store::{assert_store_contract, MemoryStore};

use async_trait::async_trait;
use shellmate_api::{ChatRequest, ChatResponse, Provider, ProviderError, ProviderInfo};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A provider that returns canned replies and records the requests it saw.
pub struct FakeProvider {
    replies: Mutex<Vec<Result<String, ProviderError>>>,
    pub seen: Mutex<Vec<ChatRequest>>,
}

impl FakeProvider {
    pub fn replying(text: &str) -> Arc<Self> {
        Self::with(vec![Ok(text.to_string())])
    }
    pub fn with(replies: Vec<Result<String, ProviderError>>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into_iter().rev().collect()),
            seen: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl Provider for FakeProvider {
    fn info(&self) -> ProviderInfo {
        ProviderInfo {
            plugin: "fake".into(),
            model: "fake-1".into(),
            endpoint: None,
        }
    }
    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.seen.lock().unwrap().push(request.clone());
        let next = self
            .replies
            .lock()
            .unwrap()
            .pop()
            .expect("FakeProvider ran out of replies");
        next.map(|text| ChatResponse {
            text,
            usage: None,
            finish_reason: Some("stop".into()),
        })
    }
}

/// Serves one fixed HTTP response per connection on a random loopback port and keeps the raw requests.
pub struct MockHttpServer {
    addr: std::net::SocketAddr,
    requests: Arc<tokio::sync::Mutex<Vec<String>>>,
}

impl MockHttpServer {
    /// `status` and a JSON `body` returned for every request.
    pub async fn start(status: u16, body: &str) -> Self {
        let body = body.to_string();
        Self::start_dynamic(status, move |_| body.clone()).await
    }

    /// Like [`start`](Self::start) but the body is computed from the raw request text.
    pub async fn start_dynamic(
        status: u16,
        respond: impl Fn(&str) -> String + Send + Sync + 'static,
    ) -> Self {
        let respond = Arc::new(respond);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let reqs = requests.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let (reqs, respond) = (reqs.clone(), respond.clone());
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 4096];
                    loop {
                        let n = sock.read(&mut chunk).await.unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        if let Some(end) = find(&buf, b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&buf[..end]).to_lowercase();
                            let len = head
                                .lines()
                                .find_map(|l| {
                                    l.strip_prefix("content-length:")
                                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                                })
                                .unwrap_or(0);
                            if buf.len() >= end + 4 + len {
                                break;
                            }
                        }
                    }
                    let raw = String::from_utf8_lossy(&buf).into_owned();
                    let mut body = respond(&raw);
                    let mut content_type = "application/json";
                    // A streaming request gets the canned completion as an SSE stream.
                    if status == 200 && raw.contains("\"stream\":true") {
                        body = completion_as_sse(&body);
                        content_type = "text/event-stream";
                    }
                    reqs.lock().await.push(raw);
                    let resp = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        Self { addr, requests }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub async fn last_request(&self) -> String {
        self.requests
            .lock()
            .await
            .last()
            .cloned()
            .unwrap_or_default()
    }

    /// Every request received so far, in order. A connection that sent nothing (a reachability probe) is an
    /// empty string.
    pub async fn requests(&self) -> Vec<String> {
        self.requests.lock().await.clone()
    }

    pub async fn request_count(&self) -> usize {
        self.requests.lock().await.len()
    }
}

/// Re-encode a `chat.completion` JSON body as SSE deltas (a few characters per event) plus `[DONE]`.
fn completion_as_sse(body: &str) -> String {
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let text = v["choices"][0]["message"]["content"].as_str().unwrap_or("");
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    for piece in chars.chunks(7) {
        let delta: String = piece.iter().collect();
        out.push_str(&format!(
            "data: {}\n\n",
            serde_json::json!({"choices":[{"delta":{"content": delta}}]})
        ));
    }
    out.push_str("data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n");
    if let Some(u) = v.get("usage").filter(|u| !u.is_null()) {
        out.push_str(&format!(
            "data: {}\n\n",
            serde_json::json!({"choices":[],"usage":u})
        ));
    }
    out.push_str("data: [DONE]\n\n");
    out
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// One piece of a streamed response: wait `delay_ms`, then send `bytes` (as one HTTP chunk).
#[derive(Clone)]
pub struct SseChunk {
    pub delay_ms: u64,
    pub bytes: Vec<u8>,
}

impl SseChunk {
    pub fn new(delay_ms: u64, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            delay_ms,
            bytes: bytes.into(),
        }
    }

    /// A complete OpenAI-style SSE event carrying a text delta.
    pub fn delta(delay_ms: u64, text: &str) -> Self {
        let body = serde_json::json!({"choices":[{"delta":{"content": text}}]});
        Self::new(delay_ms, format!("data: {body}\n\n"))
    }

    pub fn done() -> Self {
        Self::new(0, "data: [DONE]\n\n")
    }
}

/// Serves a chunked `text/event-stream` response and notices when the client goes away.
pub struct MockSseServer {
    addr: std::net::SocketAddr,
    disconnected: Arc<std::sync::atomic::AtomicBool>,
    completed: Arc<std::sync::atomic::AtomicBool>,
    requests: Arc<tokio::sync::Mutex<Vec<String>>>,
}

impl MockSseServer {
    pub async fn start(chunks: Vec<SseChunk>) -> Self {
        use std::sync::atomic::{AtomicBool, Ordering};
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let disconnected = Arc::new(AtomicBool::new(false));
        let completed = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let (d, c, r) = (disconnected.clone(), completed.clone(), requests.clone());
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let (chunks, d, c, r) = (chunks.clone(), d.clone(), c.clone(), r.clone());
                tokio::spawn(async move {
                    r.lock().await.push(read_request(&mut sock).await);
                    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n";
                    if sock.write_all(head.as_bytes()).await.is_err() {
                        d.store(true, Ordering::SeqCst);
                        return;
                    }
                    for ch in chunks {
                        tokio::time::sleep(std::time::Duration::from_millis(ch.delay_ms)).await;
                        let mut frame = format!("{:x}\r\n", ch.bytes.len()).into_bytes();
                        frame.extend_from_slice(&ch.bytes);
                        frame.extend_from_slice(b"\r\n");
                        if sock.write_all(&frame).await.is_err() || sock.flush().await.is_err() {
                            d.store(true, Ordering::SeqCst);
                            return;
                        }
                    }
                    let _ = sock.write_all(b"0\r\n\r\n").await;
                    c.store(true, Ordering::SeqCst);
                });
            }
        });
        Self {
            addr,
            disconnected,
            completed,
            requests,
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
    /// True once a write failed because the client closed the connection.
    pub fn client_disconnected(&self) -> bool {
        self.disconnected.load(std::sync::atomic::Ordering::SeqCst)
    }
    pub fn completed(&self) -> bool {
        self.completed.load(std::sync::atomic::Ordering::SeqCst)
    }
    pub async fn last_request(&self) -> String {
        self.requests
            .lock()
            .await
            .last()
            .cloned()
            .unwrap_or_default()
    }
}

async fn read_request(sock: &mut tokio::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = sock.read(&mut chunk).await.unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = find(&buf, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).to_lowercase();
            let len = head
                .lines()
                .find_map(|l| {
                    l.strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            if buf.len() >= end + 4 + len {
                break;
            }
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}
