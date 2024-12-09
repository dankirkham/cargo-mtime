use tokio::sync::oneshot::Sender;

mod disk;

#[derive(Debug)]
pub struct DatabaseQuery {
    /// Path to the file
    pub path: String,
    /// SHA256 hash of the file
    pub sha256: String,
    pub response: Sender<Option<i64>>
}

pub trait Cache: Send + Sync {
    async fn get(&self, path: String, sha256: String) -> Option<i64>;

    async fn insert(&self, path: String, sha256: String, mtime: i64);
}

pub use disk::DiskCache;
