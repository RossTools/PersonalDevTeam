//! A fake GitHub issues endpoint for bug reports (PDT-153) on `o/r`: [`TOKEN`] may create issues, [`READ_ONLY`] gets a
//! 403 (a token without Issues: write), anything else a 401. Every request is counted, so a test can assert none came.

use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

pub const TOKEN: &str = "github_pat_test_issue_token_value";
pub const READ_ONLY: &str = "github_pat_read_only_token";

#[derive(Clone, Default)]
pub struct FakeIssues {
    /// Every request's JSON body, issue created or not.
    pub requests: Arc<Mutex<Vec<Value>>>,
    /// The issues created.
    pub created: Arc<Mutex<Vec<Value>>>,
}

impl FakeIssues {
    /// Serves it on a free local port; returns its base URL.
    pub async fn serve() -> (String, FakeIssues) {
        let fake = FakeIssues::default();
        let f = fake.clone();
        let app = axum::Router::new().route(
            "/repos/o/r/issues",
            axum::routing::post(move |h: HeaderMap, axum::Json(body): axum::Json<Value>| {
                let f = f.clone();
                async move {
                    f.requests.lock().unwrap().push(body.clone());
                    let auth = h.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
                    if auth == format!("Bearer {READ_ONLY}") {
                        let msg = json!({"message": "Resource not accessible by personal access token"});
                        return (StatusCode::FORBIDDEN, axum::Json(msg)).into_response();
                    }
                    if auth != format!("Bearer {TOKEN}") {
                        return (StatusCode::UNAUTHORIZED, axum::Json(json!({"message": "Bad credentials"})))
                            .into_response();
                    }
                    let mut created = f.created.lock().unwrap();
                    created.push(body);
                    let number = created.len() + 100;
                    let url = format!("https://github.com/o/r/issues/{number}");
                    (StatusCode::CREATED, axum::Json(json!({"number": number, "html_url": url}))).into_response()
                }
            }),
        );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (base, fake)
    }

    pub fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}
