//! macOS Local Network privacy (PDT-156). Since macOS 15 a program needs the user's Local Network permission to
//! connect to an address on the local network; without it the connection fails with EHOSTUNREACH ("No route to
//! host", os error 65), which reads like a network fault. These rules tell such a failure apart, so PDT can name the
//! setting instead, and say which addresses are local (Apple's TN3179).

use crate::brand::PRODUCT_NAME;
use std::net::IpAddr;

/// EHOSTUNREACH on macOS: what a connection the Local Network permission blocks fails with.
pub const EHOSTUNREACH_MACOS: i32 = 65;

/// Whether `ip` is on a local network as far as macOS's Local Network privacy goes: the private ranges (RFC 1918,
/// IPv6 unique local), link-local, multicast and the IPv4 broadcast address. Loopback is not (it needs no permission).
pub fn is_local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local() || v4.is_multicast() || v4.is_broadcast(),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_local_ip(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80 || v6.is_multicast()
        }
    }
}

/// Whether a URL's host is known to be on the local network: a local IP address (IPv6 in brackets or not) or a
/// `.local` (mDNS) name. Other names may resolve to either; they are not.
pub fn is_local_host(host: &str) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return is_local_ip(ip);
    }
    let name = host.trim_end_matches('.').to_ascii_lowercase();
    name == "local" || name.ends_with(".local")
}

/// What to tell the user when the Local Network permission blocks PDT.
pub fn blocked_message() -> String {
    format!(
        "macOS Local Network permission is blocking {PRODUCT_NAME}: System Settings › Privacy & Security › Local \
         Network › turn on {PRODUCT_NAME}"
    )
}

/// The message for a failed connection to `host` with OS error `os_error`, when it is the Local Network permission's
/// doing: on macOS, EHOSTUNREACH to a local host. None for any other failure (a public address, another error,
/// another OS).
pub fn explain_failure(macos: bool, host: Option<&str>, os_error: Option<i32>) -> Option<String> {
    (macos && os_error == Some(EHOSTUNREACH_MACOS) && host.is_some_and(is_local_host)).then(blocked_message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_link_local_and_mdns_hosts_are_local() {
        for h in [
            "192.168.0.69",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "169.254.10.1",
            "224.0.0.251",
            "255.255.255.255",
            "[fd12:3456::1]",
            "fe80::1",
            "[::ffff:192.168.1.2]",
            "nas.local",
            "NAS.Local.",
        ] {
            assert!(is_local_host(h), "{h}");
        }
    }

    #[test]
    fn public_loopback_and_other_names_are_not_local() {
        for h in [
            "8.8.8.8",
            "172.32.0.1",
            "100.64.0.1",
            "127.0.0.1",
            "[::1]",
            "2606:4700::1111",
            "localhost",
            "example.com",
            "localish.com",
        ] {
            assert!(!is_local_host(h), "{h}");
        }
    }

    #[test]
    fn host_unreachable_to_a_private_address_on_macos_names_the_setting() {
        let m = explain_failure(true, Some("192.168.0.69"), Some(65)).unwrap();
        assert!(m.contains("Local Network") && m.contains("System Settings") && m.contains(PRODUCT_NAME), "{m}");
    }

    #[test]
    fn other_failures_keep_their_own_error() {
        assert_eq!(explain_failure(true, Some("8.8.8.8"), Some(65)), None, "a public address");
        assert_eq!(explain_failure(true, Some("example.com"), Some(65)), None, "a name that may not be local");
        assert_eq!(explain_failure(true, Some("192.168.0.69"), Some(61)), None, "connection refused");
        assert_eq!(explain_failure(true, Some("192.168.0.69"), None), None, "no OS error");
        assert_eq!(explain_failure(true, None, Some(65)), None, "no host");
        assert_eq!(explain_failure(false, Some("192.168.0.69"), Some(65)), None, "not macOS");
    }
}
