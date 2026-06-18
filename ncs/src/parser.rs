//! 语法分析器 (Parser)
//!
//! 负责将 Lexer 输出的 Token 流组装为 AST（Command 序列）。
//!
//! ## 实现逻辑
//!
//! 1. 消费 Token 流，在 CommandRegistry 中查找命令定义
//! 2. 根据命令的模式注册表匹配模式，解析 args
//! 3. 缺失必要参数 → 报 ParamMissing 错误
//! 4. 多余参数 → 警告但继续执行
//! 5. 构建 Command AST
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §4.2 "语法分析", INSTRUCTION.md §2.4
//!
//! ## 实现状态
//!
//! Phase 1 待实现。

use crate::error::ParseError;
use crate::model::{DeleteContent, LocationContent, NewContent};

/// 一条完整的命令语句（AST 节点）
#[derive(Debug, PartialEq)]
pub enum Command {
    /// Open 命令：打开目标文件或目录
    Open {
        /// 模式
        mode: OpenMode,
        /// 文件/目录路径
        path: String,
        /// 参数列表
        args: std::collections::HashMap<String, String>,
    },
    /// Location 命令：定位代码位置
    Location {
        /// 模式
        mode: LocationMode,
        /// 定位内容
        content: Option<LocationContent>,
        /// 参数列表
        args: std::collections::HashMap<String, String>,
    },
    /// New 命令：插入新内容
    New {
        /// 插入位置
        mode: NewMode,
        /// 待插入的内容
        content: NewContent,
    },
    /// Delete 命令：删除匹配内容
    Delete {
        /// 模式
        mode: DeleteMode,
        /// 用于匹配的删除内容
        content: Option<DeleteContent>,
    },
    /// Raw 命令：字面量内容
    Raw {
        /// 字面量内容
        content: String,
    },
    /// Bash 命令：执行 bash 命令
    Bash {
        /// 要执行的命令字符串
        command: String,
    },
    /// Exec 命令：直连终端执行
    Exec {
        /// 要执行的命令字符串
        command: String,
    },
    /// Read 命令：读取文件内容并显示
    Read {
        /// 文件路径
        path: String,
        /// 参数列表
        args: std::collections::HashMap<String, String>,
    },
    /// Write 命令：写入文件
    Write {
        /// 写入模式
        mode: WriteMode,
        /// 文件路径
        path: String,
        /// 写入内容
        content: Option<String>,
    },
    /// Include 命令：导入外部命令
    Include {
        /// 外部命令路径
        path: String,
        /// 参数列表
        args: std::collections::HashMap<String, String>,
    },
    /// WorkPath 命令：设置工作路径
    WorkPath {
        /// 工作路径
        path: String,
    },
    /// Get 命令：从 pools 获取数据
    Get {
        /// pool 键名
        pool_name: String,
        /// 伪装为某个命令
        like: Option<String>,
    },
    /// 关闭符号
    Close {
        /// 关闭的命令名
        name: String,
    },
}

/// Open 命令的模式
#[derive(Debug, PartialEq)]
pub enum OpenMode {
    /// 打开单个文本文件
    Normal,
    /// 打开目录，递归扫描
    Dir,
}

/// Location 命令的模式
#[derive(Debug, PartialEq)]
pub enum LocationMode {
    /// 基于内容和 diff_taps 匹配
    Normal,
    /// 匹配后调用 BlockParser
    Block,
    /// 在指定文件中执行 Normal 匹配
    Path,
}

/// New 命令的插入位置
#[derive(Debug, PartialEq)]
pub enum NewMode {
    /// 在 Location 匹配位置之后插入
    Normal,
    /// 在文件/Block 开头插入
    Start,
    /// 在文件/Block 末尾插入
    End,
}

/// Delete 命令的模式
#[derive(Debug, PartialEq)]
pub enum DeleteMode {
    /// 在 ContentBlock 内匹配并删除连续行
    Normal,
    /// 删除整个 ContentBlock
    Block,
}

/// Write 命令的模式
#[derive(Debug, PartialEq)]
pub enum WriteMode {
    /// 块内容写入文件
    Normal,
    /// 从下一行到 EOF 全部原样写入
    Raw,
}

/// 语法分析器
pub struct Parser;

impl Parser {
    /// 将 Token 序列解析为 Command 序列
    pub fn parse(_tokens: Vec<crate::lexer::Token>) -> Result<Vec<Command>, ParseError> {
        // Phase 1: 待实现
        Ok(Vec::new())
    }
}
