//! PDT's own Google OAuth clients (PDT-360), so adding a Google account is just Sign in with Google. A release build
//! has them from its environment (`PDT_GOOGLE_*`, GitHub Actions secrets, never committed); a build without them
//! offers only the user's own client. There are two Web clients, after Google's rule for apps that are partly an AI
//! agent: the standard one for what the user does (the picker, editing, the embed) and the agentic one for agents'
//! document tools, each account consenting to it separately.
//!
//! Their only redirect URI is the website's `/oauth/google` page: Google can't send the browser back to wherever PDT
//! is open (a loopback address, a Tailscale name). The page reads where to go from OAuth's `state`, which is
//! `<nonce>.<base64url(origin)>` ([`state`]), forwards only to an address that looks like a PDT ([`pdt_origin`], the
//! same rule as the page's `site/public/oauth/google.js`), and pdtd takes only the states it issued. The browser
//! starts at that page too (`AccountSignInStart::relay`), which takes the consent page only from the PDT the state
//! names and forwards only states it started, so a Google link made with these clients elsewhere goes nowhere (PDT-410).

use base64::Engine;
use std::net::{Ipv4Addr, Ipv6Addr};

/// An OAuth client: its ID and secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    pub id: String,
    pub secret: String,
}

/// PDT's own clients, and the file picker's API key and project number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltIn {
    /// For the user's own actions.
    pub standard: Client,
    /// For agents' document tools.
    pub agent: Client,
    /// The picker's API key and the Google Cloud project's number.
    pub picker: Option<(String, String)>,
}

impl BuiltIn {
    /// The clients this build was made with, if it was made with them.
    pub fn from_build() -> Option<Self> {
        Self::from_values([
            option_env!("PDT_GOOGLE_CLIENT_ID"),
            option_env!("PDT_GOOGLE_CLIENT_SECRET"),
            option_env!("PDT_GOOGLE_AGENT_CLIENT_ID"),
            option_env!("PDT_GOOGLE_AGENT_CLIENT_SECRET"),
            option_env!("PDT_GOOGLE_PICKER_KEY"),
            option_env!("PDT_GOOGLE_PROJECT_NUMBER"),
        ])
    }

    /// From the standard client's ID and secret, the agentic client's, the picker key and the project number: both
    /// clients or none; the picker only with its key and the number.
    pub fn from_values(v: [Option<&str>; 6]) -> Option<Self> {
        let [id, secret, agent_id, agent_secret, key, number] = v.map(|s| s.map(str::trim).filter(|s| !s.is_empty()));
        let client = |id: Option<&str>, secret: Option<&str>| Some(Client { id: id?.into(), secret: secret?.into() });
        Some(Self {
            standard: client(id, secret)?,
            agent: client(agent_id, agent_secret)?,
            picker: key.zip(number).map(|(k, n)| (k.to_string(), n.to_string())),
        })
    }
}

/// Where Google sends the browser back to, with PDT's own clients.
pub fn redirect_uri() -> String {
    format!("{}/oauth/google", pdt_core::brand::WEBSITE)
}

