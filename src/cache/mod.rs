mod disk;

#[derive(Debug)]
pub struct DatabaseQuery {
    /// Path to the file
    pub path: String,
    /// SHA256 hash of the file
    pub sha256: String,
}

pub trait Cache {
    async fn get(query: DatabaseQuery) -> Option<i64>;

    async fn insert(query: DatabaseQuery, mtime: impl AsRef<str>);
}
