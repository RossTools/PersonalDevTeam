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
//! Where that page can't hand the answer back (a corporate proxy isolating the website, no storage), it shows it as a
//! code to paste into PDT instead ([`pasted`]), still useless without the sign-in's state and PKCE verifier (PDT-590).
//!
//! When PDT is open on the same machine (a loopback origin, [`loopback_redirect`]), the website isn't needed: a build
//! may also have two Desktop app clients ([`Desktop`]), which Google lets come straight back to PDT's own port on
//! `127.0.0.1` (RFC 8252), so signing in there never loads the website (PDT-590). A Desktop client's secret isn't
//! confidential by design (it ships in every copy of the app); PKCE is what protects its codes.

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
    /// The Desktop app clients, for signing in on the same machine; without them that goes through the website too.
    pub desktop: Option<Desktop>,
}

/// PDT's Desktop app clients, in the same Google Cloud project as the Web ones (so files picked with one are the
/// others' too): Google sends the browser back to a loopback address on any port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Desktop {
    pub standard: Client,
    pub agent: Client,
}

impl Desktop {
    /// From the standard Desktop client's ID and secret and the agentic one's: both or none.
    pub fn from_values(v: [Option<&str>; 4]) -> Option<Self> {
        let [id, secret, agent_id, agent_secret] = v.map(|s| s.map(str::trim).filter(|s| !s.is_empty()));
        Some(Self { standard: client(id, secret)?, agent: client(agent_id, agent_secret)? })
    }
    /// The one for agents' document tools, or the user's own actions.
    pub fn client(&self, agent: bool) -> &Client {
        if agent { &self.agent } else { &self.standard }
    }
}

fn client(id: Option<&str>, secret: Option<&str>) -> Option<Client> {
    Some(Client { id: id?.into(), secret: secret?.into() })
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
        .map(|b| Self {
            desktop: Desktop::from_values([
                option_env!("PDT_GOOGLE_DESKTOP_CLIENT_ID"),
                option_env!("PDT_GOOGLE_DESKTOP_CLIENT_SECRET"),
                option_env!("PDT_GOOGLE_DESKTOP_AGENT_CLIENT_ID"),
                option_env!("PDT_GOOGLE_DESKTOP_AGENT_CLIENT_SECRET"),
            ]),
            ..b
        })
    }

    /// From the standard client's ID and secret, the agentic client's, the picker key and the project number: both
    /// clients or none; the picker only with its key and the number. No Desktop clients ([`BuiltIn::desktop`]).
    pub fn from_values(v: [Option<&str>; 6]) -> Option<Self> {
        let [id, secret, agent_id, agent_secret, key, number] = v.map(|s| s.map(str::trim).filter(|s| !s.is_empty()));
        Some(Self {
            standard: client(id, secret)?,
            agent: client(agent_id, agent_secret)?,
            picker: key.zip(number).map(|(k, n)| (k.to_string(), n.to_string())),
            desktop: None,
        })
    }
}

/// Where Google sends the browser back to, with PDT's own clients.
pub fn redirect_uri() -> String {
    format!("{}/oauth/google", pdt_core::brand::WEBSITE)
}

/// Where Google sends the browser back to with a Desktop client, when PDT is open at `origin` on this machine:
/// PDT's own callback on its loopback address (`http://127.0.0.1:7420/api/google/callback`). None anywhere else (a
/// LAN or Tailscale address, https), where the website's page is the way back.
pub fn loopback_redirect(origin: &str) -> Option<String> {
    let o = pdt_origin(origin).ok()?;
    let rest = o.strip_prefix("http://")?;
    let host = match rest.strip_prefix('[') {
        Some(r) => r.split_once(']')?.0,
        None => rest.split_once(':').map_or(rest, |(h, _)| h),
    };
    matches!(host, "127.0.0.1" | "::1" | "localhost").then(|| format!("{o}/api/google/callback"))
}

/// What the website's page shows to paste into PDT when it can't send the browser back: Google's answer (`code`,
/// `state`) as one line of base64url. Read back by [`pasted`].
pub fn paste_code(state: &str, code: &str) -> String {
    let q = pdt_forge::transport::form(&[("state", state), ("code", code)]);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(q)
}

