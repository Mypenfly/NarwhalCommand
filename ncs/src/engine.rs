//! 命令执行引擎 (Engine)
//!
//! 维护全局状态机，按顺序消费 Parser 输出的 AST 节点，
//! 管理 exec_cmds、block_stack、file、pools 状态。
//!
//! ## 状态流转
//!
//! Open → Location (可嵌套) → New/Delete/Raw → @/Open
//! Bash/Exec/Read/Write/Include/WorkPath/Get 为独立命令
//!
//! ## 实现逻辑
//!
//! 1. exec_cmds 管理：命令执行后加入，@/Cmd 时清理
//! 2. block_stack 管理：嵌套 Location 的 push/pop
//! 3. CmdContent 数据流：convert/out 传递数据
//! 4. 隐式关闭：脚本末尾自动 flush 未关闭的命令
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §3.2 "exec_cmds", §6.3 "exec_cmds 管理规则"
//! INSTRUCTION.md §1.3 "命令状态机"
//!
//! ## 实现状态
//!
//! Phase 2 待实现（核心逻辑从 n_edit 迁移并适配）。

use crate::cmd_content::CmdContent;
use crate::error::NcsError;
use std::collections::HashMap;

/// 已执行且仍在生效的命令记录
#[derive(Debug, Clone)]
pub struct ExecutedCommand {
    /// 命令名
    pub cmd_name: String,
    /// 模式名
    pub mode_name: String,
    /// 是否为独立命令（无 owners 或 owner 已退出）
    pub is_independent: bool,
}

/// 命令执行引擎
///
/// 维护全局状态机，按顺序消费 AST 节点。
pub struct Engine {
    /// exec_cmds: 记录所有已执行且仍在生效的命令
    pub exec_cmds: Vec<ExecutedCommand>,
    /// 全局数据池: Capture 指令存入的数据
    pub pools: HashMap<String, CmdContent>,
    /// 详细模式
    verbose: bool,
}

impl Engine {
    /// 创建新的执行引擎实例
    pub fn new() -> Self {
        Engine {
            exec_cmds: Vec::new(),
            pools: HashMap::new(),
            verbose: false,
        }
    }

    /// 设置详细输出模式
    pub fn set_verbose(&mut self, verbose: bool) {
        self.verbose = verbose;
    }

    /// 执行完整的 AST 命令序列
    pub fn execute(&mut self, _commands: Vec<crate::parser::Command>) -> Result<(), NcsError> {
        // Phase 2: 待实现
        Ok(())
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
