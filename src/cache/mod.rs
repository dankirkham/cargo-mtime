mod disk;
mod redis;

use async_trait::async_trait;

#[async_trait]
pub trait Cache: Send + Sync {
    /// Finalize cache writing
    ///
    /// This should clean up all workers and tasks and then finish writing to the database.
    async fn finalize(self: Box<Self>);

    /// Query an mtime entry from the database.
    ///
    /// Metrics
    ///
    /// Implementor shall record:
    /// - Query count
    /// - Hit count
    /// - Miss count
    async fn get(&self, path: String, sha256: String) -> Option<i64>;

    /// Insert an mtime record into the database because it was not cached.
    ///
    /// Metrics
    ///
    /// Implementor shall record:
    /// - Write count
    async fn insert(&self, path: String, sha256: String, mtime: i64);
}

pub use disk::DiskCache;
pub use redis::RedisCache;
