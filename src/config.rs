#[derive(Debug, Clone)]
pub struct Config {
    /// Directory to process mtimes of.
    pub root_dir: String,

    /// Cache config
    pub cache_config: CacheConfig,
}

#[derive(Debug, Clone)]
pub enum CacheType {
    Disk,
    Redis,
}

#[derive(Debug, Clone)]
pub enum CacheConfig {
    Disk(DiskConfig),
    Redis(RedisConfig),
}

#[derive(Debug, Clone)]
pub struct DiskConfig {
    /// Path to database
    pub db_path: String,
}

#[derive(Debug, Clone)]
pub struct RedisConfig {
    /// URL to Redis
    pub url: String,
}

impl Config {
    pub fn from_env() -> Self {
        let args = std::env::args().collect::<Vec<_>>();
        let root_dir = args
            .get(1)
            .cloned()
            .unwrap_or_else(|| std::env::var("CARGO_MTIME_ROOT").unwrap())
            .to_string();

        let cache_type = match std::env::var("CARGO_MTIME_CACHE_TYPE") {
            Ok(t) => match t.as_str() {
                "redis" => CacheType::Redis,
                "disk" => CacheType::Disk,
                _ => CacheType::Disk,
            },
            Err(_) => CacheType::Disk,
        };

        let cache_config = match cache_type {
            CacheType::Disk => {
                let db_path = args
                    .get(2)
                    .cloned()
                    .unwrap_or_else(|| std::env::var("CARGO_MTIME_DB_PATH").unwrap())
                    .to_string();

                let config = DiskConfig { db_path };

                CacheConfig::Disk(config)
            }
            CacheType::Redis => {
                let url = std::env::var("CARGO_MTIME_REDIS_URL")
                    .unwrap_or("redis://127.0.0.1/".to_string());

                let config = RedisConfig { url };

                CacheConfig::Redis(config)
            }
        };

        Self {
            root_dir,
            cache_config,
        }
    }
}
