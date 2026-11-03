//! Ticket links: `blocks` links must not form a cycle, and a ticket is blocked while a ticket that
//! blocks it is still open.

use std::collections::{HashMap, HashSet, VecDeque};

/// Adding `from` blocks `to` closes a cycle when `to` already (transitively) blocks `from`. Returns that cycle as
/// `[from, to, …, from]`, else None. `edges` are existing `blocks` links as (blocker, blocked).
pub fn would_cycle(edges: &[(String, String)], from: &str, to: &str) -> Option<Vec<String>> {
    if from == to {
        return Some(vec![from.to_string(), to.to_string()]);
    }
    let mut next: HashMap<&str, Vec<&str>> = HashMap::new();
    for (a, b) in edges {
        next.entry(a.as_str()).or_default().push(b.as_str());
    }
    // Breadth-first from `to`, remembering how each ticket was reached, until `from` turns up.
    let mut came_from: HashMap<&str, &str> = HashMap::new();
    let mut seen: HashSet<&str> = HashSet::from([to]);
    let mut queue = VecDeque::from([to]);
    while let Some(at) = queue.pop_front() {
        for &n in next.get(at).map(Vec::as_slice).unwrap_or(&[]) {
            if !seen.insert(n) {
                continue;
            }
            came_from.insert(n, at);
            if n == from {
                let mut path = vec![from.to_string()];
                let mut cur = from;
                while let Some(&prev) = came_from.get(cur) {
                    path.push(prev.to_string());
                    cur = prev;
                }
                path.push(from.to_string());
                path.reverse();
                return Some(path);
            }
            queue.push_back(n);
        }
    }
    None
}

/// The tickets that block `ticket` and are still open (`is_open` false means done or won't do).
pub fn open_blockers<'a>(edges: &'a [(String, String)], ticket: &str, is_open: impl Fn(&str) -> bool) -> Vec<&'a str> {
    edges.iter().filter(|(a, b)| b == ticket && is_open(a)).map(|(a, _)| a.as_str()).collect()
}

/// Blocked: some open ticket blocks it.
pub fn is_blocked(edges: &[(String, String)], ticket: &str, is_open: impl Fn(&str) -> bool) -> bool {
    !open_blockers(edges, ticket, is_open).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn a_link_that_closes_a_cycle_is_found_with_its_path() {
        let edges = e(&[("a", "b"), ("b", "c"), ("x", "y")]);
        assert_eq!(would_cycle(&edges, "c", "a"), Some(vec!["c".into(), "a".into(), "b".into(), "c".into()]));
        assert_eq!(would_cycle(&edges, "b", "a"), Some(vec!["b".into(), "a".into(), "b".into()]));
        assert_eq!(would_cycle(&edges, "a", "a"), Some(vec!["a".into(), "a".into()]));
        assert_eq!(would_cycle(&edges, "a", "c"), None, "a shortcut is not a cycle");
        assert_eq!(would_cycle(&edges, "c", "x"), None);
        assert_eq!(would_cycle(&[], "a", "b"), None);
    }

    #[test]
    fn diamonds_and_long_chains() {
        let edges = e(&[("a", "b"), ("a", "c"), ("b", "d"), ("c", "d")]);
        assert_eq!(would_cycle(&edges, "d", "a").map(|p| p.len()), Some(4));
        let chain: Vec<(String, String)> = (0..500).map(|i| (format!("t{i}"), format!("t{}", i + 1))).collect();
        assert_eq!(would_cycle(&chain, "t500", "t0").map(|p| p.len()), Some(502));
        assert_eq!(would_cycle(&chain, "t0", "t500"), None);
    }

    #[test]
    fn blocked_while_an_open_ticket_blocks_it() {
        let edges = e(&[("a", "c"), ("b", "c"), ("c", "d")]);
        let open = |t: &str| t != "b";
        assert_eq!(open_blockers(&edges, "c", open), ["a"]);
        assert!(is_blocked(&edges, "c", open));
        assert!(!is_blocked(&edges, "c", |t| t == "c"));
        assert!(!is_blocked(&edges, "a", |_| true));
    }
}
