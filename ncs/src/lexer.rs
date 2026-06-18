//! 词法分析器 (Lexer)
//!
//! 负责将输入的 .ncs 脚本内容扫描为 Token 流。
//!
//! ## 实现逻辑
//!
//! 1. 逐行读取脚本内容，识别 `!@` 标识符作为命令起始
//! 2. 根据 CommandRegistry 确定命令的执行类型（行/块）
//! 3. 块执行命令按终止规则提取后续内容行
//! 4. `!@Raw` 和 `!@Get` 作为仅展开命令，不触发块终止
//! 5. 输出有序的 Token 序列供 Parser 使用
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §4.1 "词法分析", INSTRUCTION.md §7.2
//!
//! ## 实现状态
//!
//! Phase 1 待实现。

/// 词法分析器产出的 Token
#[derive(Debug, PartialEq)]
pub enum Token {
    /// 命令语句
    Command {
        /// 命令名
        name: String,
        /// 模式名
        mode: String,
        /// 参数列表（键值对）
        args: std::collections::HashMap<String, String>,
        /// 命令所在行号
        line: crate::model::LineNumber,
        /// 块执行命令的内容行（行执行为空）
        content_lines: Vec<String>,
        /// 是否为块执行
        is_block: bool,
    },
    /// 关闭符号
    Close {
        /// 关闭的命令名
        name: String,
        /// 所在行号
        line: crate::model::LineNumber,
    },
    /// Capture 指令：捕获命令输出到 pools
    Capture {
        /// 存入 pools 的键名
        pool_name: String,
        /// 所在行号
        line: crate::model::LineNumber,
    },
}

/// 词法分析器
pub struct Lexer;

impl Lexer {
    /// 对脚本内容执行词法分析，返回 Token 流
    pub fn tokenize(_script: &str) -> Vec<Token> {
        // Phase 1: 待实现
        Vec::new()
    }
}
