//!@Open: ./tests/data/rust_complex.rs

// 场景 1：单行号 Location + New 在指定行后插入
//!@Location:@14
//!@New:
    pub region: String,
//!@Off:Location

// 场景 2：内容匹配定位 impl ConnectionPool 中的 new 方法，在其后插入 pool_size 字段
//!@Location:
    pub fn new(config: AppConfig) -> Self {
        ConnectionPool {
            config,
            connections: Vec::new(),
            active: 0,
        }
    }
//!@New:
    pub fn get_pool_size(&self) -> usize {
        self.connections.len()
    }
//!@Off:Location

// 场景 3：内容匹配 + 嵌套行号，在 impl AppConfig 的 validate 前添加 with_timeout
//!@Location:
impl AppConfig {
//!@Location:
    pub fn validate(&self) -> Result<(), String> {
//!@New:Start
    pub fn with_timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

//!@Off:Location
//!@Off:Location

//!@Off:Open
