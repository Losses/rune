//! An in-process server that answers as the Apple Music API does with scripted replies and
//! fixtures, and records what it was asked, for the tests of this crate and of those that use
//! it.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use tokio::net;
use tokio::time;

/// A scripted answer.
#[derive(Clone)]
pub struct Reply {
    pub status: u16,
    pub body: String,
    pub headers: Vec<(String, String)>,
}

impl Reply {
    pub fn fixture(name: &str) -> Self {
        Self::fixture_status(200, name)
    }

    pub fn fixture_status(status: u16, name: &str) -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let body = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
        Self {
            status,
            body,
            headers: Vec::new(),
        }
    }

    pub fn status(status: u16) -> Self {
        Self {
            status,
            body: String::new(),
            headers: Vec::new(),
        }
    }

    pub fn json(body: serde_json::Value) -> Self {
        Self {
            status: 200,
            body: body.to_string(),
            headers: Vec::new(),
        }
    }

    pub fn json_status(status: u16, body: serde_json::Value) -> Self {
        Self {
            status,
            body: body.to_string(),
            headers: Vec::new(),
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

/// What the server saw.
#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    /// Path and query as sent.
    pub target: String,
    pub path: String,
    pub authorization: Option<String>,
    pub user_token: Option<String>,
    pub content_length: Option<String>,
    pub body: String,
}

#[derive(Default)]
struct State_ {
    /// Scripted replies per "METHOD /path"; the last one repeats.
    routes: HashMap<String, VecDeque<Reply>>,
    /// Scripted replies per "METHOD /path?query", tried before `routes`.
    targets: HashMap<String, Reply>,
    /// How long to hold the answer to "METHOD /path".
    delays: HashMap<String, Duration>,
    seen: Vec<Seen>,
}

#[derive(Clone)]
pub struct FixtureServer {
    pub base_url: String,
    state: Arc<Mutex<State_>>,
}

async fn handle(
    State(state): State<Arc<Mutex<State_>>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
    };
    let (reply, delay) = {
        let mut state = state.lock().unwrap();
        state.seen.push(Seen {
            method: method.to_string(),
            target: uri
                .path_and_query()
                .map(|p| p.as_str().to_owned())
                .unwrap_or_default(),
            path: uri.path().to_owned(),
            authorization: header("authorization"),
            user_token: header("music-user-token"),
            content_length: header("content-length"),
            body: String::from_utf8_lossy(&body).into_owned(),
        });
        let key = format!("{method} {}", uri.path());
        let target = format!(
            "{method} {}",
            uri.path_and_query().map(|p| p.as_str()).unwrap_or_default()
        );
        let exact = state.targets.get(&target).cloned();
        let reply = exact.or_else(|| match state.routes.get_mut(&key) {
            Some(queue) if queue.len() > 1 => queue.pop_front(),
            Some(queue) => queue.front().cloned(),
            None => None,
        });
        let delay = state.delays.get(&key).copied();
        (reply, delay)
    };
    let Some(reply) = reply else {
        return (
            StatusCode::IM_A_TEAPOT,
            format!("no route for {method} {}", uri.path()),
        )
            .into_response();
    };
    if let Some(delay) = delay {
        time::sleep(delay).await;
    }
    let mut response = (
        StatusCode::from_u16(reply.status).unwrap(),
        reply.body.clone(),
    )
        .into_response();
    for (name, value) in &reply.headers {
        response.headers_mut().insert(
            http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
    }
    response
}

impl FixtureServer {
    pub async fn start() -> Self {
        let state = Arc::new(Mutex::new(State_::default()));
        let app = Router::new().fallback(handle).with_state(state.clone());
        let listener = net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            base_url: format!("http://{addr}"),
            state,
        }
    }

    /// Queues a reply for `METHOD path`. The last queued reply repeats.
    pub fn on(&self, method: &str, path: &str, reply: Reply) -> &Self {
        self.state
            .lock()
            .unwrap()
            .routes
            .entry(format!("{method} {path}"))
            .or_default()
            .push_back(reply);
        self
    }

    /// Answers `METHOD path?query` exactly, before any reply queued for the path.
    pub fn on_target(&self, method: &str, target: &str, reply: Reply) -> &Self {
        self.state
            .lock()
            .unwrap()
            .targets
            .insert(format!("{method} {target}"), reply);
        self
    }

    /// Holds every answer to `METHOD path` for `delay`, after recording the request.
    pub fn delay(&self, method: &str, path: &str, delay: Duration) -> &Self {
        self.state
            .lock()
            .unwrap()
            .delays
            .insert(format!("{method} {path}"), delay);
        self
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.state.lock().unwrap().seen.clone()
    }

    pub fn seen_for(&self, method: &str, path: &str) -> Vec<Seen> {
        self.seen()
            .into_iter()
            .filter(|s| s.method == method && s.path == path)
            .collect()
    }
}
