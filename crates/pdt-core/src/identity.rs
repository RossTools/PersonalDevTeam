//! Who may use this host (spec §2, decision 1). `tailscale serve` adds `Tailscale-User-Login` for untagged tailnet
//! users and always adds `X-Forwarded-For`/`X-Forwarded-Host`; pdtd listens only on loopback, so a request with none
//! of these came from this machine.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    /// From this machine (no proxy headers).
    Local,
    /// Through `tailscale serve`, from a tailnet user.
    Tailnet { login: String },
    /// Through a proxy without a user identity: a tagged device, Funnel, or something else.
    Unidentified,
}

#[derive(Debug, Clone, Copy)]
pub struct IdentityHeaders<'a> {
    /// `Tailscale-User-Login`, already decoded.
    pub login: Option<&'a str>,
    /// `X-Forwarded-For` or `X-Forwarded-Host` is present.
    pub forwarded: bool,
    /// `Tailscale-Funnel-Request` is present.
    pub funnel: bool,
}

pub fn classify(h: IdentityHeaders<'_>) -> Caller {
    if h.funnel {
        return Caller::Unidentified;
    }
    match h.login.map(str::trim).filter(|l| !l.is_empty()) {
        Some(login) => Caller::Tailnet { login: login.to_string() },
        None if h.forwarded => Caller::Unidentified,
        None if h.login.is_some() => Caller::Unidentified,
        None => Caller::Local,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Denied {
    #[error("this host has no owner yet; set one in Settings on the host itself")]
    NoOwner,
    #[error("{0} is not this host's owner")]
    NotOwner(String),
    #[error("the request has no Tailscale user identity (tagged devices and Funnel are not allowed)")]
    Unidentified,
}

pub fn same_login(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

pub fn check(caller: &Caller, owner: Option<&str>) -> Result<(), Denied> {
    match caller {
        Caller::Local => Ok(()),
        Caller::Unidentified => Err(Denied::Unidentified),
        Caller::Tailnet { login } => match owner {
            None => Err(Denied::NoOwner),
            Some(o) if same_login(o, login) => Ok(()),
            Some(_) => Err(Denied::NotOwner(login.clone())),
        },
    }
}

/// Decodes an RFC 2047 "Q"-encoded UTF-8 value (`=?utf-8?q?…?=`), which Tailscale may use for non-ASCII values
/// (fact 5). Anything else, or anything malformed, is returned unchanged.
pub fn decode_header_value(raw: &str) -> String {
    let decoded = (|| {
        let inner = raw.strip_prefix("=?")?.strip_suffix("?=")?;
        let mut parts = inner.splitn(3, '?');
        let (charset, enc, text) = (parts.next()?, parts.next()?, parts.next()?);
        if !charset.eq_ignore_ascii_case("utf-8") || !enc.eq_ignore_ascii_case("q") {
            return None;
        }
        let mut bytes = Vec::with_capacity(text.len());
        let mut it = text.bytes();
        while let Some(b) = it.next() {
            match b {
                b'_' => bytes.push(b' '),
                b'=' => {
                    let hex = [it.next()?, it.next()?];
                    bytes.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
                }
                b => bytes.push(b),
            }
        }
        String::from_utf8(bytes).ok()
    })();
    decoded.unwrap_or_else(|| raw.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(login: Option<&str>, forwarded: bool, funnel: bool) -> IdentityHeaders<'_> {
        IdentityHeaders { login, forwarded, funnel }
    }

    #[test]
    fn classification() {
        assert_eq!(classify(h(None, false, false)), Caller::Local);
        assert_eq!(classify(h(Some("me@x.com"), true, false)), Caller::Tailnet { login: "me@x.com".into() });
        assert_eq!(classify(h(None, true, false)), Caller::Unidentified, "tagged device through serve");
        assert_eq!(classify(h(Some("  "), true, false)), Caller::Unidentified);
        assert_eq!(classify(h(Some("me@x.com"), true, true)), Caller::Unidentified, "funnel is never trusted");
        assert_eq!(classify(h(None, false, true)), Caller::Unidentified);
    }

    #[test]
    fn owner_check() {
        let me = Caller::Tailnet { login: "Me@X.com".into() };
        assert_eq!(check(&Caller::Local, None), Ok(()));
        assert_eq!(check(&me, Some(" me@x.com ")), Ok(()), "case and spaces do not matter");
        assert_eq!(check(&me, Some("other@x.com")), Err(Denied::NotOwner("Me@X.com".into())));
        assert_eq!(check(&me, None), Err(Denied::NoOwner));
        assert_eq!(check(&Caller::Unidentified, Some("me@x.com")), Err(Denied::Unidentified));
    }

    #[test]
    fn rfc2047_q_values_are_decoded() {
        assert_eq!(decode_header_value("me@example.com"), "me@example.com");
        assert_eq!(decode_header_value("=?utf-8?q?Ren=C3=A9_D?="), "René D");
        assert_eq!(decode_header_value("=?UTF-8?Q?a=3Db?="), "a=b");
        assert_eq!(decode_header_value("=?utf-8?b?bm90IHE=?="), "=?utf-8?b?bm90IHE=?=", "only Q is decoded");
        assert_eq!(decode_header_value("=?utf-8?q?bad=ZZ?="), "=?utf-8?q?bad=ZZ?=", "malformed stays raw");
    }

    #[test]
    fn denial_messages_are_readable() {
        assert!(Denied::NotOwner("a@b".into()).to_string().contains("a@b"));
        assert!(Denied::Unidentified.to_string().contains("tagged"));
    }
}
