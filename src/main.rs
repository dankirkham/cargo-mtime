//! Small helper binary using a file-db to maintain an mtime cache
//! to avoid unnecessary rebuilds when using Cargo from a sandbox.
//! This is a workaround for the lack of a `mtime` cache in Cargo.
//!
//! Given the environment path `CARGO_MTIME_ROOT_DIR` and
//! `CARGO_MTIME_DB_PATH` this utility will set the mtime of every
//! file in the root directory to the mtime stored in the database,
//! iff the sha256 of the file matches the sha256 stored in the
//! database.

mod cache;
mod config;
mod metrics;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use async_walkdir::DirEntry;
use async_walkdir::Filtering;
use async_walkdir::WalkDir;
use filetime::FileTime;
use futures::StreamExt;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::info;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;

use crate::cache::{Cache, DiskCache, RedisCache};
use crate::config::{CacheConfig, Config};
use crate::metrics::Metrics;

#[tokio::main(flavor = "multi_thread", worker_threads = 10)]
async fn main() {
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_panic(info);
        std::process::exit(1);
    }));

    tracing_subscriber::fmt()
        .with_span_events(FmtSpan::CLOSE)
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let metrics = Arc::new(Metrics::default());

    let config = Config::from_env();
    info!("{:?}", &config);
    let cache: Box<dyn Cache> = match config.cache_config {
        CacheConfig::Disk(config) => Box::new(DiskCache::init(&config, metrics.clone()).unwrap()),
        CacheConfig::Redis(config) => Box::new(RedisCache::init(&config, metrics.clone()).unwrap()),
    };
    let cache = Arc::new(cache);

    // We'll use a semaphore to limit the number of concurrent file operations, to avoid running out of file descriptors.
    let semaphore = Arc::new(Semaphore::new(config.max_open_files));
    let mut entries = WalkDir::new(&config.root_dir).filter(|entry| async move {
        if let Some(true) = entry
            .path()
            .file_name()
            .map(|f| f.to_string_lossy().starts_with('.'))
        {
            return Filtering::IgnoreDir;
        }

        // any directory containing .rustc_info.json is a rustc build directory
        if entry.path().join(".rustc_info.json").exists() {
            return Filtering::IgnoreDir;
        }

        Filtering::Continue
    });

    let mut join_set = JoinSet::new();
    loop {
        match entries.next().await {
            Some(Ok(entry)) => {
                metrics.files_processed.fetch_add(1, Ordering::Relaxed);

                let is_file = entry.file_type().await.map_or(false, |t| t.is_file());

                if !is_file {
                    metrics.files_was_directory.fetch_add(1, Ordering::Relaxed);
                    continue;
                }

                let semaphore = semaphore.clone();

                join_set.spawn(manage_mtime(
                    entry,
                    semaphore,
                    metrics.clone(),
                    cache.clone(),
                ));
            }
            Some(Err(e)) => {
                eprintln!("error: {}", e);
                break;
            }
            None => break,
        }
    }

    while join_set.join_next().await.is_some() {}

    let cache = Arc::<Box<dyn Cache>>::into_inner(cache).unwrap();
    cache.finalize().await;

    info!("{:?}", metrics);
}

/// Attempts to set the mtime of the file on disk such that:
///
/// * If we have no record, we store current file mtime, sha256, and path to the database
/// * If the sha256 is different, we *record* the mtime and sha256 of the file but do not touch it
/// * If the sha256 is the same, we set the mtime of the file to the recorded previous time
#[tracing::instrument(skip_all)]
async fn manage_mtime(
    entry: DirEntry,
    permit: Arc<Semaphore>,
    metrics: Arc<Metrics>,
    cache: Arc<Box<dyn Cache>>,
) {
    let permit = permit.acquire().await.unwrap();
    let path = entry.path();

    let (sha256, metadata) = tokio::join!(sha256::try_async_digest(&path), entry.metadata(),);
    drop(entry);
    let sha256 = sha256.unwrap();

    let mtime_on_disk =
        FileTime::from_system_time(metadata.unwrap().modified().unwrap()).unix_seconds();

    let string_path = path.to_string_lossy().to_string();

    if let Some(mtime) = cache.get(string_path.clone(), sha256.clone()).await {
        if mtime != mtime_on_disk {
            metrics.files_mtime_restored.fetch_add(1, Ordering::Relaxed);
            filetime::set_file_mtime(&path, FileTime::from_unix_time(mtime, 0)).unwrap();
            drop(permit);
        } else {
            metrics.files_mtime_skipped.fetch_add(1, Ordering::Relaxed);
        }
    } else {
        drop(permit);
        metrics.files_mtime_updated.fetch_add(1, Ordering::Relaxed);
        cache.insert(string_path, sha256, mtime_on_disk).await;
    }
}
