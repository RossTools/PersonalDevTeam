//! The owner check in front of a web run's tailnet port. `tailscale serve` maps the run's HTTPS port to one of these
//! proxies (a loopback port of pdtd's own), never to the run itself: the proxy refuses requests that pdtd's identity
//! guard would refuse and passes the rest to the run unchanged (headers included, upgrades such as WebSockets too),
//! so the run sees what it saw from `tailscale serve`. Dropping the [`RunProxy`] closes its listener, its
//! connections and its WebSocket tunnels.

use crate::error::{ApiError, codes};
use crate::routes::AppState;
use axum::body::Body;
use axum::response::IntoResponse;
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioIo};
use pdt_core::identity::check;
use std::convert::Infallible;
use tokio::net::TcpListener;
use tokio::sync::watch;

/// A running proxy for one web run. Dropping it stops the proxy.
#[derive(Debug)]
pub(crate) struct RunProxy {
    /// The loopback port it listens on (what `tailscale serve` points at).
    pub(crate) port: u16,
    /// Every task of this proxy ends when the sender is dropped.
    _closed: watch::Sender<()>,
}

/// Starts a proxy on a free loopback port that passes the owner's requests (and local ones) to `target`.
pub(crate) async fn start(st: &AppState, target: u16) -> std::io::Result<RunProxy> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let (tx, rx) = watch::channel(());
    let client: Client<HttpConnector, Body> = Client::builder(TokioExecutor::new()).build_http();
    let st = st.clone();
    tokio::spawn(async move {
        let mut closed = rx.clone();
        loop {
            let stream = tokio::select! {
                _ = closed.changed() => return,
                accepted = listener.accept() => match accepted {
                    Ok((stream, _)) => stream,
                    Err(e) => {
                        tracing::debug!(port, error = %e, "a run proxy could not accept a connection");
                        continue;
                    }
                },
            };
            let (st, client, rx) = (st.clone(), client.clone(), rx.clone());
            tokio::spawn(async move {
                let svc = hyper::service::service_fn({
                    let rx = rx.clone();
                    move |req| forward(st.clone(), client.clone(), target, rx.clone(), req)
                });
                let conn = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), svc)
                    .with_upgrades();
                let mut closed = rx;
                tokio::select! {
                    _ = closed.changed() => {}
                    r = conn => if let Err(e) = r {
                        tracing::debug!(port, error = %e, "a run proxy connection ended with an error");
                    },
                }
            });
        }
    });
    Ok(RunProxy { port, _closed: tx })
}

/// One request: refused unless pdtd's identity guard would let it in, otherwise sent to the run as it came.
async fn forward(
    st: AppState,
    client: Client<HttpConnector, Body>,
    target: u16,
    closed: watch::Receiver<()>,
    mut req: Request<Incoming>,
) -> Result<Response<Body>, Infallible> {
    if let Err(denied) = check(&crate::identity_guard::caller(req.headers()), st.owner().as_deref()) {
        tracing::warn!(reason = %denied, port = target, "refused a remote request to a run");
        return Ok(ApiError::forbidden(codes::FORBIDDEN_IDENTITY, denied.to_string()).into_response());
    }
    let path = req.uri().path_and_query().map_or("/", |p| p.as_str());
    let uri = match format!("http://127.0.0.1:{target}{path}").parse() {
        Ok(u) => u,
        Err(_) => return Ok(plain(StatusCode::BAD_REQUEST, "the request's path could not be read")),
    };
    let upgrade = req.headers().contains_key(hyper::header::UPGRADE).then(|| hyper::upgrade::on(&mut req));
    let (mut parts, body) = req.into_parts();
    parts.uri = uri;
    let mut resp = match client.request(Request::from_parts(parts, Body::new(body))).await {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(port = target, error = %e, "a run proxy could not reach its run");
            return Ok(plain(StatusCode::BAD_GATEWAY, "the run is not answering"));
        }
    };
    if resp.status() == StatusCode::SWITCHING_PROTOCOLS
        && let Some(from_caller) = upgrade
    {
        let to_run = hyper::upgrade::on(&mut resp);
        tokio::spawn(tunnel(from_caller, to_run, closed));
    }
    Ok(resp.map(Body::new))
}

/// Copies bytes both ways between an upgraded caller connection and the run's, until either side or the proxy ends.
async fn tunnel(
    from_caller: hyper::upgrade::OnUpgrade,
    to_run: hyper::upgrade::OnUpgrade,
    mut closed: watch::Receiver<()>,
) {
    let (a, b) = match tokio::try_join!(from_caller, to_run) {
        Ok(both) => both,
        Err(e) => {
            tracing::debug!(error = %e, "a run proxy could not upgrade a connection");
            return;
        }
    };
    let (mut a, mut b) = (TokioIo::new(a), TokioIo::new(b));
    tokio::select! {
        _ = closed.changed() => {}
        _ = tokio::io::copy_bidirectional(&mut a, &mut b) => {}
    }
}

