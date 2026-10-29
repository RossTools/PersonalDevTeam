//! How the GitHub forge reaches GitHub: one HTTP request at a time through a [`Transport`]. pdtd installs
//! [`ReqwestTransport`]; tests use [`testing::Recorded`] (recorded answers, never the network) or [`Offline`].

use std::sync::OnceLock;
use std::time::Duration;

/// How long one request may take before it counts as hung.
pub const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    /// Names lowercased.
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// The URL that answered, after any redirects, without its query (empty: the request's own).
    pub url: String,
}

impl Response {
    pub fn new(status: u16, body: impl Into<String>) -> Self {
        Self { status, headers: vec![], body: body.into(), url: String::new() }
    }
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_ascii_lowercase(), value.into()));
        self
    }
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// Sends one request. Err is a network failure (no answer at all); any answer, an error status too, is Ok.
pub trait Transport: Send + Sync {
    fn send(&self, req: &Request) -> Result<Response, String>;
}

/// Redirects followed before a request fails (a renamed repository, then a transfer, then a job log's storage link).
pub const MAX_REDIRECTS: usize = 3;

/// The network, through reqwest on pdtd's runtime. Blocking: call from `spawn_blocking` (the forge's contract), never
/// on an async worker. Redirects are followed, at most [`MAX_REDIRECTS`]: a renamed or moved repository's (GitHub
/// answers 301 for GET and 307 for the rest, so a POST keeps its body), and job logs' storage links. reqwest keeps the
/// token on the API host only: it drops it at the first hop to another host, scheme or port (PDT-195).
pub struct ReqwestTransport {
    handle: tokio::runtime::Handle,
    client: OnceLock<reqwest::Client>,
}

impl ReqwestTransport {
    /// On the current runtime (call it from async code).
    pub fn current() -> Self {
        Self::on(tokio::runtime::Handle::current())
    }
    pub fn on(handle: tokio::runtime::Handle) -> Self {
        Self { handle, client: OnceLock::new() }
    }
    fn client(&self) -> Result<&reqwest::Client, String> {
        if let Some(c) = self.client.get() {
            return Ok(c);
        }
        let c = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .redirect(redirect_policy())
            .user_agent(pdt_core::brand::SLUG)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(self.client.get_or_init(|| c))
    }
}

impl Transport for ReqwestTransport {
    fn send(&self, req: &Request) -> Result<Response, String> {
        let client = self.client()?.clone();
        let method = reqwest::Method::from_bytes(req.method.as_bytes()).map_err(|e| e.to_string())?;
        let mut b = client.request(method, &req.url);
        for (k, v) in &req.headers {
            b = b.header(*k, v);
        }
        if let Some(body) = &req.body {
            b = b.body(body.clone());
        }
        self.handle.block_on(async move {
            let res = b.send().await.map_err(|e| e.without_url().to_string())?;
            let status = res.status().as_u16();
            let url = bare(res.url());
            let headers = res
                .headers()
                .iter()
                .filter_map(|(k, v)| Some((k.as_str().to_ascii_lowercase(), v.to_str().ok()?.to_string())))
                .collect();
            let body = res.text().await.map_err(|e| e.without_url().to_string())?;
            Ok(Response { status, headers, body, url })
        })
    }
}

/// A URL for logs: no query (a signed link's signature) and no fragment.
fn bare(u: &reqwest::Url) -> String {
    let mut u = u.clone();
    u.set_query(None);
    u.set_fragment(None);
    u.to_string()
}

fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        // `previous` holds every URL asked so far, the first request's too.
        if attempt.previous().len() > MAX_REDIRECTS {
            return attempt.error(format!("GitHub redirected more than {MAX_REDIRECTS} times"));
        }
        if let Some(from) = attempt.previous().last()
            && from.host_str() == attempt.url().host_str()
            && matches!(attempt.status().as_u16(), 301 | 308)
        {
            tracing::info!(
                from = bare(from),
                to = bare(attempt.url()),
                "GitHub moved an API request (a renamed repository?)"
            );
        }
        attempt.follow()
    })
}

/// No network: every request fails. `AppState::new`'s forge uses it, so tests never reach GitHub.
pub struct Offline;

