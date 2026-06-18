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
fn execute_start(engine: &mut Engine, content: NewContent) -> Result<(), NcsError> {
    let new_lines = executor::build_new_lines(&content);
    let new_line_count = new_lines.len();

    if let Some(block) = engine.block_stack.last_mut() {
        let insert_pos = 0;
        let tail = std::mem::take(&mut block.lines);
        let mut combined = new_lines;
        combined.extend(tail);
        block.lines = combined;
        block.reindex();

        let (changed, context_above, context_below) =
            executor::collect_added_diff_data(block, insert_pos, new_line_count);
        engine.record_diff_with_context(changed, context_above, context_below);
    } else if let Some(ref mut file) = engine.file {
        let insert_pos = 0;
        let tail = std::mem::take(&mut file.lines);
        let mut combined = new_lines;
        combined.extend(tail);
        file.lines = combined;
        executor::reindex_file(file);

        let added_entries = executor::collect_new_file_line_info(file, insert_pos, new_line_count);
        engine.record_added_lines(added_entries);
    }

    Ok(())
}

/// 在文件/Block 末尾插入新内容
fn execute_end(engine: &mut Engine, content: NewContent) -> Result<(), NcsError> {
    let new_lines = executor::build_new_lines(&content);
    let new_line_count = new_lines.len();

    if let Some(block) = engine.block_stack.last_mut() {
        let insert_start = block.lines.len();
        block.lines.extend(new_lines);
        block.reindex();

        let (changed, context_above, context_below) =
            executor::collect_added_diff_data(block, insert_start, new_line_count);
        engine.record_diff_with_context(changed, context_above, context_below);
    } else if let Some(ref mut file) = engine.file {
        let insert_start = file.lines.len();
        file.lines.extend(new_lines);
        executor::reindex_file(file);

        let added_entries =
            executor::collect_new_file_line_info(file, insert_start, new_line_count);
        engine.record_added_lines(added_entries);
    }

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

    let (changed, context_above, context_below) = {
        let block = engine.block_stack.last_mut().ok_or(NcsError::Engine(
            crate::error::EngineError::MissingLocationForNew,
        ))?;
        if insert_pos >= block.lines.len() {
            block.lines.extend(new_lines);
        } else {
            let tail = block.lines.split_off(insert_pos);
            block.lines.extend(new_lines);
            block.lines.extend(tail);
        }
        block.reindex();
        executor::collect_added_diff_data(block, insert_pos, new_line_count)
    };

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
            lines: lines
                .iter()
                .map(|(content, diff_taps, is_raw)| NewLine {
                    diff_taps: *diff_taps,
                    content: content.to_string(),
                    is_raw: *is_raw,
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

        let content = make_new_content(&[("new_code();", 4, false)]);
        let result = execute(&mut engine, NewMode::Normal, content);
        assert!(result.is_ok());

        let block = engine.block_stack.last().unwrap();
        // 原 block 有 4 行，new_code 插入在 fn foo 之后（index 1），共 5 行
        assert_eq!(block.lines.len(), 5);
        assert_eq!(block.lines[1].content, "    new_code();");
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
