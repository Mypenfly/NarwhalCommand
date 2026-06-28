//! New 命令
//!
//! `!@New` 在指定位置插入新内容，支持 Normal/Start/End 三种模式。
//!
//! ## 实现逻辑
//!
//! 1. 从 NewContent 构建 Line 列表（build_new_lines）
//! 2. 按模式确定插入位置
//! 3. 插入后 reindex 重排行号和序号
//! 4. 收集 diff 并记录上下文
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §5.3 "New", INSTRUCTION.md §3.3 "New 插入算法"
//!
//! ## 迁移来源
//!
//! 从 n_edit/src/engine.rs 的 execute_new*() 迁移。

use crate::engine::executor;
use crate::engine::Engine;
use crate::error::NcsError;
use crate::model::{MatchInfo, NewContent};
use crate::parser::NewMode;

/// New 命令的执行入口
pub fn execute(engine: &mut Engine, mode: NewMode, content: NewContent) -> Result<(), NcsError> {
    match mode {
        NewMode::Start => execute_start(engine, content),
        NewMode::End => execute_end(engine, content),
        NewMode::Normal => execute_normal(engine, content),
    }
}

/// 在文件/Block 开头插入新内容
///
/// New(Start) 始终在文件级别操作，不考虑 block_stack。
fn execute_start(engine: &mut Engine, content: NewContent) -> Result<(), NcsError> {
    let new_lines = executor::build_new_lines(&content);
    let new_line_count = new_lines.len();

    let file = engine
        .file
        .as_mut()
        .ok_or(NcsError::File(crate::error::FileError::NotFound {
            path: "(no file opened)".to_string(),
        }))?;

    let insert_pos = 0;
    let tail = std::mem::take(&mut file.lines);
    let mut combined = new_lines;
    combined.extend(tail);
    file.lines = combined;
    executor::reindex_file(file);

    let added_entries = executor::collect_new_file_line_info(file, insert_pos, new_line_count);
    engine.record_added_lines(added_entries);

    Ok(())
}

/// 在文件/Block 末尾插入新内容
///
/// New(End) 始终在文件级别操作，不考虑 block_stack。
fn execute_end(engine: &mut Engine, content: NewContent) -> Result<(), NcsError> {
    let new_lines = executor::build_new_lines(&content);
    let new_line_count = new_lines.len();

    let file = engine
        .file
        .as_mut()
        .ok_or(NcsError::File(crate::error::FileError::NotFound {
            path: "(no file opened)".to_string(),
        }))?;

    let insert_start = file.lines.len();
    file.lines.extend(new_lines);
    executor::reindex_file(file);

    let added_entries = executor::collect_new_file_line_info(file, insert_start, new_line_count);
    engine.record_added_lines(added_entries);

    Ok(())
}

