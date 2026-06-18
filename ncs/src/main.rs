//! 命令行入口 (Main)
//!
//! 解析命令行参数，读取 .ncs 脚本文件，校验后缀，
//! 驱动词法分析 → 语法分析 → 执行引擎流水线。
//!
//! ## 对应文档
//!
//! 详见 INSTRUCTION.md 第 1.1 节 "架构总览", ncs_dev.md §2.1

use clap::Parser;
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
            eprintln!("词法分析错误: {}", e);
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
            eprintln!("语法分析错误: {}", e);
            std::process::exit(1);
        }
    };

    if cli.verbose {
        eprintln!("[verbose] 语法分析完成，共 {} 条命令", commands.len());
        for command in &commands {
            eprintln!("  {:?}", command);
        }
    }

    if !cli.quiet {
        println!("加载 .ncs 脚本: {}", cli.script_path);
        println!("解析完成: {} 条命令", commands.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
