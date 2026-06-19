//! Command 三步流水线（execute_core / out）
//!
//! 从 engine/mod.rs 拆分出的 Command 执行逻辑，
//! 包含 execute_core（核心执行）和 out（输出格式化）。
//!
//! ## 实现逻辑
//!
//! 1. execute_core: 将 convert 阶段准备的 pending 数据执行实际操作
//! 2. 大命令臂（New/Delete/External）拆分为独立方法
//! 3. out: 将 execute_core 的结果格式化为下游可用的 CmdContent
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §5 "命令定义", INSTRUCTION.md §1.3 "命令状态机"

use crate::cmd_content::{CmdContent, CmdLine};
use crate::error::NcsError;
use crate::model::{DeleteContent, Line};
use crate::output::DiffLine;
use crate::parser::{Command, DeleteMode, NewMode};
use crate::registry::CommandRegistry;

use super::Engine;

impl Command {
    /// 第二阶段：执行核心逻辑
    ///
    /// 从 `internal` 读取 convert 阶段准备的 pending 数据，
    /// 执行实际操作（文件读写、匹配定位、变更记录），
    /// 引擎状态变更通过 `engine` 传递。
    pub fn execute_core(
        &self,
        engine: &mut Engine,
        mut internal: CmdContent,
        registry: &mut CommandRegistry,
    ) -> Result<CmdContent, NcsError> {
        match self {
            Command::Open { mode, path, args } => {
                crate::commands::open::execute(engine, *mode, path, args)?;
                Ok(internal)
            }
            Command::Location {
                mode,
                content,
                args,
            } => {
                crate::commands::location::execute(engine, *mode, content.clone(), args)?;
                Ok(internal)
            }
            Command::New { mode, .. } => {
                Self::execute_new_core(*mode, engine, &mut internal)?;
                Ok(internal)
            }
            Command::Delete {
                mode,
                content: del_content,
            } => {
                Self::execute_delete_core(*mode, del_content, engine, &mut internal)?;
                Ok(internal)
            }
            Command::Write {
                mode,
                path,
                content,
            } => {
                crate::commands::write::execute(*mode, path, content.as_deref())?;
                Ok(internal)
            }
            Command::Bash { command } => {
                let content = crate::commands::bash::execute(command)?;
                Ok(content)
            }
            Command::Exec { command } => {
                crate::commands::exec::execute(command)?;
                Ok(internal)
            }
            Command::Read { mode, path, args } => {
                let content = crate::commands::read::execute(engine, *mode, path, args)?;
                Ok(content)
            }
            Command::Include { path, args } => {
                crate::commands::include::execute(path, args, registry, &engine.work_path)?;
                Ok(internal)
            }
            Command::WorkPath { path } => {
                Self::execute_workpath_core(path, engine)?;
                Ok(internal)
            }
            Command::Raw { .. } => Ok(internal),
            Command::Capture { pool_name } => {
                let content = engine
                    .last_result
                    .take()
                    .map(|r| r.content)
                    .unwrap_or_default();
                engine.pools.insert(pool_name.clone(), content);
                Ok(internal)
            }
            Command::Get { pool_name, .. } => crate::commands::get::execute(engine, pool_name),
            Command::External {
                name,
                positional_args,
            } => Self::execute_external_core(name, positional_args, registry),
            _ => Err(NcsError::Engine(
                crate::error::EngineError::NotImplemented {
                    feature: self.cmd_name(),
                },
            )),
        }
    }

    /// 第三阶段：将 execute_core 的结果格式化为下游可用的 CmdContent
    ///
    /// Open/Location 从 engine 状态构建带快照的 CmdContent，
    /// New/Delete 直接透传 (changes 已在 execute_core 中记录)。
    pub fn out(&self, result: CmdContent, engine: &Engine) -> CmdContent {
        match self {
            Command::Open { .. } => Self::build_open_output(engine),
            Command::Location { .. } => Self::build_location_output(engine),
            Command::Write { .. } => CmdContent::empty(),
            Command::Exec { .. } => CmdContent::empty(),
            Command::Read { .. } => result,
            Command::Include { .. } => CmdContent::empty(),
            Command::WorkPath { .. } => CmdContent::empty(),
            Command::Bash { .. } => result,
            Command::Get { .. } => result,
            _ => result,
        }
    }
}

// ============================================================
// execute_core 子方法
// ============================================================

impl Command {
    /// New 命令核心：Start/End 直接修改文件，Normal 使用变更追踪
    fn execute_new_core(
        mode: NewMode,
        engine: &mut Engine,
        internal: &mut CmdContent,
    ) -> Result<(), NcsError> {
        let new_lines = internal.pending_new_lines.take().ok_or_else(|| {
            NcsError::Engine(crate::error::EngineError::NotImplemented {
                feature: "New execute_core: missing pending_new_lines".to_string(),
            })
        })?;

        match mode {
            NewMode::Start => Self::execute_new_start(engine, new_lines),
            NewMode::End => Self::execute_new_end(engine, new_lines),
            NewMode::Normal => Self::execute_new_normal(engine, internal, new_lines),
        }
    }

