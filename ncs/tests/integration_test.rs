//! NCS 集成测试
//!
//! 读取 tests/scripts/ 下的 .ncs 脚本和 tests/data/ 下的测试数据，
//! 端到端验证命令执行流水线。
//!
//! ## 实现状态
//!
//! Phase 2+ 逐步添加测试用例。
//!
//! ## 对应文档
//!
//! 详见 phases.md Phase 0 验证, INSTRUCTION.md §8 "测试策略"

#[cfg(test)]
mod tests {
    /// 辅助结构：持有临时目录，确保测试文件存活
    struct TestEnv {
        #[allow(dead_code)]
        dir: tempfile::TempDir,
    }

    impl TestEnv {
        #[allow(dead_code)]
        fn new() -> Self {
            TestEnv {
                dir: tempfile::tempdir().unwrap(),
            }
        }

        #[allow(dead_code)]
        fn create_file(&self, name: &str, content: &str) -> String {
            let path = self.dir.path().join(name);
            std::fs::write(&path, content).unwrap();
            path.to_str().unwrap().to_string()
        }
    }

    #[test]
    fn test_registry_init_works() {
        let registry = ncs::registry::CommandRegistry::init();
        assert_eq!(registry.entries.len(), 12);
    }

    #[test]
    fn test_cmd_content_basic() {
        let content = ncs::cmd_content::CmdContent::from_raw_text("hello".to_string());
        assert_eq!(content.lines.len(), 1);
        assert_eq!(content.raw_content, "hello");
    }

    #[test]
    fn test_error_types_exist() {
        let err = ncs::error::RegistryError::CommandNotFound {
            cmd_name: "Test".to_string(),
            line: ncs::model::LineNumber::new(1),
            suggestion: None,
        };
        assert!(err.title().contains("未注册"));
    }

    #[test]
    fn test_model_line_number_basics() {
        let ln = ncs::model::LineNumber::new(1);
        assert_eq!(ln.to_usize(), 1);
        assert_eq!(ln.to_index(), 0);
    }
}
