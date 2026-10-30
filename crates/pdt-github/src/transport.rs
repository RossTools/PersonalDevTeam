//! How the GitHub forge reaches GitHub: PDT's forge transport ([`pdt_forge::transport`]), shared with the other
//! forges.

pub use pdt_forge::transport::*;

#[cfg(test)]
mod tests {
    use super::*;

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
