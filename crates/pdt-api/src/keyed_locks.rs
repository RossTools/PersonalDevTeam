//! Locks by key: one async mutex per key (a ticket's workflow runs, a project's ticket Starts), made on first use.

use std::collections::HashMap;
use std::sync::Arc;

/// One lock per key. An entry nobody holds or waits for is dropped on the next lookup.
#[derive(Default)]
pub(crate) struct KeyedLocks(std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>);

impl KeyedLocks {
    fn get(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        m.retain(|_, l| Arc::strong_count(l) > 1);
        m.entry(key.to_string()).or_default().clone()
    }

    pub(crate) async fn lock(&self, key: &str) -> tokio::sync::OwnedMutexGuard<()> {
        self.get(key).lock_owned().await
    }

    pub(crate) fn try_lock(&self, key: &str) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        self.get(key).try_lock_owned().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn one_key_waits_and_others_do_not() {
        let locks = KeyedLocks::default();
        let held = locks.lock("a").await;
        assert!(locks.try_lock("a").is_none());
        assert!(locks.try_lock("b").is_some());
        drop(held);
        assert!(locks.try_lock("a").is_some());
    }
}
