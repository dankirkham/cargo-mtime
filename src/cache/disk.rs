use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use async_trait::async_trait;
use color_eyre::eyre::{Context, Report, Result};
use speedy::Readable;
use speedy::Writable;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::debug;

use crate::cache::Cache;
use crate::metrics::Metrics;
use crate::config::DiskConfig;

#[derive(Debug)]
pub struct DatabaseQuery {
    /// Path to the file
    pub path: String,
    /// SHA256 hash of the file
    pub sha256: String,
    pub response: oneshot::Sender<Option<i64>>,
}

type Database = BTreeMap<String, BTreeMap<String, i64>>;

type DB = Arc<RwLock<AppState>>;

#[derive(speedy::Readable, speedy::Writable, Debug, Default)]
pub struct AppState {
    version: Option<u32>,
    mtime: Database,
}

async fn database_lookup_task(
    mut query_rx: UnboundedReceiver<DatabaseQuery>,
    metrics: Arc<Metrics>,
    read_connection: DB,
) -> Result<()> {
    while let Some(DatabaseQuery {
        path,
        sha256,
        response,
    }) = query_rx.recv().await
    {
        let path: String = path;
        let sha256: String = sha256;
        let mtime = try_get_mtime(&read_connection, &path, &sha256, metrics.as_ref()).await;
        response.send(mtime).unwrap();
    }

    debug!("database_lookup_task done.");

    Ok(())
}

async fn try_get_mtime(conn: &DB, path: &str, sha256: &str, metrics: &Metrics) -> Option<i64> {
    metrics
        .database_mtime_queries
        .fetch_add(1, Ordering::Relaxed);

    let r = conn.read().await;

    let res = r.mtime.get(path).and_then(|x| x.get(sha256));

    if res.is_some() {
        debug!("Cache hit for {}", path);
        metrics.database_mtime_hits.fetch_add(1, Ordering::Relaxed);
    } else {
        debug!("Cache miss for {}", path);
        metrics
            .database_mtime_misses
            .fetch_add(1, Ordering::Relaxed);
    }

    res.copied()
}

async fn update_database(
    connection: &DB,
    batch: &mut Vec<(String, String, i64)>,
    metrics: &Metrics,
) {
    let mut conn = connection.write().await;

    for (path, sha256, mtime) in batch.drain(..) {
        debug!("Write cache for {}", path);
        let mtime_map = conn.mtime.entry(path).or_insert_with(BTreeMap::new);
        mtime_map.insert(sha256, mtime);
    }

    metrics
        .database_mtime_write
        .fetch_add(batch.len(), Ordering::Relaxed);
}

async fn gather_submit_db(
    mut rx: UnboundedReceiver<(String, String, i64)>,
    connection: DB,
    metrics: Arc<Metrics>,
) {
    let mut current_batch: Vec<(String, String, i64)> = vec![];

    while let Some((path, sha256, mtime)) = rx.recv().await {
        current_batch.push((path, sha256, mtime));

        if current_batch.len() >= 100 {
            update_database(&connection, &mut current_batch, metrics.as_ref()).await;
        }
    }
    debug!("gather_submit_db finalizing...");

    if !current_batch.is_empty() {
        debug!(
            "gather_submit_db has {} entries left to write.",
            current_batch.len()
        );
        update_database(&connection, &mut current_batch, metrics.as_ref()).await
    }
    debug!("gather_submit_db finalized");
}

#[derive(Debug)]
pub struct DiskCache {
    db_path: String,
    db: DB,
    write_tx: UnboundedSender<(String, String, i64)>,
    write_handle: JoinHandle<()>,
    read_tx: UnboundedSender<DatabaseQuery>,
    read_handle: JoinHandle<Result<(), Report>>,
}

impl DiskCache {
    pub fn init(config: &DiskConfig, metrics: Arc<Metrics>) -> Result<Self> {
        // Create the database if it doesn't exist
        let db_path = config.db_path.clone();
        let file = std::fs::read(&db_path).unwrap_or_default();
        let mut conn = if file.is_empty() {
            Ok(AppState::default())
        } else {
            AppState::read_from_buffer(&file).wrap_err("Failed to read database")
        }?;

        let version = conn.version;

        match version {
            None => {
                conn.version = Some(1);
            }
            Some(version) => {
                if version != 1 {
                    panic!("Unsupported database version: {:?}", version);
                }
            }
        }

        let db = Arc::new(RwLock::new(conn));

        let (write_tx, write_rx) = unbounded_channel();
        let write_handle = tokio::spawn(gather_submit_db(write_rx, db.clone(), metrics.clone()));

        let (read_tx, read_rx) = unbounded_channel::<DatabaseQuery>();
        let read_handle = tokio::spawn(database_lookup_task(read_rx, metrics.clone(), db.clone()));

        Ok(Self {
            db_path,
            db,
            write_tx,
            write_handle,
            read_tx,
            read_handle,
        })
    }
}

#[async_trait]
impl Cache for DiskCache {
    async fn finalize(self: Box<Self>) {
        // TODO: Use std::future::async_drop when it is stable.
        let Self {
            db_path,
            db,
            write_tx,
            write_handle,
            read_tx,
            read_handle,
        } = *self;

        drop(read_tx);
        drop(write_tx);

        write_handle.await.unwrap();
        read_handle.await.unwrap().unwrap();

        let conn = db.read().await;
        let buffer = conn.write_to_vec().unwrap();

        let path = std::path::PathBuf::from(&db_path);

        let prefix = path.parent();
        if let Some(prefix) = prefix {
            std::fs::create_dir_all(prefix).unwrap();
        }
        std::fs::write(&db_path, buffer).unwrap();
    }

    async fn get(&self, path: String, sha256: String) -> Option<i64> {
        let (response_tx, response_rx) = oneshot::channel();
        let query = DatabaseQuery {
            path,
            sha256,
            response: response_tx,
        };
        self.read_tx.send(query).unwrap();

        response_rx.await.unwrap()
    }

    async fn insert(&self, path: String, sha256: String, mtime: i64) {
        self.write_tx.send((path, sha256, mtime)).unwrap()
    }
}
