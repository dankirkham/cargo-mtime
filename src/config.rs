use envconfig::Envconfig;

#[derive(Debug, Clone, Envconfig)]
pub struct Config {
    /// Directory to process mtimes of.
    #[envconfig(from = "CARGO_MTIME_ROOT")]
    pub root_dir: String,
    /// Path to database
    #[envconfig(from = "CARGO_MTIME_DB_PATH")]
    pub db_path: String,
}
