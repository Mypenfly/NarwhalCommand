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
//! ## 迁移来源
//!
//! 从 n_edit/src/engine.rs 拆分重构。

pub mod executor;

use crate::cmd_content::CmdContent;
use crate::error::NcsError;
use crate::model::{ContentBlock, FileContent, SearchScope};
use crate::output::DiffLine;
use crate::parser::Command;
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
    // ========== n_edit 迁移字段 ==========
    /// 当前打开的文件路径（用于最终写回）
    pub file_path: Option<String>,
    /// 当前打开的文件内容（Open 命令后设置）
    pub file: Option<FileContent>,
    /// Location 嵌套栈（栈顶为当前操作作用域）
    pub block_stack: Vec<ContentBlock>,
    /// 执行过程中累积的差异输出行（New=Added, Delete=Deleted）
    pub diff_lines: Vec<DiffLine>,
    /// 上一次记录 diff 时所在的 ContentBlock 标识 (start_line, end_line)
    /// 用于判断是否需要在输出中插入分隔符
    #[allow(dead_code)]
    last_diff_block_key: Option<(usize, usize)>,

    // ========== NCS 新增字段 ==========
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
            file_path: None,
            file: None,
            block_stack: Vec::new(),
            diff_lines: Vec::new(),
            last_diff_block_key: None,
            exec_cmds: Vec::new(),
            pools: HashMap::new(),
            verbose: false,
        }
    }

    /// 设置详细输出模式
    pub fn set_verbose(&mut self, verbose: bool) {
        self.verbose = verbose;
    }

    /// 获取当前搜索范围
    ///
    /// 优先从 block_stack 栈顶获取（嵌套 Location 场景），
    /// 否则从 file 获取。若两者都无则返回错误。
    pub fn get_search_scope(&self) -> Result<SearchScope<'_>, NcsError> {
        if let Some(block) = self.block_stack.last() {
            Ok(SearchScope::Block(block))
        } else {
            self.file
                .as_ref()
                .map(SearchScope::File)
                .ok_or(NcsError::File(crate::error::FileError::NotFound {
                    path: "(no file opened)".to_string(),
                }))
        }
    }

    /// 执行完整的 AST 命令序列
    pub fn execute(&mut self, commands: Vec<Command>) -> Result<(), NcsError> {
        for command in &commands {
            match command {
                Command::Open { mode, path, args } => {
                    crate::commands::open::execute(self, *mode, path, args)?;
                    self.exec_cmds.push(ExecutedCommand {
                        cmd_name: "OPEN".to_string(),
                        mode_name: format!("{:?}", mode),
                        is_independent: false,
                    });
                }
                Command::Location {
                    mode,
                    content,
                    args,
                } => {
                    crate::commands::location::execute(self, *mode, content.clone(), args)?;
                    self.exec_cmds.push(ExecutedCommand {
                        cmd_name: "LOCATION".to_string(),
                        mode_name: format!("{:?}", mode),
                        is_independent: false,
                    });
                }
                Command::New { mode, content } => {
                    crate::commands::new::execute(self, *mode, content.clone())?;
                    self.exec_cmds.push(ExecutedCommand {
                        cmd_name: "NEW".to_string(),
                        mode_name: format!("{:?}", mode),
                        is_independent: false,
                    });
                }
                Command::Delete { mode, content } => {
                    crate::commands::delete::execute(self, *mode, content.clone())?;
                    self.exec_cmds.push(ExecutedCommand {
                        cmd_name: "DELETE".to_string(),
                        mode_name: format!("{:?}", mode),
                        is_independent: false,
                    });
                }
                Command::Raw { content } => {
                    crate::commands::raw::execute(self, content)?;
                }
                Command::Close { name } => {
                    self.handle_close(name)?;
                }
                // Phase 3+: Bash, Exec, Read, Write, Include, WorkPath, Get
                Command::Bash { .. }
                | Command::Exec { .. }
                | Command::Read { .. }
                | Command::Write { .. }
                | Command::Include { .. }
                | Command::WorkPath { .. }
                | Command::Get { .. } => {
                    // 暂未实现，跳过
                }
            }
        }

        self.handle_implicit_close()
    }

    /// 处理 @/Cmd 关闭符号
    fn handle_close(&mut self, name: &str) -> Result<(), NcsError> {
        match name.to_uppercase().as_str() {
            "LOCATION" | "NEW" => {
                let popped_block = self
                    .block_stack
                    .pop()
                    .ok_or(NcsError::Engine(crate::error::EngineError::BlockStackEmpty))?;
                self.write_back_to_parent(popped_block)?;
                // 从 exec_cmds 中移除对应命令
                self.pop_exec_cmd(name);
            }
            "OPEN" => {
                self.write_back_to_file()?;
                self.pop_exec_cmd(name);
            }
            _ => {
                // 未知的关闭目标，忽略或报错
                self.pop_exec_cmd(name);
            }
        }
        Ok(())
    }

    /// 从 exec_cmds 末尾向前移除第一个匹配的命令
    fn pop_exec_cmd(&mut self, name: &str) {
        let upper = name.to_uppercase();
        if let Some(pos) = self.exec_cmds.iter().rposition(|ec| ec.cmd_name == upper) {
            self.exec_cmds.truncate(pos);
        }
    }

    /// 将弹出的 ContentBlock 写回父级
    fn write_back_to_parent(&mut self, block: ContentBlock) -> Result<(), NcsError> {
        if let Some(parent) = self.block_stack.last_mut() {
            executor::apply_block_to_parent(&block, parent);
        } else if let Some(ref mut file) = self.file {
            executor::apply_block_to_file(file, &block);
        }
        Ok(())
    }

    /// 将所有修改最终写回磁盘文件
    fn write_back_to_file(&mut self) -> Result<(), NcsError> {
        while let Some(block) = self.block_stack.pop() {
            if let Some(parent) = self.block_stack.last_mut() {
                executor::apply_block_to_parent(&block, parent);
            } else if let Some(ref mut file) = self.file {
                executor::apply_block_to_file(file, &block);
            }
        }

        if let (Some(ref file), Some(ref path)) = (&self.file, &self.file_path) {
            file.write_back(path)?;
        }

        Ok(())
    }

    /// 处理隐式关闭：脚本末尾未显式 @/Open 时自动写回
    fn handle_implicit_close(&mut self) -> Result<(), NcsError> {
        if self.file.is_some() {
            self.write_back_to_file()?;
        }
        Ok(())
    }

    /// 记录差异行，包含上下文和分隔符
    ///
    /// 将新增/删除行及其上下文推入 diff_lines，
    /// 若 ContentBlock 变更则自动插入分隔符。
    pub fn record_diff_with_context(
        &mut self,
        changed_lines: Vec<DiffLine>,
        context_above: Vec<DiffLine>,
        context_below: Vec<DiffLine>,
    ) {
        self.insert_separator_if_needed();

        for line in context_above {
            self.diff_lines.push(line);
        }
        for line in changed_lines {
            self.diff_lines.push(line);
        }
        for line in context_below {
            self.diff_lines.push(line);
        }

        self.update_diff_block_key();
    }

    /// 记录文件级别的新增行信息
    pub fn record_added_lines(&mut self, lines: Vec<(usize, String)>) {
        for (line_num, content) in lines {
            self.diff_lines.push(DiffLine {
                kind: crate::output::DiffLineKind::Added,
                line_number: Some(crate::model::LineNumber::new(line_num)),
                content,
            });
        }
    }

    /// 若当前 ContentBlock 与上一次不同，插入分隔符
    fn insert_separator_if_needed(&mut self) {
        let current_key = self.get_current_block_key();
        if current_key != self.last_diff_block_key
            && self.last_diff_block_key.is_some()
            && !self.diff_lines.is_empty()
        {
            self.diff_lines.push(DiffLine::separator());
        }
    }

    /// 获取当前 ContentBlock 的唯一标识
    fn get_current_block_key(&self) -> Option<(usize, usize)> {
        self.block_stack
            .last()
            .map(|b| (b.start_line.to_usize(), b.end_line.to_usize()))
    }

    /// 更新最后一次记录的 block key
    fn update_diff_block_key(&mut self) {
        self.last_diff_block_key = self.get_current_block_key();
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