/// 在 Location 匹配位置之后插入新内容
fn execute_normal(engine: &mut Engine, content: NewContent) -> Result<(), NcsError> {
    let insert_pos = {
        let block = engine.block_stack.last().ok_or(NcsError::Engine(
            crate::error::EngineError::MissingLocationForNew,
        ))?;
        match &block.match_info {
            MatchInfo::Empty => block.lines.len(),
            MatchInfo::Location { matched_line_count } => *matched_line_count,
            MatchInfo::DeleteAt { position } => *position,
        }
    };

    let new_lines = executor::build_new_lines(&content);
    let new_line_count = new_lines.len();

    // Phase 3 变更追踪：记录 Insert 变更，不再直接修改 block.lines
    if let Some(ref mut result) = engine.last_result {
        let cmd_lines: Vec<crate::cmd_content::CmdLine> = new_lines
            .iter()
            .map(|l| crate::cmd_content::CmdLine {
                line_num: l.line_num.to_usize(),
                content: l.content.clone(),

                expand_from_pool: None,
            })
            .collect();
        let after_line = insert_pos.saturating_sub(1);
        result.content.record_insert(after_line, cmd_lines, "NEW");
    }

    // 从原始 block 收集 diff（上下文来自 block，新增行来自 new_lines）
    let (changed, context_above, context_below) = {
        let block = engine.block_stack.last().ok_or(NcsError::Engine(
            crate::error::EngineError::MissingLocationForNew,
        ))?;
        let actual_insert = insert_pos.min(block.lines.len());
        let context_above = executor::collect_block_context_above(block, actual_insert);
        let context_below =
            executor::collect_block_context_below(block, actual_insert.saturating_sub(1));
        let changed: Vec<crate::output::DiffLine> = new_lines
            .iter()
            .enumerate()
            .map(|(i, l)| crate::output::DiffLine {
                kind: crate::output::DiffLineKind::Added,
                line_number: Some(crate::model::LineNumber::new(
                    block.start_line.to_usize() + actual_insert + i,
                )),
                content: l.content.clone(),
            })
            .collect();

        (changed, context_above, context_below)
    };

    let _ = new_line_count;
    engine.record_diff_with_context(changed, context_above, context_below);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;
    use crate::model::{NewContent, NewLine};
    use crate::parser::{LocationMode, NewMode, OpenMode};
    use std::collections::HashMap;

    /// 创建带临时文件的引擎（已执行 Open）
    fn engine_with_file(content: &str) -> (tempfile::TempDir, Engine) {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.rs");
        std::fs::write(&file_path, content).unwrap();
        let mut engine = Engine::new();
        crate::commands::open::execute(
            &mut engine,
            OpenMode::Normal,
            file_path.to_str().unwrap(),
            &HashMap::new(),
        )
        .unwrap();
        (dir, engine)
    }

    /// 辅助：构建 NewContent
    fn make_new_content(lines: &[(&str, usize, bool)]) -> NewContent {
        NewContent {
            base_taps: 0,
            lines: lines
                .iter()
                .map(|(content, diff_taps, is_raw)| NewLine {
                    diff_taps: *diff_taps,
                    content: content.to_string(),
                    is_raw: *is_raw,

                    expand_from_pool: None,
                })
                .collect(),
        }
    }

    #[test]
    fn test_new_start_inserts_at_beginning() {
        let (_dir, mut engine) = engine_with_file("line1\nline2\nline3\n");
        let content = make_new_content(&[("new_line", 0, false)]);

        let result = execute(&mut engine, NewMode::Start, content);
        assert!(result.is_ok());
        let file = engine.file.as_ref().unwrap();
        assert_eq!(file.lines.len(), 4);
        assert_eq!(file.lines[0].content, "new_line");
        assert_eq!(file.lines[1].content, "line1");
    }

    #[test]
    fn test_new_end_inserts_at_end() {
        let (_dir, mut engine) = engine_with_file("line1\nline2\nline3\n");
        let content = make_new_content(&[("new_line", 0, false)]);

        let result = execute(&mut engine, NewMode::End, content);
        assert!(result.is_ok());
        let file = engine.file.as_ref().unwrap();
        assert_eq!(file.lines.len(), 4);
        assert_eq!(file.lines[3].content, "new_line");
    }

    #[test]
    fn test_new_normal_after_location() {
        let (_dir, mut engine) = engine_with_file("fn foo() {\n    old();\n}\nfn bar() {}\n");

        // Location: find fn foo
        crate::commands::location::execute(
            &mut engine,
            LocationMode::Normal,
            Some(build_location_content(&["fn foo() {"])),
            &HashMap::new(),
        )
        .unwrap();

        // 设置 last_result（模拟 engine.execute 管线）
        let block = engine.block_stack.last().unwrap();
        let cmd_lines: Vec<crate::cmd_content::CmdLine> = block
            .lines
            .iter()
            .map(|l| crate::cmd_content::CmdLine {
                line_num: l.line_num.to_usize(),
                content: l.content.clone(),

                expand_from_pool: None,
            })
            .collect();
        let raw = cmd_lines
            .iter()
            .map(|l| &l.content)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let mut c = crate::cmd_content::CmdContent::empty();
        c.snapshot_lines = cmd_lines.clone();
        c.snapshot_raw = raw.clone();
        c.lines = cmd_lines;
        c.raw_content = raw;
        c.source_info = Some(crate::cmd_content::ContentSource::Block { block_index: 0 });
        engine.last_result = Some(crate::cmd_content::CommandResult {
            content: c,
            is_stream: true,
        });

        let content = make_new_content(&[("new_code();", 4, false)]);
        let result = execute(&mut engine, NewMode::Normal, content);
        assert!(result.is_ok());

        // 变更记录在 CmdContent 中
        let last = engine
            .last_result
            .as_ref()
            .expect("last_result should exist");
        assert_eq!(last.content.changes.len(), 1);
        match &last.content.changes[0] {
            crate::cmd_content::ContentChange::Insert {
                after_line,
                lines,
                source_cmd,
            } => {
                assert_eq!(*after_line, 0);
                assert_eq!(lines.len(), 1);
                assert_eq!(source_cmd, "NEW");
            }
            other => panic!("Expected Insert change, got {:?}", other),
        }
    }

    #[test]
    fn test_new_without_location_errors() {
        let (_dir, mut engine) = engine_with_file("line1\n");
        let content = make_new_content(&[("new_line", 0, false)]);

        let result = execute(&mut engine, NewMode::Normal, content);
        assert!(result.is_err());
    }

    #[test]
    fn test_new_with_raw_line() {
        let (_dir, mut engine) = engine_with_file("line1\nline2\n");
        let content = make_new_content(&[("    raw_line", 4, true)]);

        let result = execute(&mut engine, NewMode::End, content);
        assert!(result.is_ok());
        let file = engine.file.as_ref().unwrap();
        assert_eq!(file.lines[2].content, "    raw_line");
    }

    // ============================================================
    // BUG-401: New(Start/End) 始终在文件级别操作
    // ============================================================

    #[test]
    fn test_new_start_operates_at_file_level_even_with_pending_location() {
        let (_dir, mut engine) =
            engine_with_file("// header\nfn main() {\n    old();\n}\n// footer\n");

        // 先创建 Location（block_stack 非空）
        crate::commands::location::execute(
            &mut engine,
            LocationMode::Normal,
            Some(build_location_content(&["fn main() {"])),
            &HashMap::new(),
        )
        .unwrap();
        assert!(!engine.block_stack.is_empty());

        // 执行 New(Start) — 应在文件开头，而非 block 开头
        let content = make_new_content(&[("#![allow(warnings)]", 0, false)]);
        let result = execute(&mut engine, NewMode::Start, content);
        assert!(result.is_ok());

        let file = engine.file.as_ref().unwrap();
        assert_eq!(file.lines[0].content, "#![allow(warnings)]");
        assert_eq!(file.lines[1].content, "// header");
    }

    /// 辅助函数：构建 LocationContent
    fn build_location_content(lines: &[&str]) -> crate::model::LocationContent {
        use crate::model::LocationLine;
        if lines.is_empty() {
            return crate::model::LocationContent { lines: vec![] };
        }
        let base_taps = crate::model::count_leading_spaces(lines[0]);
        let loc_lines: Vec<LocationLine> = lines
            .iter()
            .enumerate()
            .map(|(i, content)| {
                let line_taps = crate::model::count_leading_spaces(content);
                let diff_taps = Some(line_taps.saturating_sub(base_taps));
                LocationLine {
                    index: i,
                    diff_taps,
                    content: content.to_string(),
                    line_num: None,
                }
            })
            .collect();
        crate::model::LocationContent { lines: loc_lines }
    }
}
