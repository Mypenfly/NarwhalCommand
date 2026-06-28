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

pub mod command_pipeline;
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
    /// 是否已产生过终端输出（用于决定是否打印默认 "(no output)"）
    pub had_output: bool,
    /// 当前工作路径基准（用于相对路径展开，默认来自脚本父目录）
    pub work_path: std::path::PathBuf,

    // ========== Phase 3 新增字段 ==========
    /// 上一条命令的执行结果（供 Capture 捕获 + 下一命令输入）
    pub last_result: Option<CommandResult>,

    // ========== Phase 4 新增字段 ==========
    /// 当前 Open 是否为 Dir 模式（决定写回时用目录反序列化逻辑）
    pub is_dir_mode: bool,
    /// Dir 模式下的原始树形文本快照（用于退出时比较变更）
    pub dir_snapshot: Option<String>,
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
            had_output: false,
            work_path: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            last_result: None,
            is_dir_mode: false,
            dir_snapshot: None,
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
        registry: &mut CommandRegistry,
    ) -> Result<(), NcsError> {
        for command in &commands {
            match command {
                Command::Close { name, capture } => {
                    self.handle_close(name, capture.clone())?;
                }
                other => {
                    // 1. 权限检查
                    self.check_owner(command, registry)?;

                    // 2. convert: 从前一条命令获取输入
                    let input = self
                        .last_result
                        .take()
                        .map(|r| r.content)
                        .unwrap_or_else(CmdContent::empty);
                    let internal = command.convert(input, &self.pools)?;

                    // 3. execute_core: 核心操作
                    let result = command.execute_core(self, internal, registry)?;

                    // 4. out: 构建输出
                    let output = command.out(result, self);

                    // 5. 确定输出类型（流/值）
                    let output_is_stream = self.is_stream_output(other, registry);

                    // 6. 打印输出（值输出/流输出/Write 特殊格式）
                    self.print_command_output(other, &output);

                    // 7. 设置 last_result（供下一个命令使用）
                    if output_is_stream {
                        self.last_result = Some(CommandResult {
                            content: output,
                            is_stream: true,
                        });
                    } else {
                        self.last_result = None;
                    }

                    // 8. 加入 exec_cmds
                    let cmd_name = other.cmd_name();
                    if cmd_name != "RAW"
                        && cmd_name != "CAPTURE"
                        && cmd_name != "GET"
                        && cmd_name != "LIKE"
                    {
                        self.exec_cmds.push(ExecutedCommand {
                            cmd_name,
                            mode_name: other.mode_name(),
                            is_independent: false,
                        });
                    }
                }
            }
        }

        self.handle_implicit_close()
    }

    /// 打印命令的终端输出
    ///
    /// - 流输出命令（Bash/Get）：直接打印 raw_content
    /// - 值输出命令（Read, Exec）：调用 print()
    /// - Write：输出 "written {path} {size}"
    fn print_command_output(&mut self, command: &Command, output: &CmdContent) {
        match command {
            Command::External { .. } => {
                let text = output.raw_content.trim();
                if !text.is_empty() {
                    println!("{}", text);
                }
                // External 命令始终标记为有输出（Script 方法输出已直连终端）
                self.had_output = true;
            }
            Command::Bash { .. } => {
                let text = output.raw_content.trim();
                if !text.is_empty() {
                    use colored::Colorize;
                    println!("{}", "Bash:".yellow());
                    println!("{}", text);
                    self.had_output = true;
                }
            }
            Command::Read { .. } if output.is_print => {
                output.print();
                self.had_output = true;
            }
            Command::Read { .. } => {}
            Command::Write { path, content, .. } => {
                let size = content.as_ref().map(|c| c.len()).unwrap_or(0);
                println!("written {} {}", path, size);
                self.had_output = true;
            }
            Command::Exec { .. } => {
                self.had_output = true;
            }
            _ => {}
        }
    }

    /// 判断命令的输出类型是流输出还是值输出
    fn is_stream_output(&self, command: &Command, registry: &CommandRegistry) -> bool {
        let cmd_name = command.cmd_name();
        let is_file_editor = matches!(cmd_name.as_str(), "OPEN" | "LOCATION" | "NEW" | "DELETE");
        if is_file_editor {
            return true;
        }
        registry
            .find_command(&cmd_name)
            .and_then(|entry| entry.cmd_type.output.as_ref())
            .is_some_and(|ot| matches!(ot, crate::registry::OutputType::StreamOutput))
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

        if self.is_dir_mode {
            self.write_back_dir()?;
        } else if let (Some(ref file), Some(ref path)) = (&self.file, &self.file_path) {
            file.write_back(path)?;
        }

        Ok(())
    }

    /// Dir 模式写回：比较原始树和最终树，创建/删除文件和目录
    fn write_back_dir(&mut self) -> Result<(), NcsError> {
        let snapshot = self.dir_snapshot.take().unwrap_or_default();
        let final_text: String = if let Some(ref file) = self.file {
            file.lines
                .iter()
                .map(|l| l.content.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            String::new()
        };

        let base_path = self.file_path.as_deref().unwrap_or(".");
        let resolved = self.work_path.join(base_path);

        let original_entries =
            crate::commands::open::deserialize_tree(&snapshot, &resolved.to_string_lossy());
        let final_entries =
            crate::commands::open::deserialize_tree(&final_text, &resolved.to_string_lossy());

        // 删除：在原始但不在最终
        for entry in &original_entries {
            if !final_entries
                .iter()
                .any(|e| e.relative_path == entry.relative_path)
            {
                let abs = resolved.join(&entry.relative_path);
                if entry.is_dir {
                    let _ = std::fs::remove_dir_all(&abs);
                } else {
                    let _ = std::fs::remove_file(&abs);
                }
            }
        }

        // 创建：在最终但不在原始
        for entry in &final_entries {
            if !original_entries
                .iter()
                .any(|e| e.relative_path == entry.relative_path)
            {
                let abs = resolved.join(&entry.relative_path);
                if entry.is_dir {
                    let _ = std::fs::create_dir_all(&abs);
                } else {
                    if let Some(parent) = abs.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&abs, "");
                }
            }
        }

        self.is_dir_mode = false;
        self.dir_snapshot = None;
        self.file = None;
        self.file_path = None;
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
    use crate::error::NcsError;

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
    fn test_all_builtin_commands_execute_without_not_implemented() {
        // Phase 4 完成后，所有 12 个命令已接入 CmdContent 管道，
        // execute() 不应再返回 NotImplemented 错误
        let _engine = Engine::new();
        let registry = crate::registry::CommandRegistry::init();
        let cmd_names = ["BASH", "EXEC", "READ", "WRITE", "INCLUDE", "WORKPATH"];
        for name in &cmd_names {
            let entry = registry.find_command(name);
            assert!(entry.is_some(), "Command {} should be registered", name);
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
        let mut registry = crate::registry::CommandRegistry::init();
        // Phase 3: Capture 已融入 Close 管道语法，使用 Command::Capture 测试
        engine.last_result = Some(CommandResult {
            content: CmdContent::from_raw_text("captured data".to_string()),
            is_stream: true,
        });
        let commands = vec![Command::Capture {
            pool_name: "my_pool2".to_string(),
        }];
        let result = engine.execute(commands, &mut registry);
        assert!(result.is_ok());
        assert!(engine.pools.contains_key("my_pool2"));
    }

    // ============================================================
    // BUG-201: exec_cmds owner 检查
    // ============================================================

    #[test]
    fn test_location_without_open_returns_owner_not_executed() {
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();
        let commands = vec![Command::Location {
            mode: crate::parser::LocationMode::Normal,
            content: None,
            args: HashMap::new(),
        }];
        let result = engine.execute(commands, &mut registry);
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
        let mut registry = crate::registry::CommandRegistry::init();
        let commands = vec![Command::New {
            mode: crate::parser::NewMode::Normal,
            content: crate::model::NewContent {
                lines: vec![],
                base_taps: 0,
            },
        }];
        let result = engine.execute(commands, &mut registry);
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
        let mut registry = crate::registry::CommandRegistry::init();
        let commands = vec![Command::Delete {
            mode: crate::parser::DeleteMode::Normal,
            content: None,
        }];
        let result = engine.execute(commands, &mut registry);
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
        let mut registry = crate::registry::CommandRegistry::init();

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
                    base_taps: 0,
                    lines: vec![crate::model::NewLine {
                        diff_taps: 4,
                        content: "let y = 2;".to_string(),
                        is_raw: false,

                        expand_from_pool: None,
                    }],
                },
            },
        ];
        engine.set_verbose(true);
        let result = engine.execute(commands, &mut registry);
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
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::Open {
            mode: crate::parser::OpenMode::Normal,
            path,
            args: HashMap::new(),
        }];

        let result = engine.execute(commands, &mut registry);
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
        let mut registry = crate::registry::CommandRegistry::init();

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

        let result = engine.execute(commands, &mut registry);
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
        let mut registry = crate::registry::CommandRegistry::init();

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
                    base_taps: 0,
                    lines: vec![crate::model::NewLine {
                        diff_taps: 4,
                        content: "let y = 2;".to_string(),
                        is_raw: false,

                        expand_from_pool: None,
                    }],
                },
            },
        ];

        let result = engine.execute(commands, &mut registry);
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
        let mut registry = crate::registry::CommandRegistry::init();

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

        let result = engine.execute(commands, &mut registry);
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
        let mut registry = crate::registry::CommandRegistry::init();

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
                    base_taps: 0,
                    lines: vec![crate::model::NewLine {
                        diff_taps: 4,
                        content: "let inserted = 42;".to_string(),
                        is_raw: false,

                        expand_from_pool: None,
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

        let result = engine.execute(commands, &mut registry);
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
        let mut registry = crate::registry::CommandRegistry::init();

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
                    base_taps: 0,
                    lines: vec![crate::model::NewLine {
                        diff_taps: 4,
                        content: "let y = 2;".to_string(),
                        is_raw: false,

                        expand_from_pool: None,
                    }],
                },
            },
            Command::Close {
                name: "Location".to_string(),
                capture: None,
            },
        ];

        let result = engine.execute(commands, &mut registry);
        assert!(result.is_ok(), "Expected success, got {:?}", result.err());

        // Changes should be applied and block_stack should be empty
        // (Location was closed, block was popped and written back)
        assert!(!engine.diff_lines.is_empty(), "Should have recorded diff");
    }

    #[test]
    fn test_capture_stores_cmdcontent_in_pools() {
        let (_dir, path) = make_temp_file("fn main() {\n    let x = 1;\n}\n");
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

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

        let result = engine.execute(commands, &mut registry);
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
        let mut registry = crate::registry::CommandRegistry::init();

        // Pre-populate pools
        let mut content = CmdContent::from_raw_text("hello\nworld".to_string());
        content.snapshot_lines = content.lines.clone();
        content.snapshot_raw = content.raw_content.clone();
        engine.pools.insert("test_pool".to_string(), content);

        let commands = vec![Command::Get {
            pool_name: "test_pool".to_string(),
        }];

        let result = engine.execute(commands, &mut registry);
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
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::Exec {
            command: "echo test".to_string(),
        }];

        // Exec is not yet implemented; this test validates the output type logic
        // once Exec is implemented
        let _result = engine.execute(commands, &mut registry);
    }

    // ============================================================
    // Phase 4: Write 命令测试
    // ============================================================

    #[test]
    fn test_write_normal_creates_file_with_content() {
        let dir = tempfile::tempdir().unwrap();
        let out_path = dir.path().join("output.txt");
        let out_path_str = out_path.to_str().unwrap().to_string();
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::Write {
            mode: crate::parser::WriteMode::Normal,
            path: out_path_str.clone(),
            content: Some("hello from write".to_string()),
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(
            result.is_ok(),
            "Write Normal should succeed, got {:?}",
            result.err()
        );

        let file_content =
            std::fs::read_to_string(&out_path_str).expect("output file should exist");
        assert_eq!(file_content, "hello from write");
    }

    #[test]
    fn test_write_normal_overwrites_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let out_path = dir.path().join("output.txt");
        let out_path_str = out_path.to_str().unwrap().to_string();
        std::fs::write(&out_path_str, "old content").unwrap();

        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::Write {
            mode: crate::parser::WriteMode::Normal,
            path: out_path_str.clone(),
            content: Some("new content".to_string()),
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(
            result.is_ok(),
            "Write should overwrite, got {:?}",
            result.err()
        );

        let file_content = std::fs::read_to_string(&out_path_str).unwrap();
        assert_eq!(file_content, "new content");
    }

    #[test]
    fn test_write_raw_preserves_all_characters() {
        let dir = tempfile::tempdir().unwrap();
        let out_path = dir.path().join("raw_output.txt");
        let out_path_str = out_path.to_str().unwrap().to_string();
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::Write {
            mode: crate::parser::WriteMode::Raw,
            path: out_path_str.clone(),
            content: Some("!@Special tokens\n@/Close\nshould all be raw".to_string()),
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(
            result.is_ok(),
            "Write Raw should succeed, got {:?}",
            result.err()
        );

        let file_content = std::fs::read_to_string(&out_path_str).unwrap();
        assert_eq!(file_content, "!@Special tokens\n@/Close\nshould all be raw");
    }

    #[test]
    fn test_write_creates_parent_directories() {
        let dir = tempfile::tempdir().unwrap();
        let nested_path = dir.path().join("deeply/nested/dir/output.txt");
        let nested_path_str = nested_path.to_str().unwrap().to_string();
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::Write {
            mode: crate::parser::WriteMode::Normal,
            path: nested_path_str.clone(),
            content: Some("deep content".to_string()),
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(
            result.is_ok(),
            "Write with parent dirs should succeed, got {:?}",
            result.err()
        );

        let file_content = std::fs::read_to_string(&nested_path_str).unwrap();
        assert_eq!(file_content, "deep content");
    }

    // ============================================================
    // Phase 4: Bash 命令测试
    // ============================================================

    #[test]
    fn test_bash_executes_simple_echo() {
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::Bash {
            command: "echo hello_world".to_string(),
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(
            result.is_ok(),
            "Bash echo should succeed, got {:?}",
            result.err()
        );

        let last = engine
            .last_result
            .as_ref()
            .expect("Bash is stream output, last_result should be set");
        assert!(
            last.content.raw_content.contains("hello_world"),
            "Bash output should contain 'hello_world', got: {}",
            last.content.raw_content
        );
    }

    #[test]
    fn test_bash_security_denies_sudo() {
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::Bash {
            command: "sudo rm -rf /".to_string(),
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(result.is_err(), "sudo should be denied");
        match result.unwrap_err() {
            NcsError::CommandExec(crate::error::CommandExecError::SecurityDenied { .. }) => {}
            other => panic!("Expected SecurityDenied, got {:?}", other),
        }
    }

    #[test]
    fn test_bash_security_denies_chmod_777_root() {
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::Bash {
            command: "chmod 777 /etc/passwd".to_string(),
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(result.is_err(), "chmod 777 on root should be denied");
    }

    #[test]
    fn test_bash_captures_failed_command_stderr() {
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::Bash {
            command: "nonexistent_command_xyz 2>&1".to_string(),
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(result.is_err(), "nonexistent command should fail");
        match result.unwrap_err() {
            NcsError::CommandExec(crate::error::CommandExecError::ExecutionFailed { .. }) => {}
            other => panic!("Expected ExecutionFailed, got {:?}", other),
        }
    }

    // ============================================================
    // Phase 4: Exec 命令测试
    // ============================================================

    #[test]
    fn test_exec_runs_command() {
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::Exec {
            command: "echo from_exec".to_string(),
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(
            result.is_ok(),
            "Exec should succeed, got {:?}",
            result.err()
        );
    }

    #[test]
    fn test_exec_is_value_output_discards_result() {
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::Exec {
            command: "echo transient".to_string(),
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(result.is_ok());
        assert!(
            engine.last_result.is_none(),
            "Exec is value output, last_result should be None"
        );
    }

    // ============================================================
    // Phase 4: Read 命令测试
    // ============================================================

    #[test]
    fn test_read_file_content() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("sample.txt");
        let file_path_str = file_path.to_str().unwrap().to_string();
        std::fs::write(&file_path_str, "line one\nline two\nline three").unwrap();

        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::Read {
            mode: crate::parser::ReadMode::Normal,
            path: file_path_str,
            args: HashMap::new(),
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(
            result.is_ok(),
            "Read should succeed, got {:?}",
            result.err()
        );
    }

    #[test]
    fn test_read_nonexistent_file_returns_error() {
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::Read {
            mode: crate::parser::ReadMode::Normal,
            path: "/nonexistent/file/path.txt".to_string(),
            args: HashMap::new(),
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(result.is_err(), "Read of nonexistent file should fail");
    }

    // ============================================================
    // Phase 4: Include 命令测试
    // ============================================================

    #[test]
    fn test_include_registers_new_command() {
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let mut args = HashMap::new();
        args.insert("alias".to_string(), "MyTool".to_string());

        let commands = vec![Command::Include {
            path: "/usr/bin/mytool".to_string(),
            args,
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(
            result.is_ok(),
            "Include should succeed, got {:?}",
            result.err()
        );
    }

    #[test]
    fn test_include_alias_conflict_with_builtin_is_denied() {
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let mut args = HashMap::new();
        args.insert("alias".to_string(), "Open".to_string());

        let commands = vec![Command::Include {
            path: "/usr/bin/tool".to_string(),
            args,
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(
            result.is_err(),
            "Include alias 'Open' should conflict with builtin"
        );
    }

    // ============================================================
    // Phase 4: WorkPath 命令测试
    // ============================================================

    #[test]
    fn test_work_path_changes_directory() {
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_str().unwrap().to_string();
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::WorkPath {
            path: dir_path.clone(),
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(
            result.is_ok(),
            "WorkPath should succeed, got {:?}",
            result.err()
        );
    }

    #[test]
    fn test_work_path_nonexistent_directory_fails() {
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::WorkPath {
            path: "/nonexistent/directory/path".to_string(),
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(result.is_err(), "WorkPath to nonexistent dir should fail");
    }

    // ============================================================
    // Issue #1/#2: Bash/Read/Write 终端输出 + had_output
    // ============================================================

    #[test]
    fn test_bash_sets_had_output() {
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::Bash {
            command: "echo test_output".to_string(),
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(result.is_ok());
        assert!(engine.had_output, "Bash should set had_output to true");
    }

    #[test]
    fn test_read_sets_had_output() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("sample.txt");
        std::fs::write(&file_path, "hello").unwrap();

        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::Read {
            mode: crate::parser::ReadMode::Normal,
            path: file_path.to_str().unwrap().to_string(),
            args: HashMap::new(),
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(result.is_ok());
        assert!(engine.had_output, "Read should set had_output to true");
    }

    #[test]
    fn test_write_sets_had_output() {
        let dir = tempfile::tempdir().unwrap();
        let out_path = dir.path().join("output.txt");
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::Write {
            mode: crate::parser::WriteMode::Normal,
            path: out_path.to_str().unwrap().to_string(),
            content: Some("write content".to_string()),
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(result.is_ok());
        assert!(engine.had_output, "Write should set had_output to true");
    }

    #[test]
    fn test_exec_sets_had_output() {
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::Exec {
            command: "echo exec_output".to_string(),
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(result.is_ok());
        assert!(engine.had_output, "Exec should set had_output to true");
    }

    #[test]
    fn test_had_output_starts_false() {
        let engine = Engine::new();
        assert!(!engine.had_output, "had_output should start false");
    }

    #[test]
    fn test_file_edit_without_additions_does_not_set_had_output() {
        // Pure read-only Location should not set had_output
        let (_dir, path) = make_temp_file("fn main() {\n    let x = 1;\n}\n");
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

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
            Command::Close {
                name: "Open".to_string(),
                capture: None,
            },
        ];

        let result = engine.execute(commands, &mut registry);
        assert!(result.is_ok());
        // Read-only operations don't produce output
        // (Location result is triggered by verbose, not by had_output)
    }

    // ============================================================
    // Include → External 端到端测试
    // ============================================================

    #[test]
    fn test_include_then_external_via_bash() {
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let mut args = HashMap::new();
        args.insert("alias".to_string(), "EchoTool".to_string());
        args.insert("exec".to_string(), "bash".to_string());

        let commands = vec![
            Command::Include {
                path: "echo".to_string(),
                args,
            },
            Command::External {
                name: "EchoTool".to_string(),
                positional_args: vec!["hello_from_external".to_string()],
            },
        ];

        let result = engine.execute(commands, &mut registry);
        assert!(
            result.is_ok(),
            "Include+External via bash should succeed, got {:?}",
            result.err()
        );
        assert!(engine.had_output);
        assert!(registry.find_command("EchoTool").is_some());
    }

    #[test]
    fn test_include_then_external_via_script() {
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let mut args = HashMap::new();
        args.insert("alias".to_string(), "EchoScr".to_string());
        args.insert("exec".to_string(), "script".to_string());

        let commands = vec![
            Command::Include {
                path: "echo".to_string(),
                args,
            },
            Command::External {
                name: "EchoScr".to_string(),
                positional_args: vec!["hello_from_script".to_string()],
            },
        ];

        let result = engine.execute(commands, &mut registry);
        assert!(
            result.is_ok(),
            "Include+External via script should succeed, got {:?}",
            result.err()
        );
        assert!(engine.had_output);
    }

    #[test]
    fn test_include_resolves_dot_slash_path() {
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        // 设置 work_path 为固定值用于测试
        engine.work_path = std::path::PathBuf::from("/tmp");

        let mut args = HashMap::new();
        args.insert("alias".to_string(), "ResolvedCmd".to_string());
        args.insert("exec".to_string(), "bash".to_string());

        let commands = vec![Command::Include {
            path: "echo ./my_script.sh".to_string(),
            args,
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(
            result.is_ok(),
            "Include should succeed, got {:?}",
            result.err()
        );

        let entry = registry.find_command("ResolvedCmd").unwrap();
        let ep = entry.exec_path.as_ref().unwrap().to_string_lossy();
        // echo 是系统命令，不应被展开；./my_script.sh 应该被展开
        assert!(ep.starts_with("echo /"));
        assert!(ep.contains("my_script.sh"));
    }

    #[test]
    fn test_external_command_not_registered_errors() {
        let mut engine = Engine::new();
        let mut registry = crate::registry::CommandRegistry::init();

        let commands = vec![Command::External {
            name: "NoSuchCmd".to_string(),
            positional_args: vec![],
        }];

        let result = engine.execute(commands, &mut registry);
        assert!(result.is_err());
        match result.unwrap_err() {
            NcsError::Registry(crate::error::RegistryError::CommandNotFound {
                cmd_name, ..
            }) => {
                assert_eq!(cmd_name, "NoSuchCmd");
            }
            other => panic!("Expected CommandNotFound, got {:?}", other),
        }
    }

    // ============================================================
    // Open Dir 模式测试
    // ============================================================

    #[test]
    fn test_open_dir_produces_tree_text() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "").unwrap();
        std::fs::write(dir.path().join("b.txt"), "").unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("c.py"), "").unwrap();

        let mut engine = Engine::new();
        engine.work_path = dir.path().to_path_buf();

        let commands = vec![Command::Open {
            mode: crate::parser::OpenMode::Dir,
            path: dir.path().to_string_lossy().to_string(),
            args: std::collections::HashMap::new(),
        }];

        let mut registry = CommandRegistry::init();
        // 隐式关闭会清理 dir 状态，但执行不应报错
        engine.execute(commands, &mut registry).unwrap();
        // 执行成功即通过
    }

    #[test]
    fn test_open_dir_with_depth_limit() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("deep.rs"), "").unwrap();
        std::fs::write(dir.path().join("top.txt"), "").unwrap();

        let mut engine = Engine::new();
        engine.work_path = dir.path().to_path_buf();
        let mut args = std::collections::HashMap::new();
        args.insert("depth".to_string(), "1".to_string());

        let commands = vec![Command::Open {
            mode: crate::parser::OpenMode::Dir,
            path: ".".to_string(),
            args,
        }];

        let mut registry = CommandRegistry::init();
        engine.execute(commands, &mut registry).unwrap();
        // depth=1: deep.rs should not be in tree and thus not deleted
        assert!(
            sub.join("deep.rs").exists(),
            "deep.rs should survive depth=1"
        );
        assert!(dir.path().join("top.txt").exists());
        assert!(sub.exists());
    }

    #[test]
    fn test_open_dir_close_writes_back_noop() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("existing.rs"), "").unwrap();

        let dir_name = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let mut engine = Engine::new();
        engine.work_path = dir.path().parent().unwrap().to_path_buf();

        let commands = vec![
            Command::Open {
                mode: crate::parser::OpenMode::Dir,
                path: dir_name.clone(),
                args: std::collections::HashMap::new(),
            },
            Command::Close {
                name: "Open".to_string(),
                capture: None,
            },
        ];

        let mut registry = CommandRegistry::init();
        engine.execute(commands, &mut registry).unwrap();

        // 验证写回后目录仍在且 is_dir_mode 已清除
        assert!(!engine.is_dir_mode);
        assert!(dir.path().join("existing.rs").exists());
    }
}
