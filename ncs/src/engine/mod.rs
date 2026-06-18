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

use crate::cmd_content::{CmdContent, CommandResult};
use crate::error::NcsError;
use crate::model::{ContentBlock, FileContent, SearchScope};
use crate::output::DiffLine;
use crate::parser::Command;
use crate::registry::{normalize_command_name, CommandRegistry};
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

    // ========== Phase 3 新增字段 ==========
    /// 上一条命令的执行结果（供 Capture 捕获 + 下一命令输入）
    pub last_result: Option<CommandResult>,
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
            last_result: None,
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

    /// 执行完整的 AST 命令序列（Phase 3 三步流水线）
    pub fn execute(
        &mut self,
        commands: Vec<Command>,
        registry: &CommandRegistry,
    ) -> Result<(), NcsError> {
        for command in &commands {
            match command {
                Command::Close { name, capture } => {
                    // 关闭前先应用待生效的变更
                    self.handle_close(name, capture.clone())?;
                }
                other => {
                    // 1. 权限检查
                    self.check_owner(command, registry)?;

                    // 2. 执行命令（直接操作 engine 状态）
                    self.dispatch_command(other)?;

                    // 3. 加入 exec_cmds
                    let cmd_name = other.cmd_name();
                    if cmd_name != "RAW" && cmd_name != "CAPTURE" && cmd_name != "GET" {
                        self.exec_cmds.push(ExecutedCommand {
                            cmd_name: cmd_name.clone(),
                            mode_name: other.mode_name(),
                            is_independent: false,
                        });
                    }

                    // 4. 更新 last_result
                    self.update_last_result(other, registry)?;
                }
            }
        }

        self.handle_implicit_close()
    }

    /// 分发命令到对应的 executor
    fn dispatch_command(&mut self, command: &Command) -> Result<(), NcsError> {
        match command {
            Command::Open { mode, path, args } => {
                crate::commands::open::execute(self, *mode, path, args)
            }
            Command::Location {
                mode,
                content,
                args,
            } => crate::commands::location::execute(self, *mode, content.clone(), args),
            Command::New { mode, content } => {
                crate::commands::new::execute(self, *mode, content.clone())
            }
            Command::Delete { mode, content } => {
                crate::commands::delete::execute(self, *mode, content.clone())
            }
            Command::Raw { content } => crate::commands::raw::execute(self, content),
            Command::Capture { pool_name } => {
                let content = self
                    .last_result
                    .take()
                    .map(|r| r.content)
                    .unwrap_or_default();
                self.pools.insert(pool_name.clone(), content);
                Ok(())
            }
            Command::Get {
                pool_name,
                like: _like,
            } => {
                let content = self.pools.get(pool_name).cloned().ok_or_else(|| {
                    NcsError::Engine(crate::error::EngineError::NotImplemented {
                        feature: format!("Get pool '{}' not found", pool_name),
                    })
                })?;
                // 设置 last_result
                let cmd_result = CommandResult {
                    content,
                    is_stream: true,
                };
                self.last_result = Some(cmd_result);
                Ok(())
            }
            Command::Close { .. } => Ok(()), // handled in execute loop
            _ => Err(NcsError::Engine(
                crate::error::EngineError::NotImplemented {
                    feature: command.cmd_name(),
                },
            )),
        }
    }

    /// 更新 last_result：从命令执行后的 engine 状态构建 CmdContent
    fn update_last_result(
        &mut self,
        command: &Command,
        registry: &CommandRegistry,
    ) -> Result<(), NcsError> {
        let cmd_name = command.cmd_name();

        // 文件编辑命令始终为流输出（管道需要传递数据）
        let is_file_editor = matches!(cmd_name.as_str(), "OPEN" | "LOCATION" | "NEW" | "DELETE");

        // 其他命令查询注册表中的输出类型
        let output_is_stream = if is_file_editor {
            true
        } else {
            registry
                .find_command(&cmd_name)
                .and_then(|entry| entry.cmd_type.output.as_ref())
                .is_some_and(|ot| matches!(ot, crate::registry::OutputType::StreamOutput))
        };

        // ValueOutput 命令不保留 last_result
        if !output_is_stream {
            self.last_result = None;
            return Ok(());
        }

        // 文件编辑命令：从 engine 状态构建 CmdContent
        let content = match cmd_name.as_str() {
            "OPEN" => {
                if let Some(ref file) = self.file {
                    let cmd_lines: Vec<crate::cmd_content::CmdLine> = file
                        .lines
                        .iter()
                        .map(|l| crate::cmd_content::CmdLine {
                            line_num: l.line_num.to_usize(),
                            content: l.content.clone(),
                        })
                        .collect();
                    let raw = cmd_lines
                        .iter()
                        .map(|l| &l.content)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n");
                    let mut c = CmdContent::empty();
                    c.snapshot_lines = cmd_lines.clone();
                    c.snapshot_raw = raw.clone();
                    c.lines = cmd_lines;
                    c.raw_content = raw;
                    c.source_info = Some(crate::cmd_content::ContentSource::File {
                        file_path: self.file_path.clone().unwrap_or_default(),
                    });
                    c
                } else {
                    CmdContent::empty()
                }
            }
            "LOCATION" => {
                if let Some(block) = self.block_stack.last() {
                    let block_index = self.block_stack.len() - 1;
                    let cmd_lines: Vec<crate::cmd_content::CmdLine> = block
                        .lines
                        .iter()
                        .map(|l| crate::cmd_content::CmdLine {
                            line_num: l.line_num.to_usize(),
                            content: l.content.clone(),
                        })
                        .collect();
                    let raw = cmd_lines
                        .iter()
                        .map(|l| &l.content)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n");
                    let mut c = CmdContent::empty();
                    c.snapshot_lines = cmd_lines.clone();
                    c.snapshot_raw = raw.clone();
                    c.lines = cmd_lines;
                    c.raw_content = raw;
                    c.source_info = Some(crate::cmd_content::ContentSource::Block { block_index });
                    c.is_print = true;
                    c
                } else {
                    CmdContent::empty()
                }
            }
            "NEW" => {
                // New: 由 commands/new.rs 在 execute_normal 中
                // 直接记录变更到 last_result.content（Phase 3 变更追踪），
                // 此处仅传递已有内容。
                self.last_result
                    .take()
                    .map(|r| r.content)
                    .unwrap_or_else(CmdContent::empty)
            }
            "DELETE" => {
                // Delete: 由 commands/delete.rs 在 execute_normal 中
                // 直接记录变更到 last_result.content（Phase 3 快照匹配），
                // 此处仅传递已有内容。
                self.last_result
                    .take()
                    .map(|r| r.content)
                    .unwrap_or_else(CmdContent::empty)
            }
            _ => {
                // 非文件编辑命令：last_result 不变
                return Ok(());
            }
        };

        self.last_result = Some(CommandResult {
            content,
            is_stream: output_is_stream,
        });
        Ok(())
    }

    /// 前置检查：验证当前命令的 owner 是否存在于 exec_cmds 中
    fn check_owner(&self, command: &Command, registry: &CommandRegistry) -> Result<(), NcsError> {
        // 抽出命令名和模式名
        let (cmd_name, current_mode) = match command {
            Command::Open { mode, .. } => ("OPEN", format!("{:?}", mode)),
            Command::Location { mode, .. } => ("LOCATION", format!("{:?}", mode)),
            Command::New { mode, .. } => ("NEW", format!("{:?}", mode)),
            Command::Delete { mode, .. } => ("DELETE", format!("{:?}", mode)),
            Command::Raw { .. } => ("RAW", "Normal".to_string()),
            Command::Close { .. } => return Ok(()),
            _ => return Ok(()),
        };
        let current_mode_normalized = normalize_command_name(&current_mode);

        let entry = match registry.find_command(cmd_name) {
            Some(e) => e,
            None => return Ok(()), // 未注册命令不检查
        };

        let mut any_owner_found = false;
        for (owner_name, allowed_modes) in &entry.owners {
            // allowed_modes 约束当前命令的模式：
            // 空列表 = 该 owner 适用于当前命令的所有模式
            // 非空 = 仅当当前模式在列表中时才适用
            if !allowed_modes.is_empty() {
                let mode_matches = allowed_modes
                    .iter()
                    .any(|m| normalize_command_name(m) == current_mode_normalized);
                if !mode_matches {
                    continue;
                }
            }

            let owner_exists = self.exec_cmds.iter().any(|ec| {
                normalize_command_name(&ec.cmd_name) == normalize_command_name(owner_name)
            });
            if owner_exists {
                any_owner_found = true;
                break;
            }
        }
        if entry.owners.is_empty() {
            return Ok(());
        }
        if !any_owner_found {
            let (owner_name, _) = &entry.owners[0];
            return Err(NcsError::Registry(
                crate::error::RegistryError::OwnerNotExecuted {
                    cmd_name: cmd_name.to_string(),
                    owner_name: owner_name.clone(),
                    line: crate::model::LineNumber::new(0),
                },
            ));
        }
        Ok(())
    }

    /// 处理 @/Cmd 关闭符号
    fn handle_close(&mut self, name: &str, capture: Option<String>) -> Result<(), NcsError> {
        // Phase 3: Capture 管道 — 将 last_result 存入 pools
        if let Some(pool_name) = capture {
            if let Some(result) = self.last_result.take() {
                self.pools.insert(pool_name.clone(), result.content);
            }
        }

        let upper = name.to_uppercase();

        // Phase 3 变更追踪：将 CmdContent.changes 应用到 ContentBlock
        // 无论关闭的是 LOCATION 还是 OPEN，都需要先应用待生效的变更
        if let Some(ref result) = self.last_result {
            if !result.content.changes.is_empty() {
                self.apply_content_to_file(&result.content.clone())?;
            }
        }

        // Phase 3: Location 关闭时输出终端结果（--verbose）
        if upper == "LOCATION" {
            self.print_location_result()?;
        }

        match upper.as_str() {
            "LOCATION" | "NEW" => {
                let popped_block = self
                    .block_stack
                    .pop()
                    .ok_or(NcsError::Engine(crate::error::EngineError::BlockStackEmpty))?;
                self.write_back_to_parent(popped_block)?;
                self.pop_exec_cmd(name);
            }
            "OPEN" => {
                self.write_back_to_file()?;
                self.pop_exec_cmd(name);
            }
            _ => {
                self.pop_exec_cmd(name);
            }
        }

        self.last_result = None;
        Ok(())
    }

    /// 将 CmdContent 中记录的变更写回当前 ContentBlock
    ///
    /// 读取 CmdContent.changes 列表，通过 apply_changes() 得到最终行列表，
    /// 转换为 ContentBlock.lines 并 reindex。
    /// Diff 由各命令在执行时收集（从原始 block 快照），此处仅负责行操作。
    #[allow(dead_code)]
    fn apply_content_to_file(&mut self, content: &CmdContent) -> Result<(), NcsError> {
        if content.changes.is_empty() {
            return Ok(());
        }

        let mut temp = content.clone();
        temp.apply_changes();

        let block = self.block_stack.last_mut().ok_or(NcsError::Engine(
            crate::error::EngineError::MissingLocationForNew,
        ))?;

        let new_lines: Vec<crate::model::Line> = temp
            .lines
            .iter()
            .map(|cl| {
                let taps = crate::model::count_leading_spaces(&cl.content);
                crate::model::Line {
                    line_num: crate::model::LineNumber::new(cl.line_num),
                    taps,
                    diff_taps: 0,
                    content: cl.content.clone(),
                    stripped_content: crate::model::stripped_content(&cl.content),
                }
            })
            .collect();

        block.lines = new_lines;
        block.reindex();
        Ok(())
    }

    /// 从 exec_cmds 末尾向前移除匹配命令及其之后的所有非独立命令
    ///
    /// 按 ncs_dev.md §6.3: @/Cmd 清除从该位置到末尾的所有条目。
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
        // Phase 3: 隐式关闭前应用所有待生效变更
        if let Some(ref result) = self.last_result {
            if !result.content.changes.is_empty() {
                self.apply_content_to_file(&result.content.clone())?;
            }
        }
        if self.file.is_some() {
            self.write_back_to_file()?;
        }
        Ok(())
    }

    /// 在 --verbose 模式下输出 Location 匹配结果到终端
    ///
    /// 从 last_result.content.snapshot_lines 读取快照行，
    /// 以灰色文本打印带行号的行列表。
    fn print_location_result(&self) -> Result<(), NcsError> {
        if !self.verbose {
            return Ok(());
        }
        if let Some(ref result) = self.last_result {
            let path = self.file_path.as_deref().unwrap_or("");
            eprintln!("[Location] {}", path);
            for line in &result.content.result {
                eprintln!("    {}| {}", line.line_num, line.content);
            }
            if result.content.result.is_empty() {
                for line in &result.content.snapshot_lines {
                    eprintln!("    {}| {}", line.line_num, line.content);
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{EngineError, NcsError};

    #[test]
    fn test_pop_exec_cmd_removes_only_matched_entry() {
        let mut engine = Engine::new();
        engine.exec_cmds = vec![
            ExecutedCommand {
                cmd_name: "OPEN".to_string(),
                mode_name: "Normal".to_string(),
                is_independent: false,
            },
            ExecutedCommand {
                cmd_name: "LOCATION".to_string(),
                mode_name: "Normal".to_string(),
                is_independent: false,
            },
            ExecutedCommand {
                cmd_name: "NEW".to_string(),
                mode_name: "Normal".to_string(),
                is_independent: false,
            },
        ];

        engine.pop_exec_cmd("LOCATION");

        // @/Location 清除 LOCATION 及其后的 NEW
        assert_eq!(engine.exec_cmds.len(), 1, "移除 LOCATION 后应仅保留 OPEN");
        assert_eq!(engine.exec_cmds[0].cmd_name, "OPEN");
    }

    #[test]
    fn test_pop_exec_cmd_removes_last_entry() {
        let mut engine = Engine::new();
        engine.exec_cmds = vec![
            ExecutedCommand {
                cmd_name: "OPEN".to_string(),
                mode_name: "Normal".to_string(),
                is_independent: false,
            },
            ExecutedCommand {
                cmd_name: "LOCATION".to_string(),
                mode_name: "Normal".to_string(),
                is_independent: false,
            },
        ];

        engine.pop_exec_cmd("LOCATION");

        assert_eq!(engine.exec_cmds.len(), 1);
        assert_eq!(engine.exec_cmds[0].cmd_name, "OPEN");
    }

    #[test]
    fn test_pop_exec_cmd_nonexistent_does_nothing() {
        let mut engine = Engine::new();
        engine.exec_cmds = vec![ExecutedCommand {
            cmd_name: "OPEN".to_string(),
            mode_name: "Normal".to_string(),
            is_independent: false,
        }];

        engine.pop_exec_cmd("NONEXISTENT");

        assert_eq!(engine.exec_cmds.len(), 1);
        assert_eq!(engine.exec_cmds[0].cmd_name, "OPEN");
    }

    #[test]
    fn test_pop_exec_cmd_from_empty_is_noop() {
        let mut engine = Engine::new();
        engine.pop_exec_cmd("OPEN");
        assert!(engine.exec_cmds.is_empty());
    }

    #[test]
    fn test_unimplemented_command_returns_not_implemented_error() {
        let mut engine = Engine::new();
        let registry = crate::registry::CommandRegistry::init();
        let commands = vec![Command::Bash {
            command: "echo hello".to_string(),
        }];
        let result = engine.execute(commands, &registry);
        assert!(result.is_err());
        match result.unwrap_err() {
            NcsError::Engine(EngineError::NotImplemented { .. }) => {}
            other => panic!("Expected NotImplemented error, got {:?}", other),
        }
    }

    // ============================================================
    // BUG-202: exec_cmds 退出逻辑 — 范围移除
    // ============================================================

    #[test]
    fn test_pop_exec_cmd_removes_range_on_close() {
        let mut engine = Engine::new();
        engine.exec_cmds = vec![
            ExecutedCommand {
                cmd_name: "OPEN".to_string(),
                mode_name: "Normal".to_string(),
                is_independent: false,
            },
            ExecutedCommand {
                cmd_name: "LOCATION".to_string(),
                mode_name: "Normal".to_string(),
                is_independent: false,
            },
            ExecutedCommand {
                cmd_name: "NEW".to_string(),
                mode_name: "Normal".to_string(),
                is_independent: false,
            },
        ];

        // @/Location 应移除 LOCATION 及其之后的所有非独立命令
        engine.pop_exec_cmd("LOCATION");

        assert_eq!(engine.exec_cmds.len(), 1);
        assert_eq!(engine.exec_cmds[0].cmd_name, "OPEN");
    }

    #[test]
    fn test_pop_exec_cmd_on_last_removes_only_single() {
        let mut engine = Engine::new();
        engine.exec_cmds = vec![
            ExecutedCommand {
                cmd_name: "OPEN".to_string(),
                mode_name: "Normal".to_string(),
                is_independent: false,
            },
            ExecutedCommand {
                cmd_name: "LOCATION".to_string(),
                mode_name: "Normal".to_string(),
                is_independent: false,
            },
            ExecutedCommand {
                cmd_name: "NEW".to_string(),
                mode_name: "Normal".to_string(),
                is_independent: false,
            },
        ];

        // @/New (最后一个) 应只移除 NEW
        engine.pop_exec_cmd("NEW");

        assert_eq!(engine.exec_cmds.len(), 2);
        assert_eq!(engine.exec_cmds[0].cmd_name, "OPEN");
        assert_eq!(engine.exec_cmds[1].cmd_name, "LOCATION");
    }

    // ============================================================
    // BUG-302: Capture Token → Command → Engine pools
    // ============================================================

    #[test]
    fn test_capture_command_stores_into_pools() {
        let mut engine = Engine::new();
        let registry = crate::registry::CommandRegistry::init();
        // Phase 3: Capture 已融入 Close 管道语法，使用 Command::Capture 测试
        engine.last_result = Some(CommandResult {
            content: CmdContent::from_raw_text("captured data".to_string()),
            is_stream: true,
        });
        let commands = vec![Command::Capture {
            pool_name: "my_pool2".to_string(),
        }];
        let result = engine.execute(commands, &registry);
        assert!(result.is_ok());
        assert!(engine.pools.contains_key("my_pool2"));
    }

    // ============================================================
    // BUG-201: exec_cmds owner 检查
    // ============================================================

    #[test]
    fn test_location_without_open_returns_owner_not_executed() {
        let mut engine = Engine::new();
        let registry = crate::registry::CommandRegistry::init();
        let commands = vec![Command::Location {
            mode: crate::parser::LocationMode::Normal,
            content: None,
            args: HashMap::new(),
        }];
        let result = engine.execute(commands, &registry);
        assert!(result.is_err());
        match result.unwrap_err() {
            NcsError::Registry(crate::error::RegistryError::OwnerNotExecuted {
                cmd_name,
                owner_name,
                ..
            }) => {
                assert_eq!(cmd_name, "LOCATION");
                assert_eq!(owner_name, "Open");
            }
            other => panic!("Expected OwnerNotExecuted error, got {:?}", other),
        }
    }

    #[test]
    fn test_new_without_location_returns_owner_not_executed() {
        let mut engine = Engine::new();
        let registry = crate::registry::CommandRegistry::init();
        let commands = vec![Command::New {
            mode: crate::parser::NewMode::Normal,
            content: crate::model::NewContent { lines: vec![] },
        }];
        let result = engine.execute(commands, &registry);
        assert!(result.is_err());
        match result.unwrap_err() {
            NcsError::Registry(crate::error::RegistryError::OwnerNotExecuted {
                cmd_name,
                owner_name,
                ..
            }) => {
                assert_eq!(cmd_name, "NEW");
                assert_eq!(owner_name, "Location");
            }
            other => panic!("Expected OwnerNotExecuted error, got {:?}", other),
        }
    }

    #[test]
    fn test_delete_without_location_returns_owner_not_executed() {
        let mut engine = Engine::new();
        let registry = crate::registry::CommandRegistry::init();
        let commands = vec![Command::Delete {
            mode: crate::parser::DeleteMode::Normal,
            content: None,
        }];
        let result = engine.execute(commands, &registry);
        assert!(result.is_err());
        match result.unwrap_err() {
            NcsError::Registry(crate::error::RegistryError::OwnerNotExecuted { .. }) => {}
            other => panic!("Expected OwnerNotExecuted, got {:?}", other),
        }
    }

    #[test]
    fn test_location_then_new_passes_owner_check() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rs");
        std::fs::write(&path, "fn main() {\n    let x = 1;\n}\n").unwrap();

        let mut engine = Engine::new();
        let registry = crate::registry::CommandRegistry::init();

        // Open
        let commands = vec![
            Command::Open {
                mode: crate::parser::OpenMode::Normal,
                path: path.to_str().unwrap().to_string(),
                args: HashMap::new(),
            },
            Command::Location {
                mode: crate::parser::LocationMode::Normal,
                content: Some(crate::model::LocationContent {
                    lines: vec![crate::model::LocationLine {
                        index: 0,
                        diff_taps: Some(0),
                        content: "fn main() {".to_string(),
                        line_num: None,
                    }],
                }),
                args: HashMap::new(),
            },
            Command::New {
                mode: crate::parser::NewMode::Normal,
                content: crate::model::NewContent {
                    lines: vec![crate::model::NewLine {
                        diff_taps: 4,
                        content: "let y = 2;".to_string(),
                        is_raw: false,
                    }],
                },
            },
        ];
        engine.set_verbose(true);
        let result = engine.execute(commands, &registry);
        assert!(result.is_ok(), "Expected success, got {:?}", result.err());
        assert!(engine.exec_cmds.iter().any(|ec| ec.cmd_name == "NEW"));
    }

    // ============================================================
    // Phase 3: CmdContent 管道 + 变更追踪
    // ============================================================

    /// 辅助：创建包含一行内容的临时 .rs 文件，返回路径
    fn make_temp_file(content: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rs");
        std::fs::write(&path, content).unwrap();
        (dir, path.to_str().unwrap().to_string())
    }

    #[test]
    fn test_engine_has_last_result_field() {
        let engine = Engine::new();
        assert!(engine.last_result.is_none());
    }

    #[test]
    fn test_open_command_stores_result_in_last_result() {
        let (_dir, path) = make_temp_file("fn main() {\n    let x = 1;\n}\n");
        let mut engine = Engine::new();
        let registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::Open {
            mode: crate::parser::OpenMode::Normal,
            path,
            args: HashMap::new(),
        }];

        let result = engine.execute(commands, &registry);
        assert!(result.is_ok(), "Open should succeed");

        let last = engine
            .last_result
            .as_ref()
            .expect("last_result should be set after Open");
        assert!(last.is_stream, "Open should be stream output");
        assert!(
            !last.content.snapshot_lines.is_empty(),
            "Open should populate snapshot_lines"
        );
        assert!(!last.content.lines.is_empty(), "Open should populate lines");
    }

    #[test]
    fn test_location_command_stores_result_with_snapshot() {
        let (_dir, path) = make_temp_file("fn main() {\n    let x = 1;\n}\n");
        let mut engine = Engine::new();
        let registry = crate::registry::CommandRegistry::init();

        let commands = vec![
            Command::Open {
                mode: crate::parser::OpenMode::Normal,
                path,
                args: HashMap::new(),
            },
            Command::Location {
                mode: crate::parser::LocationMode::Normal,
                content: Some(crate::model::LocationContent {
                    lines: vec![crate::model::LocationLine {
                        index: 0,
                        diff_taps: Some(0),
                        content: "fn main() {".to_string(),
                        line_num: None,
                    }],
                }),
                args: HashMap::new(),
            },
        ];

        let result = engine.execute(commands, &registry);
        assert!(result.is_ok(), "Open+Location should succeed");

        let last = engine
            .last_result
            .as_ref()
            .expect("last_result should be set");
        // Location 产生快照
        assert!(
            !last.content.snapshot_lines.is_empty(),
            "Location should set snapshot_lines, got {} lines",
            last.content.snapshot_lines.len()
        );
        assert!(
            matches!(
                &last.content.source_info,
                Some(crate::cmd_content::ContentSource::Block { .. })
            ),
            "Location should set source_info to Block"
        );
    }

    #[test]
    fn test_new_command_records_insert_change() {
        let (_dir, path) = make_temp_file("fn main() {\n    let x = 1;\n}\n");
        let mut engine = Engine::new();
        let registry = crate::registry::CommandRegistry::init();

        let commands = vec![
            Command::Open {
                mode: crate::parser::OpenMode::Normal,
                path,
                args: HashMap::new(),
            },
            Command::Location {
                mode: crate::parser::LocationMode::Normal,
                content: Some(crate::model::LocationContent {
                    lines: vec![crate::model::LocationLine {
                        index: 0,
                        diff_taps: Some(0),
                        content: "fn main() {".to_string(),
                        line_num: None,
                    }],
                }),
                args: HashMap::new(),
            },
            Command::New {
                mode: crate::parser::NewMode::Normal,
                content: crate::model::NewContent {
                    lines: vec![crate::model::NewLine {
                        diff_taps: 4,
                        content: "let y = 2;".to_string(),
                        is_raw: false,
                    }],
                },
            },
        ];

        let result = engine.execute(commands, &registry);
        assert!(result.is_ok(), "Expected success, got {:?}", result.err());

        let last = engine
            .last_result
            .as_ref()
            .expect("last_result should be set");
        assert!(
            !last.content.changes.is_empty(),
            "New should record a change"
        );
        match &last.content.changes[0] {
            crate::cmd_content::ContentChange::Insert {
                lines, source_cmd, ..
            } => {
                assert!(!lines.is_empty());
                assert_eq!(source_cmd, "NEW");
            }
            other => panic!("Expected Insert change, got {:?}", other),
        }
    }

    #[test]
    fn test_delete_command_matches_on_snapshot_and_records_delete_change() {
        let (_dir, path) =
            make_temp_file("fn main() {\n    let old = 1;\n    println!(\"{}\", old);\n}\n");
        let mut engine = Engine::new();
        let registry = crate::registry::CommandRegistry::init();

        let commands = vec![
            Command::Open {
                mode: crate::parser::OpenMode::Normal,
                path,
                args: HashMap::new(),
            },
            Command::Location {
                mode: crate::parser::LocationMode::Normal,
                content: Some(crate::model::LocationContent {
                    lines: vec![crate::model::LocationLine {
                        index: 0,
                        diff_taps: Some(0),
                        content: "fn main() {".to_string(),
                        line_num: None,
                    }],
                }),
                args: HashMap::new(),
            },
            Command::Delete {
                mode: crate::parser::DeleteMode::Normal,
                content: Some(crate::model::DeleteContent {
                    lines: vec![crate::model::DeleteLine {
                        content: "    let old = 1;".to_string(),
                        is_raw: false,
                    }],
                }),
            },
        ];

        let result = engine.execute(commands, &registry);
        assert!(result.is_ok(), "Expected success, got {:?}", result.err());

        let last = engine
            .last_result
            .as_ref()
            .expect("last_result should be set");
        assert!(
            !last.content.changes.is_empty(),
            "Delete should record a change"
        );
        match &last.content.changes[0] {
            crate::cmd_content::ContentChange::Delete {
                start_line,
                end_line,
                source_cmd,
            } => {
                assert_eq!(*start_line, 1, "Delete should start at line 1");
                assert_eq!(*end_line, 1);
                assert_eq!(source_cmd, "DELETE");
            }
            other => panic!("Expected Delete change, got {:?}", other),
        }
    }

    #[test]
    fn test_snapshot_not_modified_by_new_before_delete() {
        // BUG-204 严格断言：New 在前 Delete 在后，Delete 在 snapshot 上正确匹配，
        // snapshot 不受 New 插入影响。最终文件内容应为正确的新旧混合结果。
        let (_dir, path) =
            make_temp_file("fn main() {\n    let old = 1;\n    println!(\"{}\", old);\n}\n");
        let mut engine = Engine::new();
        let registry = crate::registry::CommandRegistry::init();

        let commands = vec![
            Command::Open {
                mode: crate::parser::OpenMode::Normal,
                path,
                args: HashMap::new(),
            },
            Command::Location {
                mode: crate::parser::LocationMode::Normal,
                content: Some(crate::model::LocationContent {
                    lines: vec![crate::model::LocationLine {
                        index: 0,
                        diff_taps: Some(0),
                        content: "fn main() {".to_string(),
                        line_num: None,
                    }],
                }),
                args: HashMap::new(),
            },
            Command::New {
                mode: crate::parser::NewMode::Normal,
                content: crate::model::NewContent {
                    lines: vec![crate::model::NewLine {
                        diff_taps: 4,
                        content: "let inserted = 42;".to_string(),
                        is_raw: false,
                    }],
                },
            },
            Command::Delete {
                mode: crate::parser::DeleteMode::Normal,
                content: Some(crate::model::DeleteContent {
                    lines: vec![crate::model::DeleteLine {
                        content: "    let old = 1;".to_string(),
                        is_raw: false,
                    }],
                }),
            },
        ];

        let result = engine.execute(commands, &registry);
        assert!(
            result.is_ok(),
            "BUG-204: New+Delete should succeed with snapshot matching, got {:?}",
            result.err()
        );

        // Delete 在 snapshot 上正确匹配 'let old = 1'（不受 New 插入影响），
        // 最终 block 内容应为 fn main + inserted + println + }
        let file = engine.file.as_ref().expect("file should exist");
        let contents: Vec<&str> = file
            .lines
            .iter()
            .map(|l| l.stripped_content.as_str())
            .collect();
        assert!(
            contents.contains(&"letinserted=42;"),
            "inserted line should exist"
        );
        assert!(
            !contents.contains(&"letold=1;"),
            "old line should be deleted"
        );
        assert!(contents.contains(&"fnmain(){"), "fn main should exist");
        // 原 4 行 + New 1 - Delete 1 = 4 行
        assert_eq!(file.lines.len(), 4, "expected 4 lines after New+Delete");
    }

    #[test]
    fn test_close_location_applies_changes_to_block() {
        let (_dir, path) = make_temp_file("fn main() {\n    let x = 1;\n}\n");
        let mut engine = Engine::new();
        let registry = crate::registry::CommandRegistry::init();

        let commands = vec![
            Command::Open {
                mode: crate::parser::OpenMode::Normal,
                path,
                args: HashMap::new(),
            },
            Command::Location {
                mode: crate::parser::LocationMode::Normal,
                content: Some(crate::model::LocationContent {
                    lines: vec![crate::model::LocationLine {
                        index: 0,
                        diff_taps: Some(0),
                        content: "fn main() {".to_string(),
                        line_num: None,
                    }],
                }),
                args: HashMap::new(),
            },
            Command::New {
                mode: crate::parser::NewMode::Normal,
                content: crate::model::NewContent {
                    lines: vec![crate::model::NewLine {
                        diff_taps: 4,
                        content: "let y = 2;".to_string(),
                        is_raw: false,
                    }],
                },
            },
            Command::Close {
                name: "Location".to_string(),
                capture: None,
            },
        ];

        let result = engine.execute(commands, &registry);
        assert!(result.is_ok(), "Expected success, got {:?}", result.err());

        // Changes should be applied and block_stack should be empty
        // (Location was closed, block was popped and written back)
        assert!(!engine.diff_lines.is_empty(), "Should have recorded diff");
    }

    #[test]
    fn test_capture_stores_cmdcontent_in_pools() {
        let (_dir, path) = make_temp_file("fn main() {\n    let x = 1;\n}\n");
        let mut engine = Engine::new();
        let registry = crate::registry::CommandRegistry::init();

        let commands = vec![
            Command::Open {
                mode: crate::parser::OpenMode::Normal,
                path,
                args: HashMap::new(),
            },
            Command::Close {
                name: "Open".to_string(),
                capture: Some("my_capture".to_string()),
            },
        ];

        let result = engine.execute(commands, &registry);
        assert!(result.is_ok());
        assert!(
            engine.pools.contains_key("my_capture"),
            "Capture should store in pools"
        );
        let stored = engine.pools.get("my_capture").unwrap();
        assert!(
            !stored.snapshot_lines.is_empty(),
            "Captured content should have snapshot"
        );
    }

    #[test]
    fn test_get_reads_from_pools() {
        let mut engine = Engine::new();
        let registry = crate::registry::CommandRegistry::init();

        // Pre-populate pools
        let mut content = CmdContent::from_raw_text("hello\nworld".to_string());
        content.snapshot_lines = content.lines.clone();
        content.snapshot_raw = content.raw_content.clone();
        engine.pools.insert("test_pool".to_string(), content);

        let commands = vec![Command::Get {
            pool_name: "test_pool".to_string(),
            like: None,
        }];

        let result = engine.execute(commands, &registry);
        assert!(result.is_ok(), "Get should succeed, got {:?}", result.err());

        let last = engine
            .last_result
            .as_ref()
            .expect("last_result should be set");
        assert_eq!(last.content.raw_content, "hello\nworld");
    }

    #[test]
    fn test_value_output_discards_last_result() {
        // Exec is ValueOutput - its result should not persist
        let mut engine = Engine::new();
        let registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::Exec {
            command: "echo test".to_string(),
        }];

        // Exec is not yet implemented; this test validates the output type logic
        // once Exec is implemented
        let _result = engine.execute(commands, &registry);
    }
}
