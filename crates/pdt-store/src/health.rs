//! Writes that fail (a full disk, an I/O error, a read-only file) and getting over them (PDT-323): connections come
//! back to the pool clean, the failure is known while it lasts (for the Inbox and the engine's backoff), and events
//! that could not be logged wait in memory to be logged once writes work again.

use crate::{Store, StoreError};
use pdt_core::workflow::api::NewLogEvent;
use sqlx::Connection;
use std::sync::Mutex;

/// How every failed write's message starts ([`StoreError::Write`]): callers that only have the text tell a failed
/// write from their own failure by it ([`is_write_failure`]).
pub const WRITE_FAILED: &str = "PDT can't save changes"; // `StoreError::Write` says it too.

/// The message SQLite gives a full disk (SQLITE_FULL).
const DISK_FULL: &str = "database or disk is full";

/// How many events wait to be logged at most; older ones go first when more come.
const MAX_PENDING_EVENTS: usize = 10_000;

/// SQLite's primary result codes for a write the database could not make now but might later: BUSY, LOCKED, NOMEM,
/// READONLY, IOERR, FULL, CANTOPEN.
const WRITE_CODES: [i64; 7] = [5, 6, 7, 8, 10, 13, 14];

/// Whether `e` is a write the database could not make (a full disk, an I/O error, a lock held too long, a connection
/// left unusable), rather than a fault of the query or its data.
pub(crate) fn is_write_error(e: &sqlx::Error) -> bool {
    match e {
        sqlx::Error::Database(db) => {
            let code = db.code().and_then(|c| c.parse::<i64>().ok()).map(|c| c & 0xff);
            code.is_some_and(|c| WRITE_CODES.contains(&c)) || db.message().contains(DISK_FULL)
        }
        sqlx::Error::Io(_)
        | sqlx::Error::InvalidSavePointStatement
        | sqlx::Error::BeginFailed
        | sqlx::Error::WorkerCrashed
        | sqlx::Error::PoolTimedOut => true,
        _ => false,
    }
}

/// Whether an error's text (a step's, a run's) is a failed write: it says [`WRITE_FAILED`].
pub fn is_write_failure(msg: &str) -> bool {
    msg.contains(WRITE_FAILED)
}

/// Whether a failed write's message says the disk is full.
pub fn is_disk_full(msg: &str) -> bool {
    msg.contains(DISK_FULL)
}

/// Writes failing, while they do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteFailure {
    /// When the first write failed (ms).
    pub since: i64,
    /// The latest failure's message.
    pub message: String,
    /// When to try writing again (ms): [`Store::write_retry_due`].
    pub retry_at: i64,
    /// How many times writing was tried again since.
    pub retries: u32,
}

/// How long to wait before trying to write again after `retries` tries: 2 s, doubling, at most 30 s.
pub fn write_backoff_ms(retries: u32) -> i64 {
    (2_000i64 << retries.min(4)).min(30_000)
}

#[derive(Default)]
pub(crate) struct WriteHealth {
    failure: Mutex<Option<WriteFailure>>,
    pending: Mutex<Vec<(NewLogEvent, i64)>>,
}

impl WriteHealth {
    fn note(&self, msg: &str, now: i64) {
        let mut f = self.failure.lock().unwrap_or_else(|e| e.into_inner());
        match f.as_mut() {
            Some(f) => f.message = msg.to_string(),
            None => {
                *f = Some(WriteFailure {
                    since: now,
                    message: msg.to_string(),
                    retry_at: now + write_backoff_ms(0),
                    retries: 0,
                })
            }
        }
    }
}

/// The pool's `after_release`: a connection comes back clean, or not at all. A transaction whose COMMIT or statement
/// failed with a full disk is rolled back by SQLite itself; the ROLLBACK sqlx sends after it then finds none and
/// fails, and sqlx still counts the transaction as open, so every later `BEGIN` on that connection failed ("attempted
/// to call begin_with at non-zero transaction depth"). The ping waits for that queued ROLLBACK; a connection still in
/// a transaction after it is closed, and the pool opens a fresh one.
pub(crate) async fn release_clean(conn: &mut sqlx::SqliteConnection) -> Result<bool, sqlx::Error> {
    conn.ping().await?;
    Ok(!conn.is_in_transaction())
}