/// Google's answer (`state`, `code`) from what the user pasted: the page's code ([`paste_code`]), or the address the
/// browser was sent back to (`…/api/google/callback?state=…&code=…`).
pub fn pasted(text: &str) -> Option<(String, String)> {
    let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let query = match t.split_once('?') {
        Some((_, q)) => q.split('#').next().unwrap_or_default().to_string(),
        None => {
            let b = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(t.trim_end_matches('=')).ok()?;
            String::from_utf8(b).ok()?
        }
    };
    let (mut state, mut code) = (None, None);
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = unescape(v)?;
        match k {
            "state" => state = Some(v),
            "code" => code = Some(v),
            _ => {}
        }
    }
    Some((state.filter(|s| !s.is_empty())?, code.filter(|c| !c.is_empty())?))
}

/// A query string's value, unescaped (`%2F`, `+`).
fn unescape(v: &str) -> Option<String> {
    let b = v.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
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
    fn desktop_clients_come_both_or_none() {
        let d = Desktop::from_values([Some("did"), Some("ds"), Some("daid"), Some(" das ")]).unwrap();
        assert_eq!((d.client(false).id.as_str(), d.client(true).secret.as_str()), ("did", "das"));
        assert_eq!(Desktop::from_values([Some("did"), Some("ds"), Some(""), Some("das")]), None);
        assert_eq!(Desktop::from_values([None; 4]), None);
        let all = [Some("cid"), Some("cs"), Some("aid"), Some("as"), None, None];
        assert_eq!(BuiltIn::from_values(all).unwrap().desktop, None);
    }

    #[test]
    fn only_this_machine_comes_straight_back() {
        for (origin, to) in [
            ("http://127.0.0.1:7420", "http://127.0.0.1:7420/api/google/callback"),
            ("http://127.0.0.1:7421/", "http://127.0.0.1:7421/api/google/callback"),
            ("http://localhost:7420", "http://localhost:7420/api/google/callback"),
            ("http://[::1]:7420", "http://[::1]:7420/api/google/callback"),
        ] {
            assert_eq!(loopback_redirect(origin).as_deref(), Some(to), "{origin}");
        }
        for elsewhere in [
            "http://192.168.1.20:7420",
            "http://100.101.102.103:7420",
            "https://mac.tail1234.ts.net",
            "https://127.0.0.1:7420",
            "http://127.0.0.2:7420",
            "http://pdt.localhost:7420",
            "https://evil.example",
            "",
        ] {
            assert_eq!(loopback_redirect(elsewhere), None, "{elsewhere}");
        }
    }

    #[test]
    fn a_pasted_code_reads_back_googles_answer() {
        let (state, code) = ("n0nce.aHR0cDovLzEyNy4wLjAuMTo3NDIw", "4/0Ab_c-d+e");
        let want = Some((state.to_string(), code.to_string()));
        assert_eq!(pasted(&paste_code(state, code)), want);
        // Wrapped or with spaces around it, as copied.
        let c = paste_code(state, code);
        assert_eq!(pasted(&format!("  {}\n{} ", &c[..10], &c[10..])), want);
        // The address the browser was sent back to works too.
        let url = format!(
            "http://127.0.0.1:7420/api/google/callback?{}",
            pdt_forge::transport::form(&[("state", state), ("code", code), ("scope", "x")])
        );
        assert_eq!(pasted(&url), want);
        // The site's page writes it the same way (`site/src/oauth-google.test.mjs`).
        assert_eq!(pasted("c3RhdGU9bjBuY2UuYUhSMGNEb3ZMekV5Tnk0d0xqQXVNVG8zTkRJdyZjb2RlPTQlMkYwQWJfYy1kJTJCZQ"), want);
        for bad in ["", "not base64!", &paste_code("", code), &paste_code(state, ""), "http://x/?state=s", "%zz"] {
            assert_eq!(pasted(bad), None, "{bad}");
        }
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