    /// New::Start: 直接在文件级别插入开头
    fn execute_new_start(engine: &mut Engine, new_lines: Vec<CmdLine>) -> Result<(), NcsError> {
        let file =
            engine
                .file
                .as_mut()
                .ok_or(NcsError::File(crate::error::FileError::NotFound {
                    path: "(no file opened)".to_string(),
                }))?;
        let insert_pos = 0;
        let new_line_count = new_lines.len();
        let tail = std::mem::take(&mut file.lines);
        let mut combined: Vec<Line> = new_lines
            .iter()
            .map(|cl| Line {
                line_num: crate::model::LineNumber::new(0),
                taps: crate::model::count_leading_spaces(&cl.content),
                diff_taps: 0,
                content: cl.content.clone(),
                stripped_content: crate::model::stripped_content(&cl.content),
            })
            .collect();
        combined.extend(tail);
        file.lines = combined;
        super::executor::reindex_file(file);

        let added_entries =
            super::executor::collect_new_file_line_info(file, insert_pos, new_line_count);
        engine.record_added_lines(added_entries);
        Ok(())
    }

    /// New::End: 直接在文件级别插入末尾
    fn execute_new_end(engine: &mut Engine, new_lines: Vec<CmdLine>) -> Result<(), NcsError> {
        let file =
            engine
                .file
                .as_mut()
                .ok_or(NcsError::File(crate::error::FileError::NotFound {
                    path: "(no file opened)".to_string(),
                }))?;
        let insert_start = file.lines.len();
        let new_line_count = new_lines.len();
        let new_file_lines: Vec<Line> = new_lines
            .iter()
            .map(|cl| Line {
                line_num: crate::model::LineNumber::new(0),
                taps: crate::model::count_leading_spaces(&cl.content),
                diff_taps: 0,
                content: cl.content.clone(),
                stripped_content: crate::model::stripped_content(&cl.content),
            })
            .collect();
        file.lines.extend(new_file_lines);
        super::executor::reindex_file(file);

        let added_entries =
            super::executor::collect_new_file_line_info(file, insert_start, new_line_count);
        engine.record_added_lines(added_entries);
        Ok(())
    }

    /// New::Normal: 在 Location 匹配位置之后插入（变更追踪模式）
    fn execute_new_normal(
        engine: &mut Engine,
        internal: &mut CmdContent,
        new_lines: Vec<CmdLine>,
    ) -> Result<(), NcsError> {
        let insert_pos = engine
            .block_stack
            .last()
            .map(|b| match &b.match_info {
                crate::model::MatchInfo::Location { matched_line_count } => *matched_line_count,
                crate::model::MatchInfo::DeleteAt { position } => *position,
                crate::model::MatchInfo::Empty => b.lines.len(),
            })
            .unwrap_or(0);

        let after_line = insert_pos.saturating_sub(1);

        if let Some(block) = engine.block_stack.last() {
            let actual_insert = insert_pos.min(block.lines.len());
            let context_above = super::executor::collect_block_context_above(block, actual_insert);
            let context_below = super::executor::collect_block_context_below(
                block,
                actual_insert.saturating_sub(1),
            );
            let changed: Vec<DiffLine> = new_lines
                .iter()
                .enumerate()
                .map(|(i, l)| DiffLine {
                    kind: crate::output::DiffLineKind::Added,
                    line_number: Some(crate::model::LineNumber::new(
                        block.start_line.to_usize() + actual_insert + i,
                    )),
                    content: l.content.clone(),
                })
                .collect();
            engine.record_diff_with_context(changed, context_above, context_below);
        }

        internal.record_insert(after_line, new_lines, "NEW");
        Ok(())
    }

