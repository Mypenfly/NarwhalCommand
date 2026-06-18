// Application configuration module.
//
// Loads configuration from environment variables and config files.

use std::env;
use std::path::PathBuf;

/// Top-level application configuration.
#[derive(Debug, Clone)]
pub struct AppConfig {
    /// Database connection string
    pub database_url: String,
    /// Maximum number of database connections in the pool
    pub db_pool_size: u32,
    /// Server listen address (e.g., "0.0.0.0:8080")
    pub listen_address: String,
    /// Number of worker threads for the async runtime
    pub worker_threads: usize,
    /// Minimum password length for user accounts
/// Log level filter (trace, debug, info, warn, error)
pub log_level: String,
    pub min_password_length: u32,
    /// Number of bcrypt salt rounds
    pub password_salt_rounds: u32,
    /// Request timeout in seconds
    pub request_timeout_secs: u64,
    /// Path to the static assets directory
    pub assets_dir: PathBuf,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            database_url: "postgres://localhost:5432/myapp".to_string(),
            db_pool_size: 10,
            listen_address: "0.0.0.0:3000".to_string(),
            worker_threads: 4,
            min_password_length: 8,
            password_salt_rounds: 12,
            request_timeout_secs: 30,
            assets_dir: PathBuf::from("./static"),
        }
    }
}

impl AppConfig {
    /// Load configuration from environment variables, falling back to defaults.
    pub fn from_env() -> Self {
        let mut config = AppConfig::default();

        if let Ok(url) = env::var("DATABASE_URL") {
            config.database_url = url;
        }
        if let Ok(size) = env::var("DB_POOL_SIZE") {
            if let Ok(n) = size.parse() {
                config.db_pool_size = n;
            }
        }
        if let Ok(addr) = env::var("LISTEN_ADDRESS") {
            config.listen_address = addr;
        }
        if let Ok(threads) = env::var("WORKER_THREADS") {
            if let Ok(n) = threads.parse() {
                config.worker_threads = n;
            }
        }
        if let Ok(len) = env::var("MIN_PASSWORD_LENGTH") {
            if let Ok(n) = len.parse() {
                config.min_password_length = n;
            }
        }
        if let Ok(rounds) = env::var("PASSWORD_SALT_ROUNDS") {
            if let Ok(n) = rounds.parse() {
                config.password_salt_rounds = n;
            }
        }
        if let Ok(timeout) = env::var("REQUEST_TIMEOUT_SECS") {
            if let Ok(n) = timeout.parse() {
                config.request_timeout_secs = n;
            }
        }
        if let Ok(dir) = env::var("ASSETS_DIR") {
            config.assets_dir = PathBuf::from(dir);
        }

        config
    }

    /// Build a database connection URL from individual components.
    pub fn build_database_url(
        host: &str,
        port: u16,
        db_name: &str,
    ) -> String {
        format!("postgres://{}:{}/{}", host, port, db_name)
    }
}
