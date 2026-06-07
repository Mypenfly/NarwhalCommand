//!@Open: ./tests/data/rust_complex.rs

// 多操作复合重构场景

// 操作 1：行号 Delete 删除 impl Default for AppConfig（原文件行 22-34）
//!@Location:
//!@Delete:@22,34
//!@Off:Location

// 操作 2：内容匹配 Location 在 AppConfig struct 中添加新字段
//!@Location:
pub struct AppConfig {
    pub name: String,
    pub version: String,
    pub features: Vec<String>,
    pub settings: HashMap<String, String>,
    pub data_dir: PathBuf,
    pub max_connections: u32,
    pub timeout_ms: u64,
//!@New:
    pub env_prefix: String,
    pub health_check_path: String,
//!@Off:Location

// 操作 3：嵌套 Location 在 impl AppConfig 的 validate 方法开头插入 log
//!@Location:
impl AppConfig {
//!@Location:
    pub fn validate(&self) -> Result<(), String> {
//!@New:Start
        log::debug!("validating config: {}", self.name);
//!@Off:Location
//!@Off:Location

// 操作 4：内容匹配定位 impl AppConfig，在 validate 之后添加 with_env_prefix 方法
//!@Location:
impl AppConfig {
//!@Location:
    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() {
            return Err("name is empty".into());
        }
        if self.max_connections == 0 {
            return Err("max_connections is zero".into());
        }
        Ok(())
    }
//!@New:
    pub fn with_env_prefix(mut self, prefix: &str) -> Self {
        self.env_prefix = prefix.to_string();
        self
    }
//!@Off:Location
//!@Off:Location

// 操作 5：内容匹配定位 impl ConnectionPool，在 new 方法后添加 shutdown 方法
//!@Location:
impl ConnectionPool {
//!@Location:
    pub fn new(config: AppConfig) -> Self {
        ConnectionPool {
            config,
            connections: Vec::new(),
            active: 0,
        }
    }
//!@New:
    pub fn shutdown(&mut self) {
        self.connections.clear();
        self.active = 0;
    }
//!@Off:Location
//!@Off:Location

// 操作 6：内容匹配定位 run_app，在函数开头添加日志
//!@Location:
pub fn run_app(config: AppConfig) -> Result<(), String> {
//!@New:
    log::info!("application starting with config: {}", config.name);
//!@Off:Location

// 操作 7：内容匹配定位测试模块，在末尾添加新测试
//!@Location:
#[cfg(test)]
mod tests {
//!@New:End

    #[test]
    fn test_connection_pool_shutdown() {
        let config = AppConfig::default();
        let mut pool = ConnectionPool::new(config);
        assert_eq!(pool.active, 0);
        pool.shutdown();
        assert!(pool.connections.is_empty());
    }
//!@Off:Location

//!@Off:Open
