#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("conflict: {0}")]
    Conflict(String),
    #[error(transparent)]
    Db(sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl From<sqlx::Error> for StoreError {
    fn from(e: sqlx::Error) -> Self {
        if let sqlx::Error::Database(db) = &e
            && db.is_unique_violation()
        {
            return StoreError::Conflict(db.message().to_string());
        }
        StoreError::Db(e)
    }
}
