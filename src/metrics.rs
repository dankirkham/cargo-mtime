use std::sync::atomic::AtomicUsize;

#[derive(Default, Debug)]
pub struct Metrics {
    pub files_processed: AtomicUsize,
    pub files_was_directory: AtomicUsize,
    pub files_mtime_restored: AtomicUsize,
    pub files_mtime_skipped: AtomicUsize,
    pub files_mtime_updated: AtomicUsize,

    pub database_mtime_queries: AtomicUsize,
    pub database_mtime_hits: AtomicUsize,
    pub database_mtime_misses: AtomicUsize,

    pub database_mtime_write: AtomicUsize,
}