fn plain(status: StatusCode, message: &'static str) -> Response<Body> {
    (status, message).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::ws::{Message, WebSocketUpgrade};
    use axum::http::HeaderMap;
    use futures_util::{SinkExt, StreamExt};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_tungstenite::tungstenite;

    const OWNER: (&str, &str) = ("Tailscale-User-Login", "me@example.com");
    const FWD: (&str, &str) = ("X-Forwarded-For", "100.64.0.2");

    /// A run's server: `/` answers with the Host and login it saw, `/ws` echoes WebSocket text.
    async fn run_server() -> u16 {
        let app = axum::Router::new()
            .route(
                "/",
                axum::routing::get(|h: HeaderMap| async move {
                    let get = |k: &str| h.get(k).map_or("-".to_string(), |v| v.to_str().unwrap().to_string());
                    format!("{} {}", get("host"), get("tailscale-user-login"))
                }),
            )
            .route(
                "/ws",
                axum::routing::get(|ws: WebSocketUpgrade| async move {
                    ws.on_upgrade(|mut s| async move {
                        while let Some(Ok(Message::Text(t))) = s.recv().await {
                            let _ = s.send(Message::Text(format!("echo {t}").into())).await;
                        }
                    })
                }),
            );
        let l = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        port
    }

    async fn state(owner: Option<&str>) -> (AppState, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let store = pdt_store::Store::open(&tmp.path().join("pdt.db")).await.unwrap();
        let st = AppState::with_platform(store, tmp.path().join("data"), crate::isolated_platform(tmp.path()));
        *st.owner_cache.write().unwrap() = owner.map(str::to_string);
        (st, tmp)
    }

    /// One HTTP/1.1 GET to the proxy: (status, body).
    async fn get(port: u16, headers: &[(&str, &str)]) -> (u16, String) {
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let mut req = "GET / HTTP/1.1\r\nHost: box.t.ts.net:5173\r\nConnection: close\r\n".to_string();
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");
        s.write_all(req.as_bytes()).await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        let status = out.split(' ').nth(1).unwrap().parse().unwrap();
        (status, out.split("\r\n\r\n").nth(1).unwrap_or_default().to_string())
    }

    #[tokio::test]
    async fn only_the_owner_and_this_machine_reach_the_run_and_it_sees_the_request_unchanged() {
        let (st, _tmp) = state(Some("me@example.com")).await;
        let proxy = start(&st, run_server().await).await.unwrap();
        assert_eq!(get(proxy.port, &[]).await, (200, "box.t.ts.net:5173 -".into()), "from this machine");
        assert_eq!(get(proxy.port, &[OWNER, FWD]).await, (200, "box.t.ts.net:5173 me@example.com".into()));
        let (s, body) = get(proxy.port, &[("Tailscale-User-Login", "eve@example.com"), FWD]).await;
        assert_eq!(s, 403, "{body}");
        assert!(body.contains("not this host's owner"), "{body}");
        assert_eq!(get(proxy.port, &[FWD]).await.0, 403, "a tagged device has no login");
        assert_eq!(get(proxy.port, &[OWNER, FWD, ("Tailscale-Funnel-Request", "?1")]).await.0, 403, "Funnel");
    }

    #[tokio::test]
    async fn without_an_owner_every_remote_request_is_refused() {
        let (st, _tmp) = state(None).await;
        let proxy = start(&st, run_server().await).await.unwrap();
        assert_eq!(get(proxy.port, &[OWNER, FWD]).await.0, 403);
    }

    #[tokio::test]
    async fn a_run_that_is_not_answering_is_a_bad_gateway() {
        let (st, _tmp) = state(Some("me@example.com")).await;
        let gone = TcpListener::bind(("127.0.0.1", 0)).await.unwrap().local_addr().unwrap().port();
        let proxy = start(&st, gone).await.unwrap();
        assert_eq!(get(proxy.port, &[OWNER, FWD]).await.0, 502);
    }

    #[tokio::test]
    async fn websockets_pass_through_and_stopping_the_proxy_closes_them() {
        let (st, _tmp) = state(Some("me@example.com")).await;
        let proxy = start(&st, run_server().await).await.unwrap();
        let connect = |login: &'static str| {
            let mut req = tungstenite::client::IntoClientRequest::into_client_request(format!(
                "ws://127.0.0.1:{}/ws",
                proxy.port
            ))
            .unwrap();
            req.headers_mut().insert("tailscale-user-login", login.parse().unwrap());
            req.headers_mut().insert("x-forwarded-for", FWD.1.parse().unwrap());
            tokio_tungstenite::connect_async(req)
        };
        assert!(connect("eve@example.com").await.is_err(), "a non-owner gets no WebSocket");
        let (mut ws, _) = connect("me@example.com").await.unwrap();
        ws.send(tungstenite::Message::Text("hi".into())).await.unwrap();
        let reply = ws.next().await.unwrap().unwrap();
        assert_eq!(reply.into_text().unwrap().as_str(), "echo hi");

        let port = proxy.port;
        drop(proxy);
        let ended = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while let Some(Ok(_)) = ws.next().await {}
        })
        .await;
        assert!(ended.is_ok(), "the tunnel closed with the proxy");
        let refused = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await;
        assert!(refused.is_ok(), "the proxy's port is closed");
    }
}
