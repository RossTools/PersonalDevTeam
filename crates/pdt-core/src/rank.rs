//! Fractional ranks: base-62 digit strings read as fractions in (0, 1). Lexicographic order of keys
//! is their order, so moving an item rewrites only its own key. A key never ends in `0` (the smallest digit), which
//! keeps every key unique per value and leaves room below any key.
//!
//! Keys grow only when one gap is split again and again; [`needs_rebalance`] says when to rewrite a list's keys with
//! [`spread`].

/// The digits in ascending order; ASCII order matches digit order.
const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE: u8 = 62;

/// Keys longer than this trigger a rebalance.
pub const REBALANCE_LEN: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RankError {
    #[error("not a rank key: {0:?}")]
    Invalid(String),
    #[error("rank {0:?} is not below {1:?}")]
    Order(String, String),
}

fn digit(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'A'..=b'Z' => Some(c - b'A' + 10),
        b'a'..=b'z' => Some(c - b'a' + 36),
        _ => None,
    }
}

/// Non-empty, base-62 digits only, not ending in `0`.
pub fn is_valid(key: &str) -> bool {
    !key.is_empty() && !key.ends_with('0') && key.bytes().all(|c| digit(c).is_some())
}

fn digits(key: &str) -> Result<Vec<u8>, RankError> {
    if !is_valid(key) {
        return Err(RankError::Invalid(key.to_string()));
    }
    Ok(key.bytes().map(|c| digit(c).expect("checked")).collect())
}

/// Digits of a fraction strictly between `a` (empty = 0) and `b` (None = 1), with no trailing zero. Needs a < b.
fn midpoint(a: &[u8], b: Option<&[u8]>) -> Vec<u8> {
    if let Some(b) = b {
        // Shared leading digits (a padded with zeros) stay as they are.
        let n = b.iter().enumerate().take_while(|(i, d)| a.get(*i).copied().unwrap_or(0) == **d).count();
        if n > 0 {
            let mut out = b[..n].to_vec();
            out.extend(midpoint(a.get(n..).unwrap_or(&[]), Some(&b[n..])));
            return out;
        }
    }
    let da = a.first().copied().unwrap_or(0);
    let db = b.map(|b| b[0]).unwrap_or(BASE);
    if db - da > 1 {
        return vec![(da + db) / 2];
    }
    // Adjacent first digits.
    match b {
        // `b` cut to its first digit is still below `b` (b continues with a non-zero digit) and above `a`.
        Some(b) if b.len() > 1 => vec![db],
        _ => {
            let mut out = vec![da];
            out.extend(midpoint(a.get(1..).unwrap_or(&[]), None));
            out
        }
    }
}

fn to_key(d: &[u8]) -> String {
    d.iter().map(|&x| DIGITS[x as usize] as char).collect()
}

/// A key strictly between `a` and `b`; either may be absent (the start or end of the list).
pub fn between(a: Option<&str>, b: Option<&str>) -> Result<String, RankError> {
    let da = a.map(digits).transpose()?.unwrap_or_default();
    let db = b.map(digits).transpose()?;
    if let (Some(a), Some(b)) = (a, b)
        && a >= b
    {
        return Err(RankError::Order(a.to_string(), b.to_string()));
    }
    Ok(to_key(&midpoint(&da, db.as_deref())))
}

/// True when a key has grown long enough that its list should be rewritten with [`spread`].
pub fn needs_rebalance(key: &str) -> bool {
    key.len() > REBALANCE_LEN
}

/// `n` ascending keys spaced evenly over (0, 1), as short as possible.
pub fn spread(n: usize) -> Vec<String> {
    if n == 0 {
        return vec![];
    }
    let slots = n as u128 + 1;
    let mut len = 1u32;
    while (BASE as u128).pow(len) < slots * 2 {
        len += 1;
    }
    let scale = (BASE as u128).pow(len);
    (1..=n as u128)
        .map(|i| {
            let mut v = i * scale / slots;
            let mut d = vec![0u8; len as usize];
            for slot in d.iter_mut().rev() {
                *slot = (v % BASE as u128) as u8;
                v /= BASE as u128;
            }
            while d.last() == Some(&0) {
                d.pop();
            }
            to_key(&d)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic generator, so the property tests need no extra crate.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self, n: usize) -> usize {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 33) as usize) % n
        }
    }

    fn b(a: Option<&str>, c: Option<&str>) -> String {
        let k = between(a, c).unwrap();
        assert!(is_valid(&k), "{k:?}");
        if let Some(a) = a {
            assert!(a < k.as_str(), "{a:?} < {k:?}");
        }
        if let Some(c) = c {
            assert!(k.as_str() < c, "{k:?} < {c:?}");
        }
        k
    }

    #[test]
    fn between_handles_open_ends_and_neighbours() {
        assert_eq!(b(None, None), "V");
        b(Some("V"), None);
        b(None, Some("V"));
        b(Some("V"), Some("W"));
        b(Some("V"), Some("V1"));
        b(Some("0V"), Some("1"));
        b(None, Some("01"));
        b(None, Some("001"));
        b(Some("zz"), None);
        b(Some("z"), Some("zz"));
        b(Some("Vz"), Some("W"));
        b(Some("V"), Some("VzzzzzX"));
    }

    #[test]
    fn between_refuses_bad_input() {
        assert!(matches!(between(Some("W"), Some("V")), Err(RankError::Order(..))));
        assert!(matches!(between(Some("V"), Some("V")), Err(RankError::Order(..))));
        for bad in ["", "V0", "a-b", "é"] {
            assert!(matches!(between(Some(bad), None), Err(RankError::Invalid(_))), "{bad:?}");
        }
    }

    #[test]
    fn random_inserts_stay_sorted_and_unique() {
        let mut rng = Lcg(7);
        let mut keys: Vec<String> = vec![];
        for _ in 0..2000 {
            let at = rng.next(keys.len() + 1);
            let a = at.checked_sub(1).map(|i| keys[i].as_str());
            let c = keys.get(at).map(String::as_str);
            let k = b(a, c);
            keys.insert(at, k);
        }
        assert!(keys.windows(2).all(|w| w[0] < w[1]));
        assert!(keys.iter().all(|k| k.len() < 20), "random inserts keep keys short");
    }

    #[test]
    fn repeatedly_splitting_one_gap_grows_slowly_and_triggers_a_rebalance() {
        // Always insert right after the first key: the worst case.
        let (lo, mut hi) = ("V".to_string(), "W".to_string());
        let mut n = 0;
        while !needs_rebalance(&hi) {
            hi = b(Some(&lo), Some(&hi));
            n += 1;
        }
        assert!(n > 100, "{n} splits before a rebalance");
        // Always insert at the top.
        let (mut top, mut n) = ("V".to_string(), 0);
        while !needs_rebalance(&top) {
            top = b(None, Some(&top));
            n += 1;
        }
        assert!(n > 150, "{n} inserts at the top before a rebalance");
    }

    #[test]
    fn spread_gives_short_even_sorted_keys() {
        assert!(spread(0).is_empty());
        assert_eq!(spread(1), ["V"]);
        for n in [2, 3, 61, 62, 1000, 5000] {
            let keys = spread(n);
            assert_eq!(keys.len(), n);
            assert!(keys.iter().all(|k| is_valid(k)), "{n}");
            assert!(keys.windows(2).all(|w| w[0] < w[1]), "{n}");
            assert!(keys.iter().all(|k| k.len() <= 3), "{n}");
            // There is room around every key.
            b(None, Some(&keys[0]));
            b(Some(&keys[n - 1]), None);
        }
    }
}
