//! Delete 命令
//!
//! `!@Delete` 在当前 ContentBlock 中匹配并删除指定内容，
//! 支持 Normal/Block 两种模式。
//!
//! ## 实现逻辑
//!
//! 1. Normal 模式：在 block 内逐行去空白匹配 DeleteContent
//! 2. 要求连续匹配，检查邻接关系
//! 3. Block 模式：删除整个 ContentBlock
//! 4. 删除后 reindex 并收集 diff
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §5.4 "Delete", INSTRUCTION.md §3.4 "Delete 操作算法"
//!
//! ## 迁移来源
//!
//! 从 n_edit/src/engine.rs 的 execute_delete*() 迁移。

use crate::engine::executor;
use crate::engine::Engine;
use crate::error::NcsError;
use crate::model::{DeleteContent, Line, MatchInfo};
use crate::parser::DeleteMode;

/// Delete 命令的执行入口
pub fn execute(
    engine: &mut Engine,
    mode: DeleteMode,
    content: Option<DeleteContent>,
) -> Result<(), NcsError> {
    match mode {
        DeleteMode::Block => execute_block(engine),
        DeleteMode::Normal => execute_normal(engine, content),
    }
}

/// 删除整个 ContentBlock
fn execute_block(engine: &mut Engine) -> Result<(), NcsError> {
    let block = engine.block_stack.last_mut().ok_or(NcsError::Engine(
        crate::error::EngineError::MissingLocationForNew,
    ))?;

    let total = block.lines.len();
    let diff_data = if total > 0 {
        let (changed, context_above, context_below) =
            executor::collect_deleted_diff_data(block, 0, total.saturating_sub(1));
        Some((changed, context_above, context_below))
    } else {
        None
    };

    // 保留首行行号，清空所有行
    let first_line_num = block.start_line;
    block.lines.clear();
    block.lines.push(Line {
        line_num: first_line_num,
        taps: 0,
        diff_taps: 0,
        content: String::new(),
        stripped_content: String::new(),
    });
    block.match_info = MatchInfo::DeleteAt { position: 0 };
    block.reindex();

    if let Some((changed, context_above, context_below)) = diff_data {
        engine.record_diff_with_context(changed, context_above, context_below);
    }
    Ok(())
}

/// 在 ContentBlock 中逐行匹配并删除
fn execute_normal(engine: &mut Engine, content: Option<DeleteContent>) -> Result<(), NcsError> {
    let del_content = content.ok_or(NcsError::Engine(
        crate::error::EngineError::MissingLocationForNew,
    ))?;

    let block = engine.block_stack.last_mut().ok_or(NcsError::Engine(
        crate::error::EngineError::MissingLocationForNew,
    ))?;

    let (start_idx, end_idx) = match executor::find_delete_match(block, &del_content) {
        Some(range) => range,
        None => return Err(executor::delete_not_found_error(&del_content, block)),
    };

    // 检查邻接关系
    executor::check_delete_adjacency(block, start_idx)?;

    // 在删除之前收集上下文和删除行数据
    let (changed, context_above, context_below) =
        executor::collect_deleted_diff_data(block, start_idx, end_idx);

    // 执行删除
    block.lines.drain(start_idx..=end_idx);
    block.match_info = MatchInfo::DeleteAt {
        position: start_idx,
    };
    block.reindex();

    engine.record_diff_with_context(changed, context_above, context_below);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;
    use crate::model::{DeleteContent, DeleteLine};
    use crate::parser::{DeleteMode, LocationMode, OpenMode};
    use std::collections::HashMap;

    /// 创建带临时文件的引擎（已执行 Open + Location）
    fn engine_with_location(
        file_content: &str,
        location_lines: &[&str],
    ) -> (tempfile::TempDir, Engine) {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.rs");
        std::fs::write(&file_path, file_content).unwrap();

        let mut engine = Engine::new();
        crate::commands::open::execute(
            &mut engine,
            OpenMode::Normal,
            file_path.to_str().unwrap(),
            &HashMap::new(),
        )
        .unwrap();
        crate::commands::location::execute(
            &mut engine,
            LocationMode::Normal,
            Some(build_location_content(location_lines)),
            &HashMap::new(),
        )
        .unwrap();
        (dir, engine)
    }

    #[test]
    fn test_delete_normal_removes_matching_line() {
        let (_dir, mut engine) =
            engine_with_location("fn foo() {\n    bar();\n    baz();\n}\n", &["fn foo() {"]);

        let del = make_delete_content(&["bar();"]);
        let result = execute(&mut engine, DeleteMode::Normal, Some(del));
        assert!(result.is_ok());

        let block = engine.block_stack.last().unwrap();
        // 原 4 行，删除 1 行 "bar();" → 3 行
        assert_eq!(block.lines.len(), 3);
        assert_eq!(block.lines[1].content, "    baz();");
    }

    #[test]
    fn test_delete_normal_removes_multi_line() {
        let (_dir, mut engine) =
            engine_with_location("fn foo() {\n    bar();\n    baz();\n}\n", &["fn foo() {"]);

        let del = make_delete_content(&["bar();", "baz();"]);
        let result = execute(&mut engine, DeleteMode::Normal, Some(del));
        assert!(result.is_ok());

        let block = engine.block_stack.last().unwrap();
        assert_eq!(block.lines.len(), 2);
        assert_eq!(block.lines[0].content, "fn foo() {");
        assert_eq!(block.lines[1].content, "}");
    }

    #[test]
    fn test_delete_block_clears_block() {
        let (_dir, mut engine) =
            engine_with_location("fn foo() {\n    bar();\n    baz();\n}\n", &["fn foo() {"]);

        let result = execute(&mut engine, DeleteMode::Block, None);
        assert!(result.is_ok());

        let block = engine.block_stack.last().unwrap();
        assert_eq!(block.lines.len(), 1);
        assert!(block.lines[0].content.is_empty());
    }

    #[test]
    fn test_delete_without_location_errors() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.rs");
        std::fs::write(&file_path, "line1\nline2\n").unwrap();
        let mut engine = Engine::new();
        crate::commands::open::execute(
            &mut engine,
            OpenMode::Normal,
            file_path.to_str().unwrap(),
            &HashMap::new(),
        )
        .unwrap();

        let del = make_delete_content(&["line1"]);
        let result = execute(&mut engine, DeleteMode::Normal, Some(del));
        assert!(result.is_err());
    }

    #[test]
    fn test_delete_normal_not_found_errors() {
        let (_dir, mut engine) =
            engine_with_location("fn foo() {\n    bar();\n}\n", &["fn foo() {"]);

        let del = make_delete_content(&["nonexistent();"]);
        let result = execute(&mut engine, DeleteMode::Normal, Some(del));
        assert!(result.is_err());
    }

    /// 辅助
    fn make_delete_content(lines: &[&str]) -> DeleteContent {
        DeleteContent {
            lines: lines
                .iter()
                .map(|s| DeleteLine {
                    content: s.to_string(),
                    is_raw: false,
                })
                .collect(),
        }
    }

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
