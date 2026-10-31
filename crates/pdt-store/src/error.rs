#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("conflict: {0}")]
    Conflict(String),
    #[error(transparent)]
    Db(sqlx::Error),
    /// A write the database could not make now (a full disk, an I/O error, a lock held too long): it may work later.
    #[error("PDT can't save changes: {0}")]
    Write(sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("corrupt row: {0}")]
    Corrupt(String),
}

impl From<sqlx::Error> for StoreError {
    fn from(e: sqlx::Error) -> Self {
        if let sqlx::Error::Database(db) = &e
            && db.is_unique_violation()
        {
            return StoreError::Conflict(db.message().to_string());
        }
        if crate::health::is_write_error(&e) {
            return StoreError::Write(e);
        }
        StoreError::Db(e)
    }
}