/// OAuth's `state` for a sign-in started at `origin`: the nonce PDT keeps the sign-in under, and the address the
/// website's page sends the browser back to.
pub fn state(nonce: &str, origin: &str) -> String {
    format!("{nonce}.{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(origin))
}

/// `origin` (`http://127.0.0.1:7420`), as browsers write it, when it looks like a PDT: loopback, a private or
/// Tailscale address, `localhost` or a `*.ts.net` name, on http or https, with no path. The website's page forwards
/// the browser only there.
pub fn pdt_origin(origin: &str) -> Result<String, String> {
    let refuse = || {
        format!(
            "{origin} doesn't look like a PDT's address (http://host:port on this machine, your network or Tailscale)"
        )
    };
    let o = origin.trim().trim_end_matches('/').to_ascii_lowercase();
    let (scheme, rest) = o.split_once("://").ok_or_else(refuse)?;
    if !matches!(scheme, "http" | "https") || rest.is_empty() || rest.contains(['/', '?', '#', '@', '\\', ' ', '%']) {
        return Err(refuse());
    }
    let (host, port) = match rest.strip_prefix('[') {
        Some(r) => {
            let (h, after) = r.split_once(']').ok_or_else(refuse)?;
            let port = match after {
                "" => None,
                p => Some(p.strip_prefix(':').ok_or_else(refuse)?),
            };
            (h, port)
        }
        None => match rest.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (rest, None),
        },
    };
    if let Some(p) = port
        && (p.is_empty() || p.len() > 5 || !p.bytes().all(|b| b.is_ascii_digit()) || p.parse::<u16>().is_err())
    {
        return Err(refuse());
    }
    let ok = if rest.starts_with('[') {
        host.parse::<Ipv6Addr>().is_ok_and(|ip| ip.is_loopback() || (ip.segments()[0] & 0xfe00) == 0xfc00)
    } else if let Ok(ip) = host.parse::<Ipv4Addr>() {
        let [a, b, ..] = ip.octets();
        // Tailscale's addresses are in 100.64.0.0/10.
        ip.is_loopback() || ip.is_private() || (a == 100 && (b & 0xc0) == 64)
    } else {
        let labels: Vec<&str> = host.split('.').collect();
        let named = labels.iter().all(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
            && !host.bytes().all(|b| b.is_ascii_digit() || b == b'.');
        named
            && (host == "localhost" || host.ends_with(".localhost") || (labels.len() > 2 && host.ends_with(".ts.net")))
    };
    if !ok {
        return Err(refuse());
    }
    // Browsers leave the scheme's own port out.
    let rest = match (scheme, port) {
        ("http", Some("80")) | ("https", Some("443")) => rest.rsplit_once(':').map_or(rest, |(h, _)| h),
        _ => rest,
    };
    Ok(format!("{scheme}://{rest}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_has_both_clients_or_none() {
        let all = [Some("cid"), Some("cs"), Some("aid"), Some("as"), Some("key"), Some("123")];
        let b = BuiltIn::from_values(all).unwrap();
        assert_eq!((b.standard.id.as_str(), b.agent.secret.as_str()), ("cid", "as"));
        assert_eq!(b.picker, Some(("key".into(), "123".into())));
        assert_eq!(BuiltIn::from_values([Some("cid"), Some("cs"), None, Some("as"), None, None]), None);
        assert_eq!(BuiltIn::from_values([Some(" "), Some("cs"), Some("aid"), Some("as"), None, None]), None);
        let no_picker = BuiltIn::from_values([Some("cid"), Some("cs"), Some("aid"), Some("as"), Some("key"), None]);
        assert_eq!(no_picker.unwrap().picker, None);
        assert_eq!(BuiltIn::from_values([None; 6]), None);
    }

    #[test]
    fn the_state_carries_the_origin() {
        assert_eq!(state("n0nce", "http://127.0.0.1:7420"), "n0nce.aHR0cDovLzEyNy4wLjAuMTo3NDIw");
        assert_eq!(redirect_uri(), "https://pdt.dev/oauth/google");
    }

    /// The same cases as the website page's tests (`site/src/oauth-google.test.mjs`).
    #[test]
    fn only_addresses_that_look_like_a_pdt() {
        for (given, origin) in [
            ("http://127.0.0.1:7420", "http://127.0.0.1:7420"),
            ("http://localhost:7421/", "http://localhost:7421"),
            ("http://[::1]:7420", "http://[::1]:7420"),
            ("http://192.168.1.20:7420", "http://192.168.1.20:7420"),
            ("http://10.0.0.5:7420", "http://10.0.0.5:7420"),
            ("http://172.20.1.1:7420", "http://172.20.1.1:7420"),
            ("http://100.101.102.103:7420", "http://100.101.102.103:7420"),
            ("https://mac.tail1234.ts.net", "https://mac.tail1234.ts.net"),
            ("https://Mac.Tail1234.ts.net:443", "https://mac.tail1234.ts.net"),
            ("http://mac.tail1234.ts.net:7420", "http://mac.tail1234.ts.net:7420"),
            ("http://[fd7a:115c:a1e0::1]:7420", "http://[fd7a:115c:a1e0::1]:7420"),
        ] {
            assert_eq!(pdt_origin(given).as_deref(), Ok(origin), "{given}");
        }
        for bad in [
            "https://evil.example",
            "https://ts.net",
            "https://evil.com/.ts.net",
            "https://evil.com#.ts.net",
            "https://evil.com?.ts.net",
            "http://127.0.0.1.evil.com",
            "http://user@127.0.0.1:7420",
            "http://127.0.0.1:7420/api",
            "http://8.8.8.8",
            "http://172.32.0.1",
            "http://100.128.0.1",
            "http://[2001:db8::1]",
            "http://[fc::1]",
            "http://0x7f.0.0.1",
            "http://2130706433",
            "http://127.0.0.1:99999",
            "http://127.0.0.1:",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "ftp://127.0.0.1",
            "",
        ] {
            assert!(pdt_origin(bad).is_err(), "{bad}");
        }
    }
}
