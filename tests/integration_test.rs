//! N_Edit 集成测试
//!
//! 端到端验证完整的 lexer → parser → engine 流程。
//! 使用 tests/data/ 下的真实 Rust 源码作为目标文件，
//! 使用 tests/scripts/ 下的 .ned 脚本执行编辑操作。
//!
//! 所有测试操作在临时文件副本上进行，不修改原始数据文件。

use std::path::Path;

/// 测试环境：持有临时目录和目标文件的副本
struct TestEnv {
    /// 临时目录（Drop 时自动清理）
    _dir: tempfile::TempDir,
    /// 目标文件的副本路径
    target_path: String,
}

impl TestEnv {
    /// 从 tests/data/ 复制目标文件到临时目录，返回测试环境
    fn from_data_file(data_file: &str) -> Self {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let src = Path::new("tests/data").join(data_file);
        let dst = dir.path().join(data_file);

        // 确保目标路径包含的父目录结构一致
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).expect("Failed to create parent dirs");
        }

        std::fs::copy(&src, &dst).expect(&format!("Failed to copy {}", src.display()));

        TestEnv {
            target_path: dst.to_str().unwrap().to_string(),
            _dir: dir,
        }
    }

    /// 读取 .ned 脚本内容，将其中的 Open 路径替换为临时副本路径
    fn load_script(&self, script_name: &str) -> String {
        let script_path = Path::new("tests/scripts").join(script_name);
        let script = std::fs::read_to_string(&script_path)
            .expect(&format!("Failed to read script {}", script_path.display()));

        self.replace_paths(&script)
    }

    /// 将脚本中所有 Open 命令的路径替换为临时目录中的路径
    fn replace_paths(&self, script: &str) -> String {
        // 策略：匹配 Open: ./tests/data/... 并替换为 Open: ${self.target_path}
        // 但 Open 路径可能指向不同文件（如 config.rs 或 services.rs）
        // 我们用更简单的方法：提取原路径中的文件名，映射到 temp 路径
        script
            .lines()
            .map(|line| {
                if line.starts_with("//!@Open: ") {
                    let original = line.strip_prefix("//!@Open: ").unwrap().trim();
                    let file_name = Path::new(original)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(original);
                    let temp_dir = Path::new(&self.target_path).parent().unwrap();
                    let resolved = temp_dir.join(file_name);
                    format!("//!@Open: {}", resolved.to_str().unwrap())
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 读取目标文件的当前内容
    fn read_target(&self) -> String {
        std::fs::read_to_string(&self.target_path)
            .expect(&format!("Failed to read target {}", self.target_path))
    }
}

/// 辅助：执行 .ned 脚本的完整流水线并返回引擎状态
fn execute_script(script_content: &str) -> (n_edit::engine::Engine, bool) {
    let tokens = n_edit::lexer::Lexer::tokenize(script_content);
    let commands = match n_edit::parser::Parser::parse(tokens) {
        Ok(cmds) => cmds,
        Err(e) => {
            eprintln!("Parse error: {}", e);
            let engine = n_edit::engine::Engine::new();
            return (engine, false);
        }
    };

    let mut engine = n_edit::engine::Engine::new();
    let success = engine.execute(commands).is_ok();
    (engine, success)
}

// ============================================================
// Phase 1: Open / Location / Off（回归测试）
// ============================================================

#[test]
fn test_open_location_off_readonly() {
    let env = TestEnv::from_data_file("sample.rs");
    let script = env.load_script("test_open_location_off.ned");

    let (engine, success) = execute_script(&script);
    assert!(success, "Script execution failed");

    // 原始脚本只做了 read-only 操作，文件内容应不变
    let original = std::fs::read_to_string("tests/data/sample.rs").unwrap();
    let result = env.read_target();
    assert_eq!(result, original, "Read-only Location should not modify file");

    // diff_lines 应为空（只读操作不产生 diff）
    assert!(engine.diff_lines.is_empty());
}

#[test]
fn test_open_location_off_location() {
    let env = TestEnv::from_data_file("sample.rs");
    let script = env.load_script("test_open_location_offlocation.ned");

    let (engine, success) = execute_script(&script);
    assert!(success, "Script execution failed");

    assert!(engine.diff_lines.is_empty());
}

#[test]
fn test_implicit_off_open() {
    let env = TestEnv::from_data_file("sample.rs");
    let script = env.load_script("test_implicit_off.ned");

    let (_, success) = execute_script(&script);
    assert!(success, "Implicit Off:Open should succeed");
}

#[test]
fn test_open_off_roundtrip() {
    let env = TestEnv::from_data_file("sample.rs");
    let script = env.load_script("test_open_off.ned");

    let (_, success) = execute_script(&script);
    assert!(success, "Open+Off should succeed");

    let original = std::fs::read_to_string("tests/data/sample.rs").unwrap();
    let result = env.read_target();
    assert_eq!(result, original);
}

// ============================================================
// Phase 2: New 命令集成测试
// ============================================================

#[test]
fn test_add_struct_field() {
    let env = TestEnv::from_data_file("config.rs");
    let script = env.load_script("add_struct_field.ned");

    let (engine, success) = execute_script(&script);
    assert!(success, "add_struct_field script failed");

    let result = env.read_target();

    // 验证新字段已插入
    assert!(
        result.contains("pub log_level: String"),
        "Expected new field 'log_level' in output:\n{}",
        result
    );

    // 验证原有内容未被破坏
    assert!(result.contains("pub database_url: String"));
    assert!(result.contains("pub min_password_length: u32"));
    assert!(result.contains("pub password_salt_rounds: u32"));

    // 验证 diff_lines 包含 Added 条目
    assert!(!engine.diff_lines.is_empty());
    let added_lines: Vec<_> = engine
        .diff_lines
        .iter()
        .filter(|d| d.kind == n_edit::output::DiffLineKind::Added)
        .collect();
    assert!(!added_lines.is_empty(), "Should have Added diff lines");
}

#[test]
fn test_add_method_to_impl() {
    let env = TestEnv::from_data_file("config.rs");
    let script = env.load_script("add_method.ned");

    let (engine, success) = execute_script(&script);
    assert!(success, "add_method script failed");

    let result = env.read_target();

    // 验证新方法已插入
    assert!(
        result.contains("pub fn reload(&mut self)"),
        "Expected new method 'reload' in output"
    );
    assert!(
        result.contains("let env_config = AppConfig::from_env();"),
        "Expected method body in output"
    );

    // 验证原有方法未被破坏
    assert!(result.contains("pub fn from_env()"));
    assert!(result.contains("pub fn build_database_url"));

    // 验证 diff 输出
    let added_count = engine
        .diff_lines
        .iter()
        .filter(|d| d.kind == n_edit::output::DiffLineKind::Added)
        .count();
    assert!(added_count > 0, "Should have Added diff lines, got {}", added_count);
}

#[test]
fn test_add_license_header_new_start() {
    let env = TestEnv::from_data_file("config.rs");
    let script = env.load_script("add_license_header.ned");

    let (engine, success) = execute_script(&script);
    assert!(success, "add_license_header script failed");

    let result = env.read_target();

    // 验证头部已插入
    assert!(
        result.contains("// Copyright 2024 Example Corp."),
        "Expected license header in output"
    );
    assert!(
        result.contains("// SPDX-License-Identifier:"),
        "Expected SPDX identifier in output"
    );

    // 验证原有首行仍然存在
    assert!(result.contains("// Application configuration module."));

    let added_count = engine
        .diff_lines
        .iter()
        .filter(|d| d.kind == n_edit::output::DiffLineKind::Added)
        .count();
    assert!(added_count > 0, "Should have Added diff lines");
}

#[test]
fn test_add_tests_at_end_new_end() {
    let env = TestEnv::from_data_file("config.rs");
    let script = env.load_script("add_tests_at_end.ned");

    let (engine, success) = execute_script(&script);
    assert!(success, "add_tests_at_end script failed");

    let result = env.read_target();

    // 验证测试模块已追加到末尾
    assert!(
        result.contains("#[cfg(test)]"),
        "Expected test module at end of file"
    );
    assert!(
        result.contains("fn test_default_config()"),
        "Expected test function in output"
    );
    assert!(
        result.contains("fn test_from_env_respects_defaults()"),
        "Expected second test function in output"
    );

    // 验证原有内容仍在前面
    assert!(result.contains("pub struct AppConfig"));
    assert!(result.contains("impl AppConfig"));

    let added_count = engine
        .diff_lines
        .iter()
        .filter(|d| d.kind == n_edit::output::DiffLineKind::Added)
        .count();
    assert!(added_count > 1, "Should have multiple Added diff lines");
}

// ============================================================
// Phase 2: Delete 命令集成测试
// ============================================================

#[test]
fn test_delete_function() {
    let env = TestEnv::from_data_file("services.rs");
    let script = env.load_script("delete_function.ned");

    let (engine, success) = execute_script(&script);
    assert!(success, "delete_function script failed");

    let result = env.read_target();

    // 验证函数已删除
    assert!(
        !result.contains("fn bcrypt_hash(password: &str, salt: &str)"),
        "bcrypt_hash function should be deleted"
    );
    assert!(
        !result.contains("password must not be empty"),
        "bcrypt_hash body should be deleted"
    );

    // 验证相邻函数仍然存在
    assert!(
        result.contains("fn generate_salt("),
        "generate_salt should still exist"
    );

    // 验证 diff 包含 Deleted 行
    let deleted_count = engine
        .diff_lines
        .iter()
        .filter(|d| d.kind == n_edit::output::DiffLineKind::Deleted)
        .count();
    assert!(deleted_count > 0, "Should have Deleted diff lines, got {}", deleted_count);
}

// ============================================================
// Phase 2: Replace (Delete + New) 集成测试
// ============================================================

#[test]
fn test_replace_function_delete_then_new() {
    let env = TestEnv::from_data_file("services.rs");
    let script = env.load_script("replace_function.ned");

    let (engine, success) = execute_script(&script);
    assert!(success, "replace_function script failed");

    let result = env.read_target();

    // 旧实现不应存在
    assert!(
        !result.contains("let bytes: Vec<u8> = (0..16).map(|_| rng.gen()).collect();"),
        "Old salt generation code should be removed"
    );
    assert!(
        !result.contains("hex::encode(&bytes)"),
        "Old hex encoding should be removed"
    );

    // 新实现应存在
    assert!(
        result.contains("let mut bytes = [0u8; 32];"),
        "New salt generation should be present"
    );
    assert!(
        result.contains("rng.fill(&mut bytes);"),
        "New random fill should be present"
    );
    assert!(
        result.contains("base64::encode(&bytes)"),
        "New base64 encoding should be present"
    );

    // 函数签名应保持不变
    let fn_count = result
        .lines()
        .filter(|l| l.trim().starts_with("fn generate_salt("))
        .count();
    assert_eq!(fn_count, 1, "Should have exactly one generate_salt function");

    // 验证既有 Added 也有 Deleted 行
    let added: Vec<_> = engine
        .diff_lines
        .iter()
        .filter(|d| d.kind == n_edit::output::DiffLineKind::Added)
        .collect();
    let deleted: Vec<_> = engine
        .diff_lines
        .iter()
        .filter(|d| d.kind == n_edit::output::DiffLineKind::Deleted)
        .collect();
    assert!(!added.is_empty(), "Should have Added diff lines");
    assert!(!deleted.is_empty(), "Should have Deleted diff lines");
}

// ============================================================
// Phase 2: 多操作脚本测试
// ============================================================

#[test]
fn test_multi_operation_script() {
    let env = TestEnv::from_data_file("config.rs");
    let script = env.load_script("multi_operation.ned");

    let (engine, success) = execute_script(&script);
    assert!(success, "multi_operation script failed");

    let result = env.read_target();

    // 操作 1: 添加了新字段
    assert!(result.contains("pub log_level: String"));

    // 操作 2: 添加了新方法
    assert!(result.contains("pub fn reload(&mut self)"));

    // 原有结构和内容仍然完整
    assert!(result.contains("pub struct AppConfig"));
    assert!(result.contains("pub fn from_env() -> Self"));
    assert!(result.contains("pub fn build_database_url"));

    // diff_lines 应同时包含 Added 行
    let added_count = engine
        .diff_lines
        .iter()
        .filter(|d| d.kind == n_edit::output::DiffLineKind::Added)
        .count();
    assert!(
        added_count >= 2,
        "Multi-operation should produce multiple Added diff lines, got {}",
        added_count
    );
}

// ============================================================
// 边界条件测试
// ============================================================

#[test]
fn test_location_too_many_matches_errors() {
    // Location 匹配有歧义时应该报错
    let env = TestEnv::from_data_file("services.rs");

    // 创建脚本：Location 匹配 `pub fn` — 在 services.rs 中有多个匹配
    let script = format!(
        "//!@Open: {}\n//!@Location:\npub fn\n//!@Off:Open\n",
        env.target_path
    );

    let (_, success) = execute_script(&script);
    assert!(!success, "Ambiguous location should fail");
}

#[test]
fn test_new_normal_without_location_errors() {
    let env = TestEnv::from_data_file("config.rs");

    // 没有 Location 就直接 New:Normal，应该报错
    let script = format!(
        "//!@Open: {}\n//!@New:\n    let x = 1;\n...\n//!@Off:Open\n",
        env.target_path
    );

    let (_, success) = execute_script(&script);
    assert!(!success, "New without Location should fail");
}

#[test]
fn test_delete_not_found_errors() {
    let env = TestEnv::from_data_file("config.rs");

    // Delete 内容在文件中不存在，应该报错
    let script = format!(
        "//!@Open: {}\n//!@Location:\npub struct AppConfig\n...\n//!@Delete:\n    nonexistent_field: String,\n...\n//!@Off:Open\n",
        env.target_path
    );

    let (_, success) = execute_script(&script);
    assert!(!success, "Delete not found should fail");
}
