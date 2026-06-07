// rust_complex.rs — 复杂工程测试文件
//
// 包含 struct 定义、impl 块（含嵌套方法体）、trait 定义与实现、
// 枚举变体、match 表达式、cfg(test) 模块等真实工程结构。
// 用于验证多层嵌套 Location、Delete/New 跨层修改、缩进保持。

use std::collections::HashMap;
use std::path::PathBuf;

/// 应用程序配置
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub name: String,
    pub version: String,
    pub features: Vec<String>,
    pub settings: HashMap<String, String>,
    pub data_dir: PathBuf,
    pub max_connections: u32,
    pub timeout_ms: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            name: "myapp".into(),
            version: "0.1.0".into(),
            features: vec![],
            settings: HashMap::new(),
            data_dir: PathBuf::from("./data"),
            max_connections: 10,
            timeout_ms: 5000,
        }
    }
}

impl AppConfig {
    pub fn from_env() -> Self {
        let mut config = Self::default();
        config
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() {
            return Err("name is empty".into());
        }
        if self.max_connections == 0 {
            return Err("max_connections is zero".into());
        }
        Ok(())
    }
}

/// 数据库连接池
pub struct ConnectionPool {
    config: AppConfig,
    connections: Vec<Connection>,
    active: usize,
}

struct Connection {
    id: u64,
    created_at: std::time::Instant,
    last_used: std::time::Instant,
}

impl ConnectionPool {
    pub fn new(config: AppConfig) -> Self {
        ConnectionPool {
            config,
            connections: Vec::new(),
            active: 0,
        }
    }

    pub fn get_connection(&mut self) -> Option<&mut Connection> {
        if self.active < self.config.max_connections as usize {
            let conn = Connection {
                id: self.active as u64,
                created_at: std::time::Instant::now(),
                last_used: std::time::Instant::now(),
            };
            self.connections.push(conn);
            self.active += 1;
            self.connections.last_mut()
        } else {
            None
        }
    }

    pub fn release_connection(&mut self, id: u64) {
        if let Some(conn) = self.connections.iter_mut().find(|c| c.id == id) {
            conn.last_used = std::time::Instant::now();
        }
        self.active = self.active.saturating_sub(1);
    }
}

/// 数据处理管线
pub struct DataPipeline {
    stages: Vec<Box<dyn ProcessStage>>,
    config: AppConfig,
}

pub trait ProcessStage {
    fn process(&self, input: &str) -> Result<String, String>;
    fn name(&self) -> &str;
}

struct ValidationStage {
    min_length: usize,
    max_length: usize,
}

impl ProcessStage for ValidationStage {
    fn process(&self, input: &str) -> Result<String, String> {
        if input.len() < self.min_length {
            return Err(format!("input too short: {} < {}", input.len(), self.min_length));
        }
        if input.len() > self.max_length {
            return Err(format!("input too long: {} > {}", input.len(), self.max_length));
        }
        Ok(input.to_string())
    }

    fn name(&self) -> &str {
        "validation"
    }
}

struct TransformStage {
    transform_type: TransformType,
}

enum TransformType {
    Uppercase,
    Lowercase,
    Trim,
    Reverse,
}

impl ProcessStage for TransformStage {
    fn process(&self, input: &str) -> Result<String, String> {
        let result = match self.transform_type {
            TransformType::Uppercase => input.to_uppercase(),
            TransformType::Lowercase => input.to_lowercase(),
            TransformType::Trim => input.trim().to_string(),
            TransformType::Reverse => input.chars().rev().collect(),
        };
        Ok(result)
    }

    fn name(&self) -> &str {
        "transform"
    }
}

struct EnrichmentStage {
    prefix: String,
    suffix: String,
    timestamp: bool,
}

impl ProcessStage for EnrichmentStage {
    fn process(&self, input: &str) -> Result<String, String> {
        let mut result = String::new();
        if !self.prefix.is_empty() {
            result.push_str(&self.prefix);
        }
        result.push_str(input);
        if !self.suffix.is_empty() {
            result.push_str(&self.suffix);
        }
        if self.timestamp {
            use std::time::SystemTime;
            let ts = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            result.push_str(&format!(" [ts:{}]", ts));
        }
        Ok(result)
    }

    fn name(&self) -> &str {
        "enrichment"
    }
}

impl DataPipeline {
    pub fn new(config: AppConfig) -> Self {
        DataPipeline {
            stages: Vec::new(),
            config,
        }
    }

    pub fn add_stage(&mut self, stage: Box<dyn ProcessStage>) {
        self.stages.push(stage);
    }

    pub fn execute(&self, input: &str) -> Result<String, String> {
        let mut current = input.to_string();
        for stage in &self.stages {
            match stage.process(&current) {
                Ok(result) => {
                    current = result;
                }
                Err(e) => {
                    return Err(format!("stage '{}' failed: {}", stage.name(), e));
                }
            }
        }
        Ok(current)
    }

    pub fn get_metrics(&self) -> PipelineMetrics {
        PipelineMetrics {
            stage_count: self.stages.len(),
            max_connections: self.config.max_connections as usize,
        }
    }
}

pub struct PipelineMetrics {
    pub stage_count: usize,
    pub max_connections: usize,
}

/// 应用入口
pub fn run_app(config: AppConfig) -> Result<(), String> {
    let mut pool = ConnectionPool::new(config.clone());

    let mut pipeline = DataPipeline::new(config);
    pipeline.add_stage(Box::new(ValidationStage {
        min_length: 1,
        max_length: 1024,
    }));
    pipeline.add_stage(Box::new(TransformStage {
        transform_type: TransformType::Trim,
    }));

    let result = pipeline.execute("  test input  ")?;

    if result.is_empty() {
        return Err("empty result".into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = AppConfig::default();
        assert_eq!(config.name, "myapp");
    }

    #[test]
    fn test_pipeline_basic() {
        let config = AppConfig::default();
        let mut pipeline = DataPipeline::new(config);
        pipeline.add_stage(Box::new(TransformStage {
            transform_type: TransformType::Trim,
        }));
        let result = pipeline.execute("  hello  ").unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_enrichment_with_timestamp() {
        let stage = EnrichmentStage {
            prefix: "[START]".into(),
            suffix: "[END]".into(),
            timestamp: true,
        };
        let result = stage.process("data").unwrap();
        assert!(result.starts_with("[START]"));
        assert!(result.ends_with("[END]"));
    }
}
