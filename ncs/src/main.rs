//! 命令行入口 (Main)
//!
//! 解析命令行参数，读取 .ncs 脚本文件，校验后缀，
//! 驱动词法分析 → 语法分析 → 执行引擎流水线。
//!
//! ## 对应文档
//!
//! 详见 INSTRUCTION.md 第 1.1 节 "架构总览", ncs_dev.md §2.1

use clap::Parser;
use ncs::output::{self, OutputFormatter};
use std::path::Path;

/// NCS (Narwhal Command Script) — 基于注解的系统命令脚本工具
///
/// 读取 .ncs 脚本文件，解析其中的命令指令，执行相应的文件操作和系统命令。
#[derive(Parser)]
#[command(name = "ncs")]
#[command(version = "0.1.0")]
#[command(about = "Narwhal Command Script — 系统命令脚本执行工具")]
struct Cli {
    /// .ncs 脚本文件路径
    script_path: String,

    /// 详细输出模式
    #[arg(short, long)]
    verbose: bool,

    /// 静默模式（只输出错误）
    #[arg(short, long)]
    quiet: bool,
}

fn main() {
    let cli = Cli::parse();

    let script_path = Path::new(&cli.script_path);

    let extension = script_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");
    if extension != "ncs" {
        eprintln!(
            "错误: 脚本文件必须具有 .ncs 后缀，当前文件: {}",
            cli.script_path
        );
        std::process::exit(1);
    }

    if !script_path.exists() {
        eprintln!("错误: 脚本文件不存在: {}", cli.script_path);
        std::process::exit(1);
    }

    let script_content = match std::fs::read_to_string(&cli.script_path) {
        Ok(content) => content,
        Err(e) => {
            eprintln!("读取脚本文件 {} 失败: {}", cli.script_path, e);
            std::process::exit(1);
        }
    };

    if cli.verbose {
        eprintln!(
            "[verbose] 脚本加载成功，共 {} 行",
            script_content.lines().count()
        );
    }

    let registry = ncs::registry::CommandRegistry::init();

    let tokens = match ncs::lexer::Lexer::tokenize(&script_content, &registry) {
        Ok(tokens) => tokens,
        Err(e) => {
            eprintln!("{}", output::format_error_colored(&e.to_string(), "", &[]));
            std::process::exit(1);
        }
    };

    if cli.verbose {
        eprintln!("[verbose] 词法分析完成，共 {} 个 Token", tokens.len());
        for token in &tokens {
            eprintln!("  {:?}", token);
        }
    }

    let commands = match ncs::parser::Parser::parse(tokens, &registry) {
        Ok(commands) => commands,
        Err(e) => {
            eprintln!("{}", output::format_error_colored(&e.to_string(), "", &[]));
            std::process::exit(1);
        }
    };

    if cli.verbose {
        eprintln!("[verbose] 语法分析完成，共 {} 条命令", commands.len());
        for command in &commands {
            eprintln!("  {:?}", command);
        }
    }

    let mut engine = ncs::engine::Engine::new();
    engine.set_verbose(cli.verbose);

    match engine.execute(commands, &registry) {
        Ok(()) => {
            if !cli.quiet {
                if !engine.diff_lines.is_empty() {
                    let formatter = OutputFormatter::new();
                    let formatted = formatter.format_diff_lines(&engine.diff_lines);
                    println!("{}", formatted);
                } else {
                    println!("脚本执行完成，无文件变更。");
                }
            }
        }
        Err(e) => {
            eprintln!(
                "{}",
                output::format_error_colored(&e.title(), &e.detail(), &e.hints())
            );
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ncs::engine::Engine;
    use ncs::output::DiffLineKind;
    use ncs::registry::CommandRegistry;

    struct TestEnv {
        dir: tempfile::TempDir,
    }

    impl TestEnv {
        fn new() -> Self {
            TestEnv {
                dir: tempfile::tempdir().unwrap(),
            }
        }

        fn create_file(&self, name: &str, content: &str) -> String {
            let path = self.dir.path().join(name);
            std::fs::write(&path, content).unwrap();
            path.to_str().unwrap().to_string()
        }
    }

    /// 执行完整的 lexer → parser → engine 流水线，返回 engine
    fn run_full_pipeline(script: &str) -> Result<Engine, String> {
        let registry = CommandRegistry::init();
        let tokens =
            ncs::lexer::Lexer::tokenize(script, &registry).map_err(|e| format!("Lexer: {}", e))?;
        let commands =
            ncs::parser::Parser::parse(tokens, &registry).map_err(|e| format!("Parser: {}", e))?;
        let mut engine = Engine::new();
        engine
            .execute(commands, &registry)
            .map_err(|e| format!("Engine: {}", e))?;
        Ok(engine)
    }

    #[test]
    fn test_script_extension_validation() {
        let path = Path::new("test.txt");
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        assert_ne!(ext, "ncs");
    }

    #[test]
    fn test_script_extension_valid() {
        let path = Path::new("test.ncs");
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        assert_eq!(ext, "ncs");
    }

    #[test]
    fn test_read_valid_ncs_file() {
        let env = TestEnv::new();
        let script = "!@Open ./test.rs\n!@Off Open\n";
        let path = env.create_file("test.ncs", script);
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, script);
    }

    #[test]
    fn test_full_pipeline_produces_diff_and_modifies_file() {
        let env = TestEnv::new();
        // 创建目标文件和脚本
        let target_path = env.create_file("target.rs", "fn main() {\n    let x = 1;\n}\n");
        let script = format!(
            "!@Open {}\n!@Location\nfn main() {{\n!@New\n    let y = 2;\n@/New\n@/Open\n",
            target_path
        );

        let engine = run_full_pipeline(&script).expect("Pipeline should succeed");

        // 验证产生了 diff 行
        let added_count = engine
            .diff_lines
            .iter()
            .filter(|d| d.kind == DiffLineKind::Added)
            .count();
        assert!(added_count > 0, "流水线应产生 Added diff 行");

        // 验证文件被修改
        let result = std::fs::read_to_string(&target_path).unwrap();
        assert!(result.contains("let y = 2;"), "目标文件应包含新插入的行");
        assert!(result.contains("fn main() {"), "目标文件应保留原有内容");
    }
}