    /// Delete 命令核心：Block 删除整个块，Normal 在 snapshot 中匹配删除
    fn execute_delete_core(
        mode: DeleteMode,
        del_content: &Option<DeleteContent>,
        engine: &mut Engine,
        internal: &mut CmdContent,
    ) -> Result<(), NcsError> {
        match mode {
            DeleteMode::Block => {
                let total = internal.snapshot_lines.len();
                let block = engine.block_stack.last().ok_or(NcsError::Engine(
                    crate::error::EngineError::MissingLocationForNew,
                ))?;
                let (changed, context_above, context_below) =
                    super::executor::collect_deleted_diff_data(block, 0, total.saturating_sub(1));
                internal.record_delete(0, total.saturating_sub(1), "DELETE");
                engine.record_diff_with_context(changed, context_above, context_below);
            }
            DeleteMode::Normal => {
                let del_content = del_content.as_ref().ok_or(NcsError::Engine(
                    crate::error::EngineError::MissingLocationForNew,
                ))?;

                let (s, e) = super::executor::find_delete_match_in_snapshot(
                    &internal.snapshot_lines,
                    del_content,
                )
                .ok_or_else(|| {
                    super::executor::delete_not_found_in_snapshot_error(
                        del_content,
                        &internal.snapshot_lines,
                    )
                })?;

                let block = engine.block_stack.last().ok_or(NcsError::Engine(
                    crate::error::EngineError::MissingLocationForNew,
                ))?;
                let matched_line_count = match &block.match_info {
                    crate::model::MatchInfo::Location { matched_line_count } => *matched_line_count,
                    _ => 0,
                };
                super::executor::check_delete_adjacency_in_snapshot(
                    &internal.snapshot_lines,
                    matched_line_count,
                    s,
                )?;

                let (changed, context_above, context_below) =
                    super::executor::collect_deleted_diff_data(block, s, e);
                engine.record_diff_with_context(changed, context_above, context_below);

                internal.record_delete(s, e, "DELETE");
            }
        }
        Ok(())
    }

    /// WorkPath 命令核心：解析路径并更新工作目录
    fn execute_workpath_core(path: &str, engine: &mut Engine) -> Result<(), NcsError> {
        let target = crate::commands::work_path::resolve(path)?;
        std::env::set_current_dir(&target).map_err(|e| {
            NcsError::File(crate::error::FileError::WriteFailed {
                path: target.display().to_string(),
                reason: e.to_string(),
            })
        })?;
        engine.work_path = target;
        Ok(())
    }

    /// External 命令核心：根据 ExecMethod 执行外部命令
    fn execute_external_core(
        name: &str,
        positional_args: &[String],
        registry: &mut CommandRegistry,
    ) -> Result<CmdContent, NcsError> {
        let entry = registry.find_command(name).ok_or_else(|| {
            NcsError::Registry(crate::error::RegistryError::CommandNotFound {
                cmd_name: name.to_string(),
                line: crate::model::LineNumber::new(0),
                suggestion: None,
            })
        })?;
        let exec_path = entry
            .exec_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| name.to_string());
        let full_command = if positional_args.is_empty() {
            exec_path.clone()
        } else {
            format!("{} {}", exec_path, positional_args.join(" "))
        };

        match entry.exec_method {
            crate::registry::ExecMethod::Bash => crate::commands::bash::execute(&full_command),
            crate::registry::ExecMethod::Script => {
                crate::commands::exec::execute(&full_command)?;
                Ok(CmdContent::empty())
            }
            crate::registry::ExecMethod::Default => {
                Self::execute_external_default(&exec_path, positional_args, &full_command)
            }
        }
    }

    /// External Default 模式：直接进程执行
    fn execute_external_default(
        exec_path: &str,
        positional_args: &[String],
        full_command: &str,
    ) -> Result<CmdContent, NcsError> {
        let mut parts: Vec<&str> = exec_path.split_whitespace().collect();
        let prog = parts.remove(0);
        let mut cmd = std::process::Command::new(prog);
        for part in &parts {
            cmd.arg(part);
        }
        for arg in positional_args {
            cmd.arg(arg);
        }
        let output = cmd.output().map_err(|e| {
            NcsError::CommandExec(crate::error::CommandExecError::ExecutionFailed {
                command: full_command.to_string(),
                exit_code: None,
                stderr: e.to_string(),
            })
        })?;
        if !output.status.success() {
            return Err(NcsError::CommandExec(
                crate::error::CommandExecError::ExecutionFailed {
                    command: full_command.to_string(),
                    exit_code: output.status.code(),
                    stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                },
            ));
        }
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let mut content = CmdContent::from_raw_text(stdout);
        content.source_info = Some(crate::cmd_content::ContentSource::CommandOutput);
        Ok(content)
    }
}

// ============================================================
// out 子方法
// ============================================================

impl Command {
    /// 构建 Open 命令的输出 CmdContent（从 engine.file 构建快照）
    fn build_open_output(engine: &Engine) -> CmdContent {
        if let Some(ref file) = engine.file {
            let cmd_lines: Vec<CmdLine> = file
                .lines
                .iter()
                .map(|l| CmdLine {
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
                file_path: engine.file_path.clone().unwrap_or_default(),
            });
            c
        } else {
            CmdContent::empty()
        }
    }

    /// 构建 Location 命令的输出 CmdContent（从 block_stack 栈顶构建快照）
    fn build_location_output(engine: &Engine) -> CmdContent {
        if let Some(block) = engine.block_stack.last() {
            let block_index = engine.block_stack.len() - 1;
            let cmd_lines: Vec<CmdLine> = block
                .lines
                .iter()
                .map(|l| CmdLine {
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
}