impl Store {
    /// Records `e` if it is a failed write ([`StoreError::Write`]); returns it.
    pub fn note_error(&self, e: StoreError) -> StoreError {
        if let StoreError::Write(_) = &e {
            self.note_write_failure(&e.to_string());
        }
        e
    }

    /// `r`, with a failed write recorded ([`Store::note_error`]).
    pub(crate) fn noted<T>(&self, r: Result<T, StoreError>) -> Result<T, StoreError> {
        r.map_err(|e| self.note_error(e))
    }

    /// Records a failed write from its message, when it is one ([`is_write_failure`]): a step's or a caller's error
    /// text.
    pub fn note_write_failure(&self, msg: &str) {
        if is_write_failure(msg) {
            self.health.note(msg, now_ms());
        }
    }

    /// Writes failing, while they do: from the first failure until a write works again ([`Store::probe_writes`]).
    pub fn write_failure(&self) -> Option<WriteFailure> {
        self.health.failure.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Whether it is time to try writing again: always while writes work; while they fail, once the backoff since the
    /// last try is over, which then starts a longer one.
    pub fn write_retry_due(&self, now: i64) -> bool {
        let mut f = self.health.failure.lock().unwrap_or_else(|e| e.into_inner());
        match f.as_mut() {
            None => true,
            Some(f) if now >= f.retry_at => {
                f.retries += 1;
                f.retry_at = now + write_backoff_ms(f.retries);
                true
            }
            Some(_) => false,
        }
    }

    /// Writes a row to see whether writes work: true (and the failure is over) when it does, else the failure is
    /// recorded.
    pub async fn probe_writes(&self) -> bool {
        let now = now_ms();
        let r: Result<(), StoreError> = async {
            let mut tx = self.write_tx().await?;
            sqlx::query(
                "INSERT INTO write_probe (id, at) VALUES (1, ?) ON CONFLICT (id) DO UPDATE SET at = excluded.at",
            )
            .bind(now)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(())
        }
        .await;
        match r {
            Err(StoreError::Write(e)) => {
                self.health.note(&StoreError::Write(e).to_string(), now);
                false
            }
            // Any other error is not about writing: writes work.
            _ => {
                *self.health.failure.lock().unwrap_or_else(|e| e.into_inner()) = None;
                true
            }
        }
    }

    /// Keeps an event that could not be logged, to log once writes work ([`Store::log_pending_events`]).
    pub(crate) fn keep_pending_event(&self, e: &NewLogEvent, now: i64) {
        let mut p = self.health.pending.lock().unwrap_or_else(|e| e.into_inner());
        if p.iter().any(|(x, _)| x.kind == e.kind && x.key == e.key) {
            return;
        }
        if p.len() >= MAX_PENDING_EVENTS {
            p.remove(0);
        }
        p.push((e.clone(), now));
    }

    /// How many events wait to be logged.
    pub fn pending_event_count(&self) -> usize {
        self.health.pending.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Logs the events that could not be logged when they came, oldest first, with the time they came; stops at the
    /// first that still fails. How many were logged (or were duplicates).
    pub async fn log_pending_events(&self) -> Result<usize, StoreError> {
        let mut done = 0;
        loop {
            let Some((e, at)) = self.health.pending.lock().unwrap_or_else(|e| e.into_inner()).first().cloned() else {
                return Ok(done);
            };
            self.noted(crate::event_log::emit_now(self, &e, at).await)?;
            let mut p = self.health.pending.lock().unwrap_or_else(|e| e.into_inner());
            if p.first().is_some_and(|(x, _)| x.kind == e.kind && x.key == e.key) {
                p.remove(0);
            }
            done += 1;
        }
    }
}

fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

#[cfg(test)]
mod tests {
    use crate::Store;

    /// Makes every insert into `projects` fail as SQLite does when the disk fills: the statement fails and SQLite rolls
    /// the whole transaction back itself, so the ROLLBACK sqlx sends after finds none.
    async fn fail_project_inserts(store: &Store, on: bool) {
        let sql = if on {
            "CREATE TRIGGER test_full BEFORE INSERT ON projects BEGIN SELECT RAISE(ROLLBACK, 'database or disk is full'); END"
        } else {
            "DROP TRIGGER test_full"
        };
        sqlx::query(sql).execute(&store.pool).await.unwrap();
    }

    fn project(id: &str) -> pdt_core::project::Project {
        pdt_core::project::Project {
            id: id.into(),
            name: id.into(),
            repo_path: format!("/r/{id}"),
            base_branch: "main".into(),
            created_at: 1,
            key: id.to_uppercase(),
            forge: None,
            kind: Default::default(),
        }
    }

    #[tokio::test]
    async fn a_transaction_the_database_rolled_back_leaves_no_connection_in_one() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
        fail_project_inserts(&store, true).await;
        for i in 0..4 {
            assert!(store.insert_project(&project(&format!("f{i}"))).await.is_err());
        }
        fail_project_inserts(&store, false).await;
        // Every connection the pool hands out again starts clean.
        for i in 0..20 {
            store.insert_project(&project(&format!("p{i}"))).await.unwrap();
        }
        assert_eq!(store.list_projects().await.unwrap().len(), 20);
    }

    #[tokio::test]
    async fn a_full_disk_is_a_failed_write_known_until_a_write_works() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
        assert_eq!(store.write_failure(), None);
        fail_project_inserts(&store, true).await;
        let e = store.insert_project(&project("a")).await.unwrap_err();
        assert!(matches!(e, crate::StoreError::Write(_)), "{e:?}");
        let msg = e.to_string();
        assert!(super::is_write_failure(&msg) && super::is_disk_full(&msg), "{msg}");
        // A caller that only has the text records it.
        store.note_write_failure(&format!("could not start: {msg}"));
        let f = store.write_failure().unwrap();
        assert!(f.message.contains("disk is full"));
        // Not again until the backoff is over, then with a longer one.
        assert!(!store.write_retry_due(f.since));
        assert!(store.write_retry_due(f.retry_at));
        assert_eq!(store.write_failure().unwrap().retries, 1);
        assert!(!store.write_retry_due(f.retry_at + 1));
        // Another failure's text is not a failed write.
        store.note_write_failure("no such node");
        assert_eq!(store.write_failure().unwrap().retries, 1);
        fail_project_inserts(&store, false).await;
        assert!(store.probe_writes().await);
        assert_eq!(store.write_failure(), None);
        assert!(store.write_retry_due(0));
    }

