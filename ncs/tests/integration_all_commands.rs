//! NCS 全部命令集成测试 (Phase 5)
//!
//! 端到端验证所有 13 个已实现的 NCS 命令。
//! 使用 tests/data/ 下的真实源码和 tests/scripts/ 下的 .ncs 脚本。
//!
//! 所有测试操作在临时文件副本上进行，不修改原始数据文件。

use ncs::engine::Engine;
use ncs::output::DiffLineKind;
use ncs::registry::CommandRegistry;
use std::path::Path;

/// 测试环境：持有临时目录和目标文件的副本
struct TestEnv {
    _dir: tempfile::TempDir,
    target_path: String,
}

/// 获取 ncs crate 根目录的绝对路径
fn ncs_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// 确保 Python 测试脚本在临时副本中可执行
fn ensure_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        if perms.mode() & 0o111 == 0 {
            perms.set_mode(perms.mode() | 0o111);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
}

impl TestEnv {
    /// 从 ncs/tests/data/ 复制所有数据文件到临时目录
    fn from_data_file(data_file: &str) -> Self {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");

        let data_dir = ncs_dir().join("tests").join("data");
        if data_dir.exists() {
            for entry in std::fs::read_dir(&data_dir).expect("Failed to read data dir") {
                let entry = entry.expect("Failed to read entry");
                let file_name = entry.file_name();
                let src = entry.path();
                let dst = dir.path().join(&file_name);
                std::fs::copy(&src, &dst)
                    .unwrap_or_else(|_| panic!("Failed to copy {}", src.display()));

                // 确保 Python 脚本可执行
                if file_name
                    .to_str()
                    .map(|n| n.ends_with(".py"))
                    .unwrap_or(false)
                {
                    ensure_executable(&dst);
                }
            }
        }

        let target_path = dir.path().join(data_file).to_str().unwrap().to_string();
        TestEnv {
            target_path,
            _dir: dir,
        }
    }

    /// 读取 .ncs 脚本并替换 Open/Write/Read/WorkPath/Include 路径为临时路径
    fn load_script(&self, script_name: &str) -> String {
        let script_path = ncs_dir().join("tests").join("scripts").join(script_name);
        let script = std::fs::read_to_string(&script_path)
            .unwrap_or_else(|_| panic!("Failed to read script {}", script_path.display()));
        self.replace_paths(&script)
    }

