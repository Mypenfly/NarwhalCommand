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
    pub fn execute(
        &mut self,
        commands: Vec<Command>,
        registry: &CommandRegistry,
    ) -> Result<(), NcsError> {
        for command in &commands {
            // 前置检查：确认 owner 存在
            self.check_owner(command, registry)?;

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
                Command::Capture { pool_name } => {
                    // 将上一条命令的输出存入 pools
                    // Phase 3+: 从 CommandResult 中获取实际的 CmdContent
                    let content = CmdContent::default();
                    self.pools.insert(pool_name.clone(), content);
                }
                // Phase 3+: Bash, Exec, Read, Write, Include, WorkPath, Get
                Command::Bash { .. } => {
                    return Err(NcsError::Engine(
                        crate::error::EngineError::NotImplemented {
                            feature: "Bash 命令".to_string(),
                        },
                    ));
                }
                Command::Exec { .. } => {
                    return Err(NcsError::Engine(
                        crate::error::EngineError::NotImplemented {
                            feature: "Exec 命令".to_string(),
                        },
                    ));
                }
                Command::Read { .. } => {
                    return Err(NcsError::Engine(
                        crate::error::EngineError::NotImplemented {
                            feature: "Read 命令".to_string(),
                        },
                    ));
                }
                Command::Write { .. } => {
                    return Err(NcsError::Engine(
                        crate::error::EngineError::NotImplemented {
                            feature: "Write 命令".to_string(),
                        },
                    ));
                }
                Command::Include { .. } => {
                    return Err(NcsError::Engine(
                        crate::error::EngineError::NotImplemented {
                            feature: "Include 命令".to_string(),
                        },
                    ));
                }
                Command::WorkPath { .. } => {
                    return Err(NcsError::Engine(
                        crate::error::EngineError::NotImplemented {
                            feature: "WorkPath 命令".to_string(),
                        },
                    ));
                }
                Command::Get { .. } => {
                    return Err(NcsError::Engine(
                        crate::error::EngineError::NotImplemented {
                            feature: "Get 命令".to_string(),
                        },
                    ));
                }
            }
        }

        self.handle_implicit_close()
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
        let commands = vec![Command::Capture {
            pool_name: "my_pool".to_string(),
        }];
        let result = engine.execute(commands, &registry);
        assert!(result.is_ok());
        assert!(engine.pools.contains_key("my_pool"));
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
}
