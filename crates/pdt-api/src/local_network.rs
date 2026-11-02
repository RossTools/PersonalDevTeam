//! Asking for macOS's Local Network permission while the user is at the Mac (PDT-156). macOS asks the first time a
//! program reaches the local network; if that first time is a sensor or a workflow step, nobody sees the alert and
//! the use just fails. So when pdtd starts, and when a connection is saved, pdtd connects a UDP socket to the first
//! connection address that is on the local network: that brings up the alert without sending anything (Apple's
//! TN3179). Nothing is probed when no connection is local, so a user who never adds one is never asked.

use crate::routes::AppState;
use pdt_core::local_network::is_local_ip;
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);

/// A connection URL's host (without IPv6 brackets) and port.
fn host_port(url: &str) -> Option<(String, u16)> {
    let u = reqwest::Url::parse(url.trim()).ok()?;
    let host = u.host_str()?.trim_start_matches('[').trim_end_matches(']').to_string();
    Some((host, u.port_or_known_default().unwrap_or(80)))
}

/// The local network address `url` reaches, if any (a name is resolved; resolving a `.local` name may itself bring up
/// the alert).
async fn local_addr(url: &str) -> Option<SocketAddr> {
    let (host, port) = host_port(url)?;
    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_local_ip(ip).then_some(SocketAddr::new(ip, port));
    }
    let addrs =
        tokio::time::timeout(RESOLVE_TIMEOUT, tokio::net::lookup_host((host.as_str(), port))).await.ok()?.ok()?;
    addrs.into_iter().find(|a| is_local_ip(a.ip()))
}

/// Connects a UDP socket to `addr`: a local network operation that sends no packet.
fn touch(addr: SocketAddr) -> std::io::Result<()> {
    let any: SocketAddr = if addr.is_ipv4() { "0.0.0.0:0" } else { "[::]:0" }.parse().expect("an address");
    std::net::UdpSocket::bind(any)?.connect(addr)
}

/// On macOS, touches the first local network address among the connections' URLs, so macOS asks for the Local
/// Network permission now (it asks once; once answered this changes nothing).
pub async fn prompt(st: &AppState) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let conns = match st.store.list_connections().await {
        Ok(c) => c,
        Err(e) => return tracing::warn!(error = %e, "could not read the connections"),
    };
    for c in conns.iter().filter(|c| !c.url.trim().is_empty()) {
        if let Some(addr) = local_addr(&c.url).await {
            match touch(addr) {
                Ok(()) => {
                    tracing::info!(connection = %c.name, %addr, "touched the local network (Local Network permission)")
                }
                Err(e) => {
                    tracing::info!(connection = %c.name, %addr, error = %e, "the local network is not reachable yet")
                }
            }
            return;
        }
    }
}

/// [`prompt`] in the background.
pub(crate) fn prompt_soon(st: &AppState) {
    let st = st.clone();
    tokio::spawn(async move { prompt(&st).await });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_and_port_come_from_the_url() {
        assert_eq!(host_port("http://192.168.1.10:8080/"), Some(("192.168.1.10".into(), 8080)));
        assert_eq!(host_port("https://nas.local"), Some(("nas.local".into(), 443)));
        assert_eq!(host_port("http://[fd00::1]:8080/x"), Some(("fd00::1".into(), 8080)));
        assert_eq!(host_port("not a url"), None);
    }

    #[tokio::test]
    async fn only_local_addresses_are_touched() {
        assert_eq!(local_addr("http://192.168.1.10:8080/").await, Some("192.168.1.10:8080".parse().unwrap()));
        assert_eq!(local_addr("https://8.8.8.8/").await, None);
        assert_eq!(local_addr("http://127.0.0.1:7/").await, None);
        assert_eq!(local_addr("").await, None);
    }

    #[test]
    fn touching_sends_nothing_and_needs_no_listener() {
        // Loopback needs no permission anywhere; nothing listens on port 9 and UDP connect sends no packet.
        touch("127.0.0.1:9".parse().unwrap()).unwrap();
    }
}