impl Transport for Offline {
    fn send(&self, _: &Request) -> Result<Response, String> {
        Err("no network (a test's forge)".into())
    }
}

/// `application/x-www-form-urlencoded` (and query values): every byte but the unreserved ones escaped.
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn form(pairs: &[(&str, &str)]) -> String {
    pairs.iter().map(|(k, v)| format!("{}={}", encode(k), encode(v))).collect::<Vec<_>>().join("&")
}

pub mod testing {
    use super::*;
    use std::sync::Mutex;

    /// Recorded answers: each request takes the first unused one whose method and URL match (the URL by its start,
    /// so a test names `…/actions/runs?branch=b` without every parameter), else a 404. Every request is kept.
    #[derive(Default)]
    pub struct Recorded {
        answers: Mutex<Vec<(&'static str, String, Response, bool)>>,
        pub requests: Mutex<Vec<Request>>,
    }

    impl Recorded {
        pub fn new() -> Self {
            Self::default()
        }
        /// Answers the next `method` request to a URL starting with `url` with `res`.
        pub fn on(&self, method: &'static str, url: &str, res: Response) -> &Self {
            self.answers.lock().unwrap().push((method, url.into(), res, false));
            self
        }
        /// The JSON answer of a 200.
        pub fn ok(&self, method: &'static str, url: &str, body: &str) -> &Self {
            self.on(method, url, Response::new(200, body))
        }
        pub fn sent(&self) -> Vec<Request> {
            self.requests.lock().unwrap().clone()
        }
        /// The URLs requested, in order.
        pub fn urls(&self) -> Vec<String> {
            self.sent().into_iter().map(|r| format!("{} {}", r.method, r.url)).collect()
        }
    }

