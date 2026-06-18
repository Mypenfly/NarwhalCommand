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
use crate::model::{DeleteContent, MatchInfo};
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
///
/// 要求前一个 Location 也使用 Block 模式。
fn execute_block(engine: &mut Engine) -> Result<(), NcsError> {
    let location_is_block = engine
        .exec_cmds
        .iter()
        .rev()
        .find(|ec| ec.cmd_name == "LOCATION")
        .is_some_and(|ec| ec.mode_name == "Block");

    if !location_is_block {
        return Err(NcsError::Engine(
            crate::error::EngineError::BlockRequiredForDelete,
        ));
    }

    let block = engine.block_stack.last().ok_or(NcsError::Engine(
        crate::error::EngineError::MissingLocationForNew,
    ))?;

    let total = block.lines.len();
    let diff_data = if total > 0 {
        // 从原始 block 收集 diff
        let (changed, context_above, context_below) =
            executor::collect_deleted_diff_data(block, 0, total.saturating_sub(1));
        Some((changed, context_above, context_below))
    } else {
        None
    };

    // Phase 3: 记录全部删除变更，不再直接 drain block.lines
    if let Some(ref mut result) = engine.last_result {
        if total > 0 {
            result
                .content
                .record_delete(0, total.saturating_sub(1), "DELETE");
        }
    }

    // 更新 match_info（handle_close 中 apply 时会重新设置 block.lines）
    let block = engine.block_stack.last_mut().ok_or(NcsError::Engine(
        crate::error::EngineError::MissingLocationForNew,
    ))?;
    block.match_info = MatchInfo::DeleteAt { position: 0 };

    if let Some((changed, context_above, context_below)) = diff_data {
        engine.record_diff_with_context(changed, context_above, context_below);
    }
    Ok(())
}

