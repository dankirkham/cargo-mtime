use std::fmt::Debug;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use async_trait::async_trait;
use color_eyre::eyre::Result;
use redis::aio::MultiplexedConnection;
use redis::AsyncCommands;
use redis::Client;
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::debug;

use crate::cache::Cache;
use crate::config::RedisConfig;
use crate::metrics::Metrics;

struct ReadRequest {
    path: String,
    sha256: String,
    response: oneshot::Sender<Option<i64>>,
}

async fn read_task(
    client: Client,
    mut read_rx: UnboundedReceiver<ReadRequest>,
    metrics: Arc<Metrics>,
) {
    let mut conn = client.get_multiplexed_tokio_connection().await.unwrap();

    while let Some(ReadRequest {
        mut path,
        sha256,
        response,
    }) = read_rx.recv().await
    {
        metrics
            .database_mtime_queries
            .fetch_add(1, Ordering::Relaxed);

        // Concatenate path and sha256
        path.push_str(&sha256);

        let res: Option<i64> = conn.get(&path).await.unwrap();

        if res.is_some() {
            debug!("Cache hit for {}", path);
            metrics.database_mtime_hits.fetch_add(1, Ordering::Relaxed);
        } else {
            debug!("Cache miss for {}", path);
            metrics
                .database_mtime_misses
                .fetch_add(1, Ordering::Relaxed);
        }

        response.send(res).unwrap();
    }

    debug!("read task finished");
}

struct WriteRequest {
    path: String,
    sha256: String,
    mtime: i64,
}

async fn write_operation(
    conn: &mut MultiplexedConnection,
    batch: &mut [WriteRequest],
    metrics: &Metrics,
) {
    let ops: Vec<_> = batch
        .iter()
        .map(|req| {
            let WriteRequest {
                path,
                sha256,
                mtime,
            } = req;
            let mut key = path.clone();
            key.push_str(sha256);
            (key, *mtime)
        })
        .collect();

    let _: () = conn.mset(&ops).await.unwrap();

    metrics
        .database_mtime_write
        .fetch_add(batch.len(), Ordering::Relaxed);
}

async fn write_task(
    client: Client,
    mut write_rx: UnboundedReceiver<WriteRequest>,
    metrics: Arc<Metrics>,
) {
    let mut current_batch: Vec<WriteRequest> = vec![];
    let mut conn = client.get_multiplexed_tokio_connection().await.unwrap();

    while let Some(request) = write_rx.recv().await {
        current_batch.push(request);

        if current_batch.len() >= 100 {
            write_operation(&mut conn, &mut current_batch, metrics.as_ref()).await;
        }
    }
    debug!("write_task finalizing...");

    if !current_batch.is_empty() {
        debug!(
            "write_task has {} entries left to write.",
            current_batch.len()
        );
        write_operation(&mut conn, &mut current_batch, metrics.as_ref()).await
    }
    debug!("write_task finalized");
}

pub struct RedisCache {
    client: Client,
    metrics: Arc<Metrics>,
    read_tx: UnboundedSender<ReadRequest>,
    read_handle: JoinHandle<()>,
    write_tx: UnboundedSender<WriteRequest>,
    write_handle: JoinHandle<()>,
}

// Debug is required for eyre pretty printing, but [Connection] does not impl Debug.
impl Debug for RedisCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("client", &self.client)
            .finish()
    }
}

impl RedisCache {
    pub fn init(config: &RedisConfig, metrics: Arc<Metrics>) -> Result<Self> {
        let client = Client::open(config.url.clone())?;

        let (read_tx, read_rx) = unbounded_channel();
        let read_client = client.clone();
        let read_metrics = metrics.clone();
        let read_handle = tokio::task::spawn(async move {
            read_task(read_client, read_rx, read_metrics).await;
        });

        let (write_tx, write_rx) = unbounded_channel();
        let write_client = client.clone();
        let write_metrics = metrics.clone();
        let write_handle = tokio::task::spawn(async move {
            write_task(write_client, write_rx, write_metrics).await;
        });

        Ok(Self {
            client,
            metrics,
            read_tx,
            read_handle,
            write_tx,
            write_handle,
        })
    }
}

#[async_trait]
impl Cache for RedisCache {
    async fn finalize(self: Box<Self>) {
        let Self {
            client: _client,
            metrics: _metrics,
            read_tx,
            read_handle,
            write_tx,
            write_handle,
        } = *self;

        drop(read_tx);
        drop(write_tx);

        write_handle.await.unwrap();
        read_handle.await.unwrap();
    }

    async fn get(&self, path: String, sha256: String) -> Option<i64> {
        let (response_tx, response_rx) = oneshot::channel();
        let query = ReadRequest {
            path,
            sha256,
            response: response_tx,
        };
        self.read_tx.send(query).unwrap();

        response_rx.await.unwrap()
    }

    async fn insert(&self, path: String, sha256: String, mtime: i64) {
        let req = WriteRequest {
            path,
            sha256,
            mtime,
        };
        self.write_tx.send(req).unwrap()
    }
}