    impl Transport for Recorded {
        fn send(&self, req: &Request) -> Result<Response, String> {
            self.requests.lock().unwrap().push(req.clone());
            let mut answers = self.answers.lock().unwrap();
            let hit = answers.iter_mut().find(|(m, u, _, used)| !*used && *m == req.method && req.url.starts_with(u));
            match hit {
                Some((_, _, res, used)) => {
                    *used = true;
                    let mut res = res.clone();
                    if res.url.is_empty() {
                        res.url = req.url.split('?').next().unwrap_or_default().to_string();
                    }
                    Ok(res)
                }
                None => Ok(Response::new(404, r#"{"message":"Not Found"}"#)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms_escape_everything_but_unreserved_bytes() {
        assert_eq!(encode("pdt/PDT-1 x&y"), "pdt%2FPDT-1%20x%26y");
        assert_eq!(form(&[("scope", "repo read:org"), ("a", "b")]), "scope=repo%20read%3Aorg&a=b");
    }

    #[test]
    fn recorded_answers_are_used_once_in_order() {
        let r = testing::Recorded::new();
        r.ok("GET", "https://x/a", "1").ok("GET", "https://x/a", "2");
        let get = |u: &str| Request { method: "GET", url: u.into(), headers: vec![], body: None };
        assert_eq!(r.send(&get("https://x/a?p=1")).unwrap().body, "1");
        assert_eq!(r.send(&get("https://x/a")).unwrap().body, "2");
        assert_eq!(r.send(&get("https://x/a")).unwrap().status, 404);
        assert_eq!(r.urls().len(), 3);
        assert!(Offline.send(&get("https://x")).is_err());
    }

    /// A local GitHub where `old/r` was renamed (to `/repositories/5`), `away` leads to another host, and `loop`
    /// redirects to itself. Answers say whether the token arrived.
    mod moved {
        use super::*;
        use axum::extract::State;
        use axum::http::{HeaderMap, StatusCode, Uri, header};
        use axum::response::IntoResponse;
        use pdt_forge::Forge;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Clone, Default)]
        struct Seen {
            other: Arc<std::sync::Mutex<Option<String>>>,
            loops: Arc<AtomicUsize>,
        }

        fn auth(h: &HeaderMap) -> String {
            h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).unwrap_or("none").to_string()
        }

        async fn serve(app: axum::Router) -> String {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
            tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
            base
        }

        async fn github() -> (String, Seen) {
            let seen = Seen::default();
            let other = serve(
                axum::Router::new()
                    .fallback(|State(s): State<Seen>, h: HeaderMap| async move {
                        *s.other.lock().unwrap() = Some(auth(&h));
                        "[]"
                    })
                    .with_state(seen.clone()),
            )
            .await;
            let app = axum::Router::new()
                .route(
                    "/repos/old/r/{*rest}",
                    axum::routing::any(|m: axum::http::Method, uri: Uri| async move {
                        let to = uri.path_and_query().unwrap().as_str().replace("/repos/old/r/", "/repositories/5/");
                        let code = if m == axum::http::Method::GET {
                            StatusCode::MOVED_PERMANENTLY
                        } else {
                            StatusCode::TEMPORARY_REDIRECT
                        };
                        (code, [(header::LOCATION, to)]).into_response()
                    }),
                )
                .route(
                    "/repositories/5/issues",
                    axum::routing::get(
                        |h: HeaderMap| async move { axum::Json(serde_json::json!([{"auth": auth(&h)}])) },
                    ),
                )
                .route(
                    "/repositories/5/issues/1/comments",
                    axum::routing::post(|h: HeaderMap, body: String| async move {
                        let url = format!("{} {body}", auth(&h));
                        (StatusCode::CREATED, axum::Json(serde_json::json!({"html_url": url})))
                    }),
                )
                .route(
                    "/repos/away/r/issues",
                    axum::routing::get(move || {
                        let to = format!("{other}/issues?sig=x");
                        async move { (StatusCode::MOVED_PERMANENTLY, [(header::LOCATION, to)]).into_response() }
                    }),
                )
                .route(
                    "/repos/loop/r/issues",
                    axum::routing::get(|State(s): State<Seen>| async move {
                        s.loops.fetch_add(1, Ordering::SeqCst);
                        (StatusCode::FOUND, [(header::LOCATION, "/repos/loop/r/issues")]).into_response()
                    }),
                )
                .with_state(seen.clone());
            (serve(app).await, seen)
        }

        fn forge(api: &str) -> crate::GithubApi {
            let g = crate::GithubApi::new(Arc::new(ReqwestTransport::current()));
            g.set_account(Some(crate::Account {
                token: "gho_t".into(),
                api_url: api.into(),
                login: "me".into(),
                signed_in_at: None,
                ..Default::default()
            }));
            g
        }

        fn send(base: &str, path: &str) -> Response {
            let url = format!("{base}/{path}");
            let headers = vec![("Authorization", "Bearer gho_t".to_string())];
            ReqwestTransport::current().send(&Request { method: "GET", url, headers, body: None }).unwrap()
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn a_renamed_repository_is_followed_with_the_token() {
            let (base, _) = github().await;
            let g = forge(&base);
            let url = tokio::task::spawn_blocking(move || g.comment_issue("old/r", 1, "hi").unwrap()).await.unwrap();
            assert!(url.starts_with("Bearer gho_t "), "the POST kept its token: {url}");
            assert!(url.contains("hi"), "and its body: {url}");
            let b = base.clone();
            let res = tokio::task::spawn_blocking(move || send(&b, "repos/old/r/issues")).await.unwrap();
            assert_eq!((res.status, res.body.as_str()), (200, r#"[{"auth":"Bearer gho_t"}]"#));
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn a_redirect_to_another_host_drops_the_token() {
            let (base, seen) = github().await;
            let res = tokio::task::spawn_blocking(move || send(&base, "repos/away/r/issues")).await.unwrap();
            assert_eq!(res.status, 200);
            assert_eq!(seen.other.lock().unwrap().as_deref(), Some("none"));
        }

        #[tokio::test(flavor = "multi_thread")]
        async fn a_redirect_loop_stops_after_three_hops() {
            let (base, seen) = github().await;
            let url = format!("{base}/repos/loop/r/issues");
            let e = tokio::task::spawn_blocking(move || {
                ReqwestTransport::current().send(&Request { method: "GET", url, headers: vec![], body: None })
            })
            .await
            .unwrap()
            .unwrap_err();
            assert!(!e.is_empty());
            assert_eq!(seen.loops.load(Ordering::SeqCst), 1 + MAX_REDIRECTS);
        }
    }
}