/// 在 CmdContent 快照或 ContentBlock 中逐行匹配并删除
fn execute_normal(engine: &mut Engine, content: Option<DeleteContent>) -> Result<(), NcsError> {
    let del_content = content.ok_or(NcsError::Engine(
        crate::error::EngineError::MissingLocationForNew,
    ))?;

    let block = engine.block_stack.last_mut().ok_or(NcsError::Engine(
        crate::error::EngineError::MissingLocationForNew,
    ))?;

    // 优先从 last_result.snapshot_lines 匹配（Phase 3 快照路径），
    // 若不可用则回退到 block-based 匹配。
    let snapshot_lines: Option<&[crate::cmd_content::CmdLine]> =
        engine.last_result.as_ref().and_then(|r| {
            if r.content.snapshot_lines.is_empty() {
                None
            } else {
                Some(r.content.snapshot_lines.as_slice())
            }
        });

    let (snapshot_start_idx, snapshot_end_idx) = if let Some(snapshot) = snapshot_lines {
        let (s, e) = executor::find_delete_match_in_snapshot(snapshot, &del_content)
            .ok_or_else(|| executor::delete_not_found_in_snapshot_error(&del_content, snapshot))?;

        let matched_line_count = match &block.match_info {
            MatchInfo::Location { matched_line_count } => *matched_line_count,
            _ => 0,
        };
        executor::check_delete_adjacency_in_snapshot(snapshot, matched_line_count, s)?;
        (s, e)
    } else {
        let (s, e) = executor::find_delete_match(block, &del_content)
            .ok_or_else(|| executor::delete_not_found_error(&del_content, block))?;
        executor::check_delete_adjacency(block, s)?;
        (s, e)
    };

    // 将快照索引映射到 block.lines 中的实际位置。
    // 若快照与 block 内容一致（无先前修改），直接使用快照索引。
    // 否则通过去空白内容逐行搜索对应位置。
    let (block_start_idx, block_end_idx) = if let Some(snapshot) = snapshot_lines {
        let synced = snapshot.len() == block.lines.len()
            && snapshot
                .iter()
                .zip(block.lines.iter())
                .all(|(s, b)| s.content == b.content);

        if synced {
            (snapshot_start_idx, snapshot_end_idx)
        } else {
            let start =
                executor::map_snapshot_index_to_block_index(block, snapshot, snapshot_start_idx);
            let end =
                executor::map_snapshot_index_to_block_index(block, snapshot, snapshot_end_idx);

            match (start, end) {
                (Some(bs), Some(be)) if be >= bs => (bs, be),
                (Some(bs), None) => (bs, bs + (snapshot_end_idx - snapshot_start_idx)),
                _ => (snapshot_start_idx, snapshot_end_idx),
            }
        }
    } else {
        (snapshot_start_idx, snapshot_end_idx)
    };

    let block_start_idx = block_start_idx.min(block.lines.len().saturating_sub(1));
    let block_end_idx = block_end_idx.min(block.lines.len().saturating_sub(1));

    if block_end_idx < block_start_idx {
        return Err(NcsError::Match(
            crate::error::MatchError::DeleteMatchFailed {
                delete_content: del_content
                    .lines
                    .first()
                    .map(|l| l.content.clone())
                    .unwrap_or_default(),
                block_snippet: String::from("mapping failed: block_end < block_start"),
            },
        ));
    }

    // 从原始 block 收集 diff（删除前快照）
    let (changed, context_above, context_below) =
        executor::collect_deleted_diff_data(block, block_start_idx, block_end_idx);

    // Phase 3 变更追踪：记录变更到 CmdContent，由 handle_close 统一应用到 block
    // 不再直接 drain block.lines
    if let Some(ref mut result) = engine.last_result {
        result
            .content
            .record_delete(snapshot_start_idx, snapshot_end_idx, "DELETE");
    }

    block.match_info = MatchInfo::DeleteAt {
        position: block_start_idx,
    };

    engine.record_diff_with_context(changed, context_above, context_below);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;
    use crate::model::{DeleteContent, DeleteLine};
    use crate::parser::{Command, DeleteMode, LocationMode, OpenMode};
    use std::collections::HashMap;

    /// 创建带临时文件的引擎（已执行 Open + Location），并设置 last_result
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

        // Phase 3: 设置 last_result（模拟 engine.execute 管线）
        let block = engine.block_stack.last().unwrap();
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

        engine.exec_cmds.push(crate::engine::ExecutedCommand {
            cmd_name: "OPEN".to_string(),
            mode_name: "Normal".to_string(),
            is_independent: false,
        });
        engine.exec_cmds.push(crate::engine::ExecutedCommand {
            cmd_name: "LOCATION".to_string(),
            mode_name: "Normal".to_string(),
            is_independent: false,
        });

        (dir, engine)
    }

    #[test]
    fn test_delete_normal_removes_matching_line() {
        let (_dir, mut engine) =
            engine_with_location("fn foo() {\n    bar();\n    baz();\n}\n", &["fn foo() {"]);

        let del = make_delete_content(&["bar();"]);
        let result = execute(&mut engine, DeleteMode::Normal, Some(del));
        assert!(result.is_ok());

        // 变更记录在 engine.last_result.content.changes 中
        let last = engine
            .last_result
            .as_ref()
            .expect("last_result should exist after execute");
        assert_eq!(last.content.changes.len(), 1, "should have one change");
        match &last.content.changes[0] {
            crate::cmd_content::ContentChange::Delete {
                start_line,
                end_line,
                source_cmd,
            } => {
                assert_eq!(*start_line, 1);
                assert_eq!(*end_line, 1);
                assert_eq!(source_cmd, "DELETE");
            }
            other => panic!("Expected Delete change, got {:?}", other),
        }
    }

    #[test]
    fn test_delete_normal_removes_multi_line() {
        let (_dir, mut engine) =
            engine_with_location("fn foo() {\n    bar();\n    baz();\n}\n", &["fn foo() {"]);

        let del = make_delete_content(&["bar();", "baz();"]);
        let result = execute(&mut engine, DeleteMode::Normal, Some(del));
        assert!(result.is_ok());

        let last = engine
            .last_result
            .as_ref()
            .expect("last_result should exist");
        assert_eq!(last.content.changes.len(), 1);
        match &last.content.changes[0] {
            crate::cmd_content::ContentChange::Delete {
                start_line,
                end_line,
                ..
            } => {
                assert_eq!(*start_line, 1);
                assert_eq!(*end_line, 2);
            }
            other => panic!("Expected Delete, got {:?}", other),
        }
    }

    #[test]
    fn test_delete_block_clears_block() {
        let (_dir, mut engine) =
            engine_with_location("fn foo() {\n    bar();\n    baz();\n}\n", &["fn foo() {"]);
        engine.exec_cmds.push(crate::engine::ExecutedCommand {
            cmd_name: "LOCATION".to_string(),
            mode_name: "Block".to_string(),
            is_independent: false,
        });

        let result = execute(&mut engine, DeleteMode::Block, None);
        assert!(result.is_ok());

        // Delete:Block 记录全部删除
        let last = engine
            .last_result
            .as_ref()
            .expect("last_result should exist after block delete");
        assert_eq!(last.content.changes.len(), 1);
        match &last.content.changes[0] {
            crate::cmd_content::ContentChange::Delete {
                start_line,
                end_line,
                ..
            } => {
                assert_eq!(*start_line, 0);
                // 4 行 block, delete all: end = 3
                assert_eq!(*end_line, 3);
            }
            other => panic!("Expected Delete, got {:?}", other),
        }
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
        // 先设置 exec_cmds 以便 Delete:Block 检查通过
        engine.exec_cmds.push(crate::engine::ExecutedCommand {
            cmd_name: "LOCATION".to_string(),
            mode_name: "Normal".to_string(),
            is_independent: false,
        });

        let del = make_delete_content(&["nonexistent();"]);
        let result = execute(&mut engine, DeleteMode::Normal, Some(del));
        assert!(result.is_err());
    }

    // ============================================================
    // BUG-402: Delete:Block 校验 Location:Block
    // ============================================================

    #[test]
    fn test_delete_block_with_location_normal_errors() {
        let (_dir, mut engine) =
            engine_with_location("fn foo() {\n    bar();\n    baz();\n}\n", &["fn foo() {"]);
        // Location 是 Normal 模式
        engine.exec_cmds.push(crate::engine::ExecutedCommand {
            cmd_name: "LOCATION".to_string(),
            mode_name: "Normal".to_string(),
            is_independent: false,
        });

        let result = execute(&mut engine, DeleteMode::Block, None);
        assert!(result.is_err());
        match result.unwrap_err() {
            NcsError::Engine(crate::error::EngineError::BlockRequiredForDelete) => {}
            other => panic!("Expected BlockRequiredForDelete, got {:?}", other),
        }
    }

    #[test]
    fn test_delete_block_with_location_block_succeeds() {
        let (_dir, mut engine) =
            engine_with_location("fn foo() {\n    bar();\n    baz();\n}\n", &["fn foo() {"]);
        // Location 是 Block 模式
        engine.exec_cmds.push(crate::engine::ExecutedCommand {
            cmd_name: "LOCATION".to_string(),
            mode_name: "Block".to_string(),
            is_independent: false,
        });

        let result = execute(&mut engine, DeleteMode::Block, None);
        assert!(result.is_ok());
    }

    // ============================================================
    // BUG-204: Delete 在 snapshot 上匹配（New 在前不污染匹配）
    // ============================================================

    #[test]
    fn test_delete_matches_on_snapshot_not_affected_by_prior_new() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.rs");
        std::fs::write(
            &file_path,
            "fn main() {\n    let old = 1;\n    println!(\"{}\", old);\n}\n",
        )
        .unwrap();

        let mut engine = Engine::new();
        let registry = crate::registry::CommandRegistry::init();

        let commands = vec![
            Command::Open {
                mode: crate::parser::OpenMode::Normal,
                path: file_path.to_str().unwrap().to_string(),
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
                content: Some(make_delete_content(&["    let old = 1;"])),
            },
        ];

        let result = engine.execute(commands, &registry);
        assert!(
            result.is_ok(),
            "Delete should succeed when matching on snapshot, got: {:?}",
            result.err()
        );

        // 验证快照匹配工作：last_result 应有 Delete 变更记录
        let last = engine.last_result.as_ref().unwrap();
        assert!(
            !last.content.changes.is_empty(),
            "Should have recorded Delete change"
        );
    }

    #[test]
    fn test_delete_matches_on_snapshot_with_multiple_new_before() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.rs");
        std::fs::write(
            &file_path,
            "fn main() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n}\n",
        )
        .unwrap();

        let mut engine = Engine::new();
        let registry = crate::registry::CommandRegistry::init();

        let commands = vec![
            Command::Open {
                mode: crate::parser::OpenMode::Normal,
                path: file_path.to_str().unwrap().to_string(),
                args: HashMap::new(),
            },
            Command::Location {
                mode: crate::parser::LocationMode::Normal,
                content: Some(crate::model::LocationContent {
                    lines: vec![
                        crate::model::LocationLine {
                            index: 0,
                            diff_taps: Some(0),
                            content: "fn main() {".to_string(),
                            line_num: None,
                        },
                        crate::model::LocationLine {
                            index: 1,
                            diff_taps: Some(4),
                            content: "    let a = 1;".to_string(),
                            line_num: None,
                        },
                    ],
                }),
                args: HashMap::new(),
            },
            Command::New {
                mode: crate::parser::NewMode::Normal,
                content: crate::model::NewContent {
                    lines: vec![crate::model::NewLine {
                        diff_taps: 4,
                        content: "let x = 0;".to_string(),
                        is_raw: false,
                    }],
                },
            },
            Command::New {
                mode: crate::parser::NewMode::Normal,
                content: crate::model::NewContent {
                    lines: vec![crate::model::NewLine {
                        diff_taps: 4,
                        content: "let y = 0;".to_string(),
                        is_raw: false,
                    }],
                },
            },
            Command::Delete {
                mode: crate::parser::DeleteMode::Normal,
                content: Some(make_delete_content(&["    let b = 2;"])),
            },
        ];

        let result = engine.execute(commands, &registry);
        assert!(
            result.is_ok(),
            "Delete should still find 'let b = 2' in snapshot, got: {:?}",
            result.err()
        );

        // block_stack 已被隐式关闭清空，检查 file 中的最终内容
        let file = engine.file.as_ref().unwrap();
        // 原 5 行, Delete 删 1, New 加 2 = 6 行
        assert_eq!(file.lines.len(), 6);
        // 验证 let b 已被删除
        let contents: Vec<&str> = file
            .lines
            .iter()
            .map(|l| l.stripped_content.as_str())
            .collect();
        assert!(!contents.contains(&"letb=2;"), "let b should be deleted");
    }

    #[test]
    fn test_delete_with_empty_location_and_new_replacement() {
        let file_content = "// header\n#[test]\nfn test_config_default() {\n    let config = AppConfig::default();\n    assert_eq!(config.name, \"myapp\");\n}\n// footer\n";
        let (_dir, mut engine) = engine_with_location(file_content, &[]);

        let del = make_delete_content(&[
            "#[test]",
            "fn test_config_default() {",
            "let config = AppConfig::default();",
            "assert_eq!(config.name, \"myapp\");",
            "}",
        ]);
        let result = execute(&mut engine, DeleteMode::Normal, Some(del));
        assert!(
            result.is_ok(),
            "Delete with empty Location should work, got: {:?}",
            result.err()
        );

        // 变更应记录在 CmdContent 中
        let last = engine
            .last_result
            .as_ref()
            .expect("last_result should exist");
        assert_eq!(last.content.changes.len(), 1);
        match &last.content.changes[0] {
            crate::cmd_content::ContentChange::Delete {
                start_line,
                end_line,
                ..
            } => {
                assert_eq!(*start_line, 1);
                assert_eq!(*end_line, 5);
            }
            other => panic!("Expected Delete, got {:?}", other),
        }
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
