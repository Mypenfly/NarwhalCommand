//!@Open: ./tests/data/rust_complex.rs

// 边界 1：空 Location + Delete + New（替换整个文件内的测试模块末尾）
//!@Location:
//!@Delete:
    #[test]
    fn test_config_default() {
        let config = AppConfig::default();
        assert_eq!(config.name, "myapp");
    }
//!@New:
    #[test]
    fn test_config_default() {
        let config = AppConfig::default();
        assert_eq!(config.name, "myapp");
    }

    #[test]
    fn test_config_validation() {
        let mut config = AppConfig::default();
        config.name = String::new();
        assert!(config.validate().is_err());
    }
//!@Off:Location

// 边界 2：嵌套 Location 然后在 validate 方法开头 New:Start
//!@Location:
impl AppConfig {
//!@Location:
    pub fn validate(&self) -> Result<(), String> {
//!@New:Start
        log::debug!("validating config: {}", self.name);
//!@Off:Location
//!@Off:Location

// 边界 3：深度嵌套验证缩进保持 — execute → match Ok 分支 → New
//!@Location:
impl DataPipeline {
//!@Location:
    pub fn execute(&self, input: &str) -> Result<String, String> {
//!@Location:
                Ok(result) => {
                    current = result;
//!@New:
                    log::trace!("stage '{}' produced: {} chars", stage.name(), result.len());
//!@Off:Location
//!@Off:Location
//!@Off:Location

// 边界 4：嵌套 Location 精确定位并替换 run_app 中的执行调用
//!@Location:
pub fn run_app(config: AppConfig) -> Result<(), String> {
//!@Location:
    let result = pipeline.execute("  test input  ")?;
//!@Delete:
    let result = pipeline.execute("  test input  ")?;
//!@New:
    // Process the sample input through pipeline
    let result = pipeline.execute("  test input  ")?;
    log::info!("pipeline result: {:?}", result);
//!@Off:Location
//!@Off:Location

//!@Off:Open