    #[test]
    fn the_backoff_doubles_up_to_thirty_seconds() {
        let b: Vec<i64> = (0..7).map(super::write_backoff_ms).collect();
        assert_eq!(b, vec![2_000, 4_000, 8_000, 16_000, 30_000, 30_000, 30_000]);
    }

    #[tokio::test]
    async fn an_event_that_could_not_be_logged_is_logged_once_writes_work() {
        use pdt_core::workflow::api::NewLogEvent;
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::open(&tmp.path().join("pdt.db")).await.unwrap();
        sqlx::query(
            "CREATE TRIGGER test_full BEFORE INSERT ON events BEGIN SELECT RAISE(ROLLBACK, 'database or disk is full'); END",
        )
        .execute(&store.pool)
        .await
        .unwrap();
        let a = NewLogEvent::new("ticket.moved", "k1", "test", serde_json::json!({"n": 1}));
        let b = NewLogEvent::new("ticket.moved", "k2", "test", serde_json::json!({"n": 2}));
        assert!(store.emit_event(&a, 10).await.is_err());
        assert!(store.emit_event(&b, 20).await.is_err());
        // The same event again is kept once.
        assert!(store.emit_event(&a, 30).await.is_err());
        assert_eq!(store.pending_event_count(), 2);
        assert!(store.log_pending_events().await.is_err());
        assert_eq!(store.pending_event_count(), 2);
        assert!(store.write_failure().is_some());
        sqlx::query("DROP TRIGGER test_full").execute(&store.pool).await.unwrap();
        assert_eq!(store.log_pending_events().await.unwrap(), 2);
        assert_eq!(store.pending_event_count(), 0);
        let logged = store.unprocessed_events(10).await.unwrap();
        let got: Vec<(&str, i64)> = logged.iter().map(|e| (e.key.as_str(), e.created_at)).collect();
        assert_eq!(got, vec![("k1", 10), ("k2", 20)]);
    }
}