    /// 替换脚本中的路径引用
    fn replace_paths(&self, script: &str) -> String {
        let temp_dir = Path::new(&self.target_path).parent().unwrap();
        script
            .lines()
            .map(|line| {
                let resolve_path = |prefix: &str| -> String {
                    let original = line.strip_prefix(prefix).unwrap().trim();
                    let path_part = original.split_whitespace().next().unwrap_or(original);
                    let file_name = Path::new(path_part)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(path_part);
                    let resolved = temp_dir.join(file_name);
                    let rest = original.strip_prefix(path_part).unwrap_or("");
                    format!("{}{}{}", prefix, resolved.to_str().unwrap(), rest)
                };

                if line.trim_start().starts_with("!@Open ") {
                    resolve_path("!@Open ")
                } else if line.trim_start().starts_with("!@Write Normal ") {
                    resolve_path("!@Write Normal ")
                } else if line.trim_start().starts_with("!@Write Raw ") {
                    resolve_path("!@Write Raw ")
                } else if line.trim_start().starts_with("!@Read ") && !line.contains("Dir ") {
                    resolve_path("!@Read ")
                } else if line.trim_start().starts_with("!@Read Dir ") {
                    let prefix = if line.starts_with("!@Read Dir ") {
                        "!@Read Dir "
                    } else {
                        "!@Read "
                    };
                    let original = line.strip_prefix(prefix).unwrap().trim();
                    // Dir mode: resolve the directory path
                    let path_part = original.split_whitespace().next().unwrap_or(original);
                    let file_name = Path::new(path_part)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(path_part);
                    let resolved = temp_dir.join(file_name);
                    let rest = original.strip_prefix(path_part).unwrap_or("");
                    format!("{}{}{}", prefix, resolved.to_str().unwrap(), rest)
                } else if line.trim_start().starts_with("!@Include ") {
                    resolve_path("!@Include ")
                } else if line.trim_start().starts_with("!@WorkPath ")
                    && !line.contains("NONEXISTENT")
                {
                    format!("!@WorkPath {}", temp_dir.to_str().unwrap())
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// 读取目标文件当前内容
    fn read_target(&self) -> String {
        std::fs::read_to_string(&self.target_path)
            .unwrap_or_else(|_| panic!("Failed to read target {}", self.target_path))
    }
}

/// 执行 NCS 脚本的完整流水线
fn execute_script(script: &str) -> Result<Engine, String> {
    let mut registry = CommandRegistry::init();
    let tokens =
        ncs::lexer::Lexer::tokenize(script, &registry).map_err(|e| format!("Lexer: {}", e))?;
    let commands =
        ncs::parser::Parser::parse(tokens, &registry).map_err(|e| format!("Parser: {}", e))?;
    let mut engine = Engine::new();
    engine
        .execute(commands, &mut registry)
        .map_err(|e| format!("Engine: {}", e))?;
    Ok(engine)
}

/// 验证 diff 输出中 Added/Deleted 行数量
fn assert_diff_counts(engine: &Engine, min_added: usize, min_deleted: usize) {
    let added = engine
        .diff_lines
        .iter()
        .filter(|d| d.kind == DiffLineKind::Added)
        .count();
    let deleted = engine
        .diff_lines
        .iter()
        .filter(|d| d.kind == DiffLineKind::Deleted)
        .count();
    assert!(
        added >= min_added,
        "Expected >= {} Added lines, got {}",
        min_added,
        added
    );
    assert!(
        deleted >= min_deleted,
        "Expected >= {} Deleted lines, got {}",
        min_deleted,
        deleted
    );
}

/// 验证文件包含指定内容
fn assert_file_contains(actual: &str, expected: &[&str]) {
    for sub in expected {
        assert!(actual.contains(sub), "Expected file to contain '{}'", sub);
    }
}

/// 验证文件不包含指定内容
fn assert_file_not_contains(actual: &str, removed: &[&str]) {
    for sub in removed {
        assert!(
            !actual.contains(sub),
            "Expected file to NOT contain '{}'",
            sub
        );
    }
}

// ============================================================
// Test Cases
// ============================================================

/// Test 1: Rust Nested Edit
/// - Add method, delete method, insert test in nested Location
#[test]
fn test_rust_nested_edit() {
    let env = TestEnv::from_data_file("rust_service.rs");
    let script = env.load_script("rust_nested_edit.ncs");

    let engine = execute_script(&script).expect("Script should execute");
    let result = env.read_target();

    // Verify new method added
    assert_file_contains(
        &result,
        &["pub fn update_email", "user.email = new_email;", "Ok(())"],
    );

    // Verify old method deleted
    assert_file_not_contains(&result, &["pub fn count_by_domain"]);

    // Verify test added
    assert_file_contains(&result, &["fn test_update_email", "b@ex.com"]);

    // Verify diff output
    assert_diff_counts(&engine, 5, 5);
}

/// Test 2: Dart Refactor
/// - Add logout method, delete validateSession
#[test]
fn test_dart_refactor() {
    let env = TestEnv::from_data_file("dart_auth.dart");
    let script = env.load_script("dart_refactor.ncs");

    let engine = execute_script(&script).expect("Script should execute");
    let result = env.read_target();

    // Verify logout method added
    assert_file_contains(&result, &["Future<void> logout", "auth/logout"]);

    // Verify validateSession deleted
    assert_file_not_contains(&result, &["Future<bool> validateSession"]);

    assert_diff_counts(&engine, 3, 4);
}

/// Test 3: Markdown Sections
/// - Replace heading, add section, update code block
#[test]
fn test_markdown_sections() {
    let env = TestEnv::from_data_file("docs_guide.md");
    let script = env.load_script("markdown_sections.ncs");

    let engine = execute_script(&script).expect("Script should execute");
    let result = env.read_target();

    // Verify heading replaced
    assert_file_contains(&result, &["## Getting Started"]);
    assert_file_not_contains(&result, &["## Quick Start"]);

    // Verify new Security section
    assert_file_contains(
        &result,
        &[
            "## Security",
            "Enable HTTPS with TLS 1.3",
            "Rotate API keys",
        ],
    );

    // Verify code block updated
    assert_file_contains(&result, &["cargo test", "cargo install --path ."]);

    assert_diff_counts(&engine, 4, 3);
}

/// Test 4: WorkPath + Write + Read
/// - Set work path, write files, read them back
#[test]
fn test_workpath_write() {
    let env = TestEnv::from_data_file("rust_service.rs"); // any file, we only need temp dir
    let script = env.load_script("workpath_write.ncs");

    let engine = execute_script(&script).expect("Script should execute");

    // The config file should be written relative to work path (temp dir)
    let temp_dir = Path::new(&env.target_path).parent().unwrap();
    let config_path = temp_dir.join("_ncs_generated_config.toml");
    assert!(
        config_path.exists(),
        "Config file should be written at {:?}",
        config_path
    );
    let content = std::fs::read_to_string(&config_path).expect("Read config file");
    assert_file_contains(&content, &["[server]", "host = \"0.0.0.0\"", "port = 8080"]);

    // Verify second file
    let notes_path = temp_dir.join("_ncs_notes.txt");
    assert!(
        notes_path.exists(),
        "Notes file should be written at {:?}",
        notes_path
    );
    let notes = std::fs::read_to_string(&notes_path).expect("Read notes file");
    assert_file_contains(&notes, &["NCS integration test notes."]);

    // Should have some diff lines or output
    assert!(engine.had_output, "Script should produce output");
}

/// Test 5: Bash + File Edit
/// - Run bash, edit markdown file
#[test]
fn test_bash_execution() {
    let env = TestEnv::from_data_file("docs_guide.md");
    let script = env.load_script("bash_capture_get.ncs");

    let engine = execute_script(&script).expect("Script should execute");
    let result = env.read_target();

    // Verify new section was added
    assert_file_contains(&result, &["## Test Results", "NCS bash_capture_get test"]);

    assert_diff_counts(&engine, 2, 0);
}

/// Test 6: Raw Expand
/// - Use !@Raw inside !@New block, verify indentation
#[test]
fn test_raw_expand() {
    let env = TestEnv::from_data_file("rust_service.rs");
    let script = env.load_script("raw_expand.ncs");

    let engine = execute_script(&script).expect("Script should execute");
    let result = env.read_target();

    // Verify new method with raw content
    assert_file_contains(
        &result,
        &[
            "pub fn find_by_name",
            ".values()",
            ".filter(|u| u.name.contains(name))",
            ".collect()",
        ],
    );

    // Verify proper indentation (4 spaces for body)
    let lines: Vec<&str> = result.lines().collect();
    let filter_line = lines
        .iter()
        .find(|l| l.contains(".filter(|u| u.name.contains(name))"))
        .expect("Raw line should be present");
    let taps = filter_line.len() - filter_line.trim_start().len();
    assert!(
        taps >= 4,
        "Raw expanded lines should have proper indentation, got {}",
        taps
    );

    assert_diff_counts(&engine, 4, 0);
}

/// Test 7: Include Python
/// - Register external Python script and call it
#[test]
fn test_include_python() {
    let env = TestEnv::from_data_file("python_echo.py");
    let script = env.load_script("include_python.ncs");

    let engine = execute_script(&script).expect("Script should execute");

    // External command output should have been printed
    assert!(
        engine.had_output,
        "Include external command should produce output"
    );

    // Check that the echo command returned content in last_result or pools
    // (the exact output is captured by stdout but we verify no crash)
}

/// Test 8: Like Disguise
/// - Capture + Like disguise + edit
#[test]
fn test_like_disguise() {
    let env = TestEnv::from_data_file("docs_guide.md");
    let script = env.load_script("like_disguise.ncs");

    let engine = execute_script(&script).expect("Script should execute");
    let result = env.read_target();

    // Verify the disguised operation worked (Open was in exec_cmds after Like)
    assert_file_contains(
        &result,
        &[
            "### Performance Issues",
            "high CPU usage",
            "worker thread count",
        ],
    );

    assert_diff_counts(&engine, 2, 0);
}

/// Test 9: Full Pipeline
/// - Multi-command sequence: Bash + Open + Location + New + Delete Block + New End
#[test]
fn test_full_pipeline() {
    let env = TestEnv::from_data_file("rust_service.rs");
    let script = env.load_script("full_pipeline.ncs");

    let engine = execute_script(&script).expect("Script should execute");
    let result = env.read_target();

    // Verify Display impl added after User struct
    assert_file_contains(
        &result,
        &[
            "impl std::fmt::Display for User",
            "write!(f, \"User({}, {}, active={})\"",
        ],
    );

    // Verify count_by_domain deleted
    assert_file_not_contains(&result, &["pub fn count_by_domain"]);

    // Verify New End method added at file end
    assert_file_contains(&result, &["pub fn total_users", "self.users.len()"]);

    // Verify output was produced
    assert!(engine.had_output, "Pipeline should produce output");

    assert_diff_counts(&engine, 5, 4);
}
