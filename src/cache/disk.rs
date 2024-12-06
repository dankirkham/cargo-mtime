use std::collections::BTreeMap;

use speedy::Readable;
use speedy::Writable;

use crate::cache::DatabaseQuery;

type Database = BTreeMap<String, BTreeMap<String, i64>>;

#[derive(speedy::Readable, speedy::Writable, Debug, Default)]
pub struct DiskCache {
    version: Option<u32>,
    mtime: Database,
}

type DB = Arc<RwLock<DiskCache>>;

pub trait Cache {
    async fn get(query: DatabaseQuery) -> Option<i64>;

    async fn insert(query: DatabaseQuery, mtime: impl AsRef<str>);
}
