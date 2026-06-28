//! 引擎执行辅助函数 (Executor)
//!
//! 提供引擎命令执行时复用的纯函数——不依赖 Engine 状态，
//! 通过参数显式传递所需数据。
//!
//! ## 实现逻辑
//!
//! 1. Delete 辅助：find_delete_match / lines_continuously_match / check_delete_adjacency
//! 2. 文件/Block 写回：apply_block_to_file / apply_block_to_parent / reindex_file
//! 3. New 内容构建：build_new_lines / collect_new_line_info / collect_new_file_line_info
//! 4. Diff 收集：collect_block_context_above/below / collect_added/deleted_diff_data
//!
//! ## 对应文档
//!
//! 详见 INSTRUCTION.md §3.3-3.4, n_edit_dev.md
//!
//! ## 迁移来源
//!
//! 从 n_edit/src/engine.rs 拆分出来的纯函数。

use crate::cmd_content::CmdLine;
use crate::error::NcsError;
use crate::model::{
    self, ContentBlock, DeleteContent, DeleteLine, FileContent, Line, LineNumber, MatchInfo,
    NewContent, BLOCK_SNIPPET_MAX_LINES,
};
use crate::output::{DiffLine, DiffLineKind, CONTEXT_MAX_LINES};
use std::collections::HashMap;

// ============================================================
// Delete 匹配辅助函数
// ============================================================

/// 在 ContentBlock 中查找 DeleteContent 的连续匹配区间
///
/// 返回 (start_index, end_index) 在 block.lines 中的索引。
/// 要求所有行连续匹配，不可跳行。
pub fn find_delete_match(
    block: &ContentBlock,
    del_content: &DeleteContent,
) -> Option<(usize, usize)> {
    let del_lines = &del_content.lines;
    if del_lines.is_empty() || block.lines.is_empty() {
        return None;
    }

    let first_del_stripped = model::stripped_content(&del_lines[0].content);

    for start_idx in 0..block.lines.len() {
        if block.lines[start_idx].stripped_content() != first_del_stripped {
            continue;
        }

        if start_idx + del_lines.len() > block.lines.len() {
            continue;
        }

        if lines_continuously_match(block, del_lines, start_idx) {
            return Some((start_idx, start_idx + del_lines.len() - 1));
        }
    }

    None
}

/// 检查从 start_idx 开始，block 的行是否与 delete_content 所有行连续匹配
pub fn lines_continuously_match(
    block: &ContentBlock,
    del_lines: &[DeleteLine],
    start_idx: usize,
) -> bool {
    for (offset, del_line) in del_lines.iter().enumerate() {
        let block_line = &block.lines[start_idx + offset];

        let block_stripped = block_line.stripped_content();
        let del_stripped = model::stripped_content(&del_line.content);

        let block_is_empty = block_line.content.trim().is_empty();
        let del_is_empty = del_line.content.trim().is_empty();

        if block_is_empty && del_is_empty {
            continue;
        }
        if block_is_empty || del_is_empty {
            return false;
        }
        if block_stripped != del_stripped {
            return false;
        }
    }
    true
}

/// 检查 Delete 匹配位置是否与 Location 最后一行的位置紧邻
///
/// 若之间隔了非空行，说明 Delete 可能删错了位置。
pub fn check_delete_adjacency(block: &ContentBlock, start_idx: usize) -> Result<(), NcsError> {
    if let MatchInfo::Location { matched_line_count } = &block.match_info {
        if *matched_line_count == 0 {
            return Ok(());
        }
        let location_last_idx = matched_line_count.saturating_sub(1);
        if start_idx <= location_last_idx {
            return Ok(());
        }
        let gap_non_empty: Vec<_> = block.lines[location_last_idx + 1..start_idx]
            .iter()
            .filter(|l| !l.content.trim().is_empty())
            .collect();
        if !gap_non_empty.is_empty() {
            let loc_last = &block.lines[location_last_idx].content;
            let del_first = &block.lines[start_idx].content;
            return Err(NcsError::Match(
                crate::error::MatchError::DeleteNotAdjacent {
                    location_last_line: loc_last.clone(),
                    delete_first_line: del_first.clone(),
                    gap_lines: gap_non_empty.len(),
                },
            ));
        }
    }
    Ok(())
}

/// 记录被删除的行到 diff_lines
#[allow(dead_code)]
pub fn record_deleted_lines(
    block: &ContentBlock,
    start_idx: usize,
    end_idx: usize,
) -> Vec<DiffLine> {
    block.lines[start_idx..=end_idx]
        .iter()
        .map(|line| DiffLine {
            kind: DiffLineKind::Deleted,
            line_number: Some(line.line_num),
            content: line.content.clone(),
        })
        .collect()
}

/// 构建 Delete 未找到匹配时的错误信息
pub fn delete_not_found_error(del_content: &DeleteContent, block: &ContentBlock) -> NcsError {
    let first_del_line = del_content
        .lines
        .first()
        .map(|l| l.content.as_str())
        .unwrap_or("");
    let block_snippet = block
        .lines
        .iter()
        .take(BLOCK_SNIPPET_MAX_LINES)
        .map(|l| l.content.as_str())
        .collect::<Vec<&str>>()
        .join("\n");
    NcsError::Match(crate::error::MatchError::DeleteMatchFailed {
        delete_content: first_del_line.to_string(),
        block_snippet,
    })
}

/// 构建 Delete 未找到匹配时的错误信息（基于 CmdLine 快照）
pub fn delete_not_found_in_snapshot_error(
    del_content: &DeleteContent,
    snapshot: &[CmdLine],
) -> NcsError {
    let first_del_line = del_content
        .lines
        .first()
        .map(|l| l.content.as_str())
        .unwrap_or("");
    let block_snippet = snapshot
        .iter()
        .take(BLOCK_SNIPPET_MAX_LINES)
        .map(|l| l.content.as_str())
        .collect::<Vec<&str>>()
        .join("\n");
    NcsError::Match(crate::error::MatchError::DeleteMatchFailed {
        delete_content: first_del_line.to_string(),
        block_snippet,
    })
}

// ============================================================
// Delete 匹配辅助函数（CmdLine 快照版本）
// ============================================================
// 以下函数与上面的 ContentBlock 版本对偶，但操作 CmdLine 快照。
// 用于 Phase 3 变更追踪模型：Delete 匹配在 snapshot_lines 上进行。

/// 在 CmdLine 快照中查找 DeleteContent 的连续匹配区间
///
/// 返回 (start_index, end_index) 在 snapshot 中的索引。
/// 要求匹配行连续，不可跳行。
pub fn find_delete_match_in_snapshot(
    snapshot: &[CmdLine],
    del_content: &DeleteContent,
) -> Option<(usize, usize)> {
    let del_lines = &del_content.lines;
    if del_lines.is_empty() || snapshot.is_empty() {
        return None;
    }

    let first_del_stripped = model::stripped_content(&del_lines[0].content);

    for start_idx in 0..snapshot.len() {
        if snapshot[start_idx].stripped_content() != first_del_stripped {
            continue;
        }
        if start_idx + del_lines.len() > snapshot.len() {
            continue;
        }
        if cmd_lines_continuously_match(snapshot, del_lines, start_idx) {
            return Some((start_idx, start_idx + del_lines.len() - 1));
        }
    }
    None
}

/// 检查从 start_idx 开始，CmdLine 快照的行是否与 delete_content 所有行连续匹配
pub fn cmd_lines_continuously_match(
    snapshot: &[CmdLine],
    del_lines: &[DeleteLine],
    start_idx: usize,
) -> bool {
    for (offset, del_line) in del_lines.iter().enumerate() {
        let snap_line = &snapshot[start_idx + offset];
        let snap_stripped = snap_line.stripped_content();
        let del_stripped = model::stripped_content(&del_line.content);
        let snap_is_empty = snap_line.content.trim().is_empty();
        let del_is_empty = del_line.content.trim().is_empty();

        if snap_is_empty && del_is_empty {
            continue;
        }
        if snap_is_empty || del_is_empty {
            return false;
        }
        if snap_stripped != del_stripped {
            return false;
        }
    }
    true
}

/// 检查 Delete 在快照中的匹配位置是否与 Location 匹配的最后一行紧邻
///
/// matched_line_count 是 Location 在快照中匹配的行数。
pub fn check_delete_adjacency_in_snapshot(
    snapshot: &[CmdLine],
    matched_line_count: usize,
    start_idx: usize,
) -> Result<(), NcsError> {
    if matched_line_count == 0 {
        return Ok(());
    }
    let location_last_idx = matched_line_count.saturating_sub(1);
    if start_idx <= location_last_idx {
        return Ok(());
    }
    let gap_non_empty: Vec<_> = snapshot[location_last_idx + 1..start_idx]
        .iter()
        .filter(|l| !l.content.trim().is_empty())
        .collect();
    if !gap_non_empty.is_empty() {
        let loc_last = &snapshot[location_last_idx].content;
        let del_first = &snapshot[start_idx].content;
        return Err(NcsError::Match(
            crate::error::MatchError::DeleteNotAdjacent {
                location_last_line: loc_last.clone(),
                delete_first_line: del_first.clone(),
                gap_lines: gap_non_empty.len(),
            },
        ));
    }
    Ok(())
}

/// 将 CmdLine 快照中的匹配索引映射到 ContentBlock 的 lines 中对应位置
///
/// 通过去空白内容匹配找到 block.lines 中的对应索引。
pub fn map_snapshot_index_to_block_index(
    block: &ContentBlock,
    snapshot: &[CmdLine],
    snapshot_index: usize,
) -> Option<usize> {
    if snapshot_index >= snapshot.len() {
        return None;
    }
    let target_stripped = snapshot[snapshot_index].stripped_content();
    block
        .lines
        .iter()
        .position(|l| l.stripped_content() == target_stripped)
}

// ============================================================
// Block / File 写回辅助函数
// ============================================================

/// 将 ContentBlock 的修改应用到 FileContent 中对应位置
///
/// 使用 block.start_line 和 block.end_line 确定原始范围，
/// 将其替换为 block 的当前行。
pub fn apply_block_to_file(file: &mut FileContent, block: &ContentBlock) {
    let start_index = block.start_line.to_index();
    let end_index = block.end_line.to_index();

    let count = end_index.saturating_sub(start_index) + 1;
    let count = count.min(file.lines.len().saturating_sub(start_index));

    let new_lines: Vec<Line> = block
        .lines
        .iter()
        .map(|line| Line {
            line_num: line.line_num,
            taps: line.taps,
            diff_taps: line.diff_taps,
            content: line.content.clone(),
            stripped_content: line.stripped_content.clone(),
        })
        .collect();

    file.lines
        .splice(start_index..start_index + count, new_lines);

    reindex_file(file);
}

/// 将内层 ContentBlock 的修改应用到父级 ContentBlock 中
///
/// 用于嵌套 Location 场景：内层 Block（inner）弹出后，
/// 通过 start_line 差值计算偏移量，将内层修改合并回父级 Block（outer）。
pub fn apply_block_to_parent(inner: &ContentBlock, outer: &mut ContentBlock) {
    let start_offset = inner
        .start_line
        .to_index()
        .saturating_sub(outer.start_line.to_index());
    let end_offset = inner
        .end_line
        .to_index()
        .saturating_sub(outer.start_line.to_index());

    let start_offset = start_offset.min(outer.lines.len());
    let end_offset = end_offset.min(outer.lines.len().saturating_sub(1));

    let count = if end_offset >= start_offset {
        end_offset - start_offset + 1
    } else {
        0
    };

    let new_lines: Vec<Line> = inner
        .lines
        .iter()
        .map(|line| Line {
            line_num: line.line_num,
            taps: line.taps,
            diff_taps: line.diff_taps,
            content: line.content.clone(),
            stripped_content: line.stripped_content.clone(),
        })
        .collect();

    outer
        .lines
        .splice(start_offset..start_offset + count, new_lines);
    outer.reindex();
}

/// 从 NewContent 构建 Line 列表
///
/// 使用 NewContent 中各行的 diff_taps 作为绝对缩进量计算实际 taps，
/// 生成 Line 结构用于插入。line_num 设为占位值，调用方通过 reindex 重算。
pub fn build_new_lines(content: &NewContent) -> Vec<Line> {
    const PLACEHOLDER_LINE_NUM: LineNumber = LineNumber::new(1);

    content
        .lines
        .iter()
        .map(|new_line| {
            let actual_taps = if new_line.is_raw {
                model::count_leading_spaces(&new_line.content)
            } else {
                new_line.diff_taps
            };
            let indented_content = if new_line.is_raw {
                new_line.content.clone()
            } else if actual_taps > 0 {
                format!("{:indent$}{}", "", new_line.content, indent = actual_taps)
            } else {
                new_line.content.clone()
            };
            let stripped = model::stripped_content(&indented_content);
            Line {
                line_num: PLACEHOLDER_LINE_NUM,
                taps: actual_taps,
                diff_taps: 0,
                content: indented_content,
                stripped_content: stripped,
            }
        })
        .collect()
}

/// 从 ContentBlock 中收集新增行的 (line_num, content) 信息
#[allow(dead_code)]
pub fn collect_new_line_info(
    block: &ContentBlock,
    insert_pos: usize,
    new_line_count: usize,
) -> Vec<(usize, String)> {
    let end = (insert_pos + new_line_count).min(block.lines.len());
    (insert_pos..end)
        .map(|i| {
            (
                block.lines[i].line_num.to_usize(),
                block.lines[i].content.clone(),
            )
        })
        .collect()
}

/// 从 FileContent 中收集新增行的 (line_num, content) 信息
pub fn collect_new_file_line_info(
    file: &FileContent,
    insert_pos: usize,
    new_line_count: usize,
) -> Vec<(usize, String)> {
    let end = (insert_pos + new_line_count).min(file.lines.len());
    (insert_pos..end)
        .map(|i| {
            (
                file.lines[i].line_num.to_usize(),
                file.lines[i].content.clone(),
            )
        })
        .collect()
}

/// 从 ContentBlock 中收集指定位置之前的上下文行（最多 CONTEXT_MAX_LINES 行）
pub fn collect_block_context_above(block: &ContentBlock, position: usize) -> Vec<DiffLine> {
    if position == 0 {
        return Vec::new();
    }
    let start = position.saturating_sub(CONTEXT_MAX_LINES);
    block.lines[start..position]
        .iter()
        .map(|line| DiffLine {
            kind: DiffLineKind::Unchanged,
            line_number: Some(line.line_num),
            content: line.content.clone(),
        })
        .collect()
}

/// 从 ContentBlock 中收集指定位置之后的上下文行（最多 CONTEXT_MAX_LINES 行）
pub fn collect_block_context_below(block: &ContentBlock, position: usize) -> Vec<DiffLine> {
    if position + 1 >= block.lines.len() {
        return Vec::new();
    }
    let end = (position + 1 + CONTEXT_MAX_LINES).min(block.lines.len());
    block.lines[position + 1..end]
        .iter()
        .map(|line| DiffLine {
            kind: DiffLineKind::Unchanged,
            line_number: Some(line.line_num),
            content: line.content.clone(),
        })
        .collect()
}

/// 重新为 FileContent 的所有行分配行号和重算 diff_taps，重建首行索引
pub fn reindex_file(file: &mut FileContent) {
    let base_taps = file.lines.first().map(|l| l.taps).unwrap_or(0);
    for (index, line) in file.lines.iter_mut().enumerate() {
        line.line_num = LineNumber::from_index(index);
        line.diff_taps = line.taps.saturating_sub(base_taps);
    }
    let mut index: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, line) in file.lines.iter().enumerate() {
        index
            .entry(line.stripped_content.clone())
            .or_default()
            .push(i);
    }
    file.first_line_index = index;
}

// ============================================================
// Diff 数据收集
// ============================================================

/// 获取 ContentBlock 的唯一标识
pub fn get_block_key(block: &ContentBlock) -> (usize, usize) {
    (block.start_line.to_usize(), block.end_line.to_usize())
}

/// 收集新增行的 diff 数据（changed + context），供调用方传给 record_diff_with_context
pub fn collect_added_diff_data(
    block: &ContentBlock,
    insert_pos: usize,
    new_line_count: usize,
) -> (Vec<DiffLine>, Vec<DiffLine>, Vec<DiffLine>) {
    let end_idx = (insert_pos + new_line_count).min(block.lines.len());
    let context_above = collect_block_context_above(block, insert_pos);
    let context_below = collect_block_context_below(block, end_idx.saturating_sub(1));
    let changed: Vec<DiffLine> = block.lines[insert_pos..end_idx]
        .iter()
        .map(|line| DiffLine {
            kind: DiffLineKind::Added,
            line_number: Some(line.line_num),
            content: line.content.clone(),
        })
        .collect();
    (changed, context_above, context_below)
}

/// 收集删除行的 diff 数据（changed + context），供调用方传给 record_diff_with_context
pub fn collect_deleted_diff_data(
    block: &ContentBlock,
    start_idx: usize,
    end_idx: usize,
) -> (Vec<DiffLine>, Vec<DiffLine>, Vec<DiffLine>) {
    let context_above = collect_block_context_above(block, start_idx);
    let context_below = collect_block_context_below(block, end_idx);
    let changed: Vec<DiffLine> = block.lines[start_idx..=end_idx]
        .iter()
        .map(|line| DiffLine {
            kind: DiffLineKind::Deleted,
            line_number: Some(line.line_num),
            content: line.content.clone(),
        })
        .collect();
    (changed, context_above, context_below)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DeleteContent, DeleteLine, Line, NewContent, NewLine};
    use std::collections::HashMap;

    /// 辅助函数：根据字符串切片构建 ContentBlock
    fn make_block(lines: &[&str], start_line: usize) -> ContentBlock {
        let mut block_lines: Vec<Line> = Vec::new();
        for (i, content) in lines.iter().enumerate() {
            let taps = model::count_leading_spaces(content);
            let stripped = model::stripped_content(content);
            block_lines.push(Line {
                line_num: LineNumber::new(start_line + i),
                taps,
                diff_taps: taps,
                content: content.to_string(),
                stripped_content: stripped,
            });
        }
        let end = start_line + lines.len() - 1;
        let mut block = ContentBlock {
            start_line: LineNumber::new(start_line),
            end_line: LineNumber::new(end),
            lines: block_lines,
            first_line_index: HashMap::new(),
            match_info: MatchInfo::Location {
                matched_line_count: lines.len(),
            },
        };
        block.reindex();
        block
    }

    fn make_file(lines: &[&str]) -> FileContent {
        let mut file_lines: Vec<Line> = Vec::new();
        let mut index: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, content) in lines.iter().enumerate() {
            let taps = model::count_leading_spaces(content);
            let stripped = model::stripped_content(content);
            index.entry(stripped.clone()).or_default().push(i);
            file_lines.push(Line {
                line_num: LineNumber::from_index(i),
                taps,
                diff_taps: 0,
                content: content.to_string(),
                stripped_content: stripped,
            });
        }
        FileContent {
            lines: file_lines,
            first_line_index: index,
        }
    }

    // ============================================================
    // find_delete_match 测试
    // ============================================================

    #[test]
    fn test_find_delete_match_single_line() {
        let block = make_block(&["aaa", "bbb", "ccc"], 1);
        let del = DeleteContent {
            lines: vec![DeleteLine {
                content: "bbb".to_string(),
                is_raw: false,
            }],
        };
        let result = find_delete_match(&block, &del);
        assert_eq!(result, Some((1, 1)));
    }

    #[test]
    fn test_find_delete_match_multi_line() {
        let block = make_block(&["aaa", "bbb", "ccc", "ddd"], 1);
        let del = DeleteContent {
            lines: vec![
                DeleteLine {
                    content: "bbb".to_string(),
                    is_raw: false,
                },
                DeleteLine {
                    content: "ccc".to_string(),
                    is_raw: false,
                },
            ],
        };
        let result = find_delete_match(&block, &del);
        assert_eq!(result, Some((1, 2)));
    }

    #[test]
    fn test_find_delete_match_not_found() {
        let block = make_block(&["aaa", "bbb"], 1);
        let del = DeleteContent {
            lines: vec![DeleteLine {
                content: "zzz".to_string(),
                is_raw: false,
            }],
        };
        let result = find_delete_match(&block, &del);
        assert_eq!(result, None);
    }

    #[test]
    fn test_find_delete_match_empty_del() {
        let block = make_block(&["aaa"], 1);
        let del = DeleteContent { lines: vec![] };
        assert_eq!(find_delete_match(&block, &del), None);
    }

    // ============================================================
    // check_delete_adjacency 测试
    // ============================================================

    #[test]
    fn test_check_delete_adjacency_ok() {
        let block = make_block(&["fn foo() {", "    bar();", "    baz();", "}"], 1);
        // matched_line_count = 2 (fn foo and bar)
        let mut block = block;
        block.match_info = MatchInfo::Location {
            matched_line_count: 2,
        };
        // Delete at index 2 (baz) is adjacent to location last (index 1, bar)
        assert!(check_delete_adjacency(&block, 2).is_ok());
    }

    #[test]
    fn test_check_delete_adjacency_only_empty_lines_in_gap() {
        let block = make_block(&["fn foo() {", "", "", "    baz();", "}"], 1);
        let mut block = block;
        block.match_info = MatchInfo::Location {
            matched_line_count: 1,
        };
        // Delete at index 3 (baz), gap has only empty lines → adjacent
        assert!(check_delete_adjacency(&block, 3).is_ok());
    }

    #[test]
    fn test_check_delete_adjacency_non_empty_gap() {
        let block = make_block(
            &[
                "fn foo() {",
                "    bar();",
                "    extra();",
                "    baz();",
                "}",
            ],
            1,
        );
        let mut block = block;
        block.match_info = MatchInfo::Location {
            matched_line_count: 1,
        };
        // Delete at index 3 (baz), gap has "    extra();" which is non-empty
        let result = check_delete_adjacency(&block, 3);
        assert!(result.is_err());
    }

    // ============================================================
    // apply_block_to_file 测试
    // ============================================================

    #[test]
    fn test_apply_block_to_file() {
        let mut file = make_file(&["line1", "line2", "line3"]);
        let block = make_block(&["line2_new", "line3_new"], 2);
        apply_block_to_file(&mut file, &block);
        assert_eq!(file.lines.len(), 3);
        assert_eq!(file.lines[0].content, "line1");
        assert_eq!(file.lines[1].content, "line2_new");
        assert_eq!(file.lines[2].content, "line3_new");
    }

    // ============================================================
    // build_new_lines 测试
    // ============================================================

    #[test]
    fn test_build_new_lines_basic() {
        let content = NewContent {
            base_taps: 0,
            lines: vec![
                NewLine {
                    diff_taps: 0,
                    content: "let x = 1;".to_string(),
                    is_raw: false,

                    expand_from_pool: None,
                },
                NewLine {
                    diff_taps: 4,
                    content: "let y = 2;".to_string(),
                    is_raw: false,

                    expand_from_pool: None,
                },
            ],
        };
        let lines = build_new_lines(&content);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].content, "let x = 1;");
        assert_eq!(lines[0].taps, 0);
        assert_eq!(lines[1].content, "    let y = 2;");
        assert_eq!(lines[1].taps, 4);
    }

    #[test]
    fn test_build_new_lines_raw() {
        let content = NewContent {
            base_taps: 0,
            lines: vec![NewLine {
                diff_taps: 4,
                content: "    raw_line".to_string(),
                is_raw: true,

                expand_from_pool: None,
            }],
        };
        let lines = build_new_lines(&content);
        assert_eq!(lines.len(), 1);
        // Raw: preserves original content exactly, taps from actual leading spaces
        assert_eq!(lines[0].content, "    raw_line");
        assert_eq!(lines[0].taps, 4);
    }

    // ============================================================
    // reindex_file 测试
    // ============================================================

    #[test]
    fn test_reindex_file() {
        let mut file = make_file(&["line1", "line2", "    line3"]);
        // Manually mess up line numbers
        file.lines[0].line_num = LineNumber::new(100);
        file.lines[1].line_num = LineNumber::new(200);
        reindex_file(&mut file);
        assert_eq!(file.lines[0].line_num, LineNumber::new(1));
        assert_eq!(file.lines[1].line_num, LineNumber::new(2));
        assert_eq!(file.lines[2].line_num, LineNumber::new(3));
        // first_line_index should be rebuilt
        assert!(file.first_line_index.contains_key("line3"));
    }

    // ============================================================
    // collect_block_context 测试
    // ============================================================

    #[test]
    fn test_collect_context_above() {
        let block = make_block(&["a", "b", "c", "d", "e"], 1);
        let ctx = collect_block_context_above(&block, 3);
        assert_eq!(ctx.len(), 3); // a, b, c
        assert_eq!(ctx[0].content, "a");
        assert_eq!(ctx[1].content, "b");
        assert_eq!(ctx[2].content, "c");
    }

    #[test]
    fn test_collect_context_below() {
        let block = make_block(&["a", "b", "c", "d", "e"], 1);
        let ctx = collect_block_context_below(&block, 1);
        assert_eq!(ctx.len(), 3); // c, d, e
        assert_eq!(ctx[0].content, "c");
    }

    // ============================================================
    // get_block_key 测试
    // ============================================================

    #[test]
    fn test_get_block_key() {
        let block = make_block(&["a", "b"], 10);
        assert_eq!(get_block_key(&block), (10, 11));
    }

    // ============================================================
    // Phase 3: 快照匹配函数测试 (CmdLine)
    // ============================================================

    fn make_snapshot(lines: &[&str]) -> Vec<CmdLine> {
        lines
            .iter()
            .enumerate()
            .map(|(i, s)| CmdLine {
                line_num: i + 1,
                content: s.to_string(),

                expand_from_pool: None,
            })
            .collect()
    }

    #[test]
    fn test_find_delete_match_in_snapshot_single_line() {
        let snapshot = make_snapshot(&["aaa", "bbb", "ccc"]);
        let del = DeleteContent {
            lines: vec![DeleteLine {
                content: "bbb".to_string(),
                is_raw: false,
            }],
        };
        let result = find_delete_match_in_snapshot(&snapshot, &del);
        assert_eq!(result, Some((1, 1)));
    }

    #[test]
    fn test_find_delete_match_in_snapshot_multi_line() {
        let snapshot = make_snapshot(&["aaa", "   bbb", "   ccc", "ddd"]);
        let del = DeleteContent {
            lines: vec![
                DeleteLine {
                    content: "   bbb".to_string(),
                    is_raw: false,
                },
                DeleteLine {
                    content: "   ccc".to_string(),
                    is_raw: false,
                },
            ],
        };
        let result = find_delete_match_in_snapshot(&snapshot, &del);
        assert_eq!(result, Some((1, 2)));
    }

    #[test]
    fn test_find_delete_match_in_snapshot_not_found() {
        let snapshot = make_snapshot(&["aaa", "bbb"]);
        let del = DeleteContent {
            lines: vec![DeleteLine {
                content: "zzz".to_string(),
                is_raw: false,
            }],
        };
        let result = find_delete_match_in_snapshot(&snapshot, &del);
        assert_eq!(result, None);
    }

    #[test]
    fn test_find_delete_match_in_snapshot_empty_del() {
        let snapshot = make_snapshot(&["aaa"]);
        let del = DeleteContent { lines: vec![] };
        assert_eq!(find_delete_match_in_snapshot(&snapshot, &del), None);
    }

    #[test]
    fn test_check_delete_adjacency_in_snapshot_ok() {
        let snapshot = make_snapshot(&["fn foo() {", "    bar();", "    baz();", "}"]);
        // matched_line_count = 2, Delete starts at 2 (baz) → adjacent to location last (1)
        assert!(check_delete_adjacency_in_snapshot(&snapshot, 2, 2).is_ok());
    }

    #[test]
    fn test_check_delete_adjacency_in_snapshot_only_empty_gap() {
        let snapshot = make_snapshot(&["fn foo() {", "", "", "    baz();", "}"]);
        // matched_line_count = 1, Delete at 3, gap has only empty lines → adjacent
        assert!(check_delete_adjacency_in_snapshot(&snapshot, 1, 3).is_ok());
    }

    #[test]
    fn test_check_delete_adjacency_in_snapshot_non_empty_gap() {
        let snapshot = make_snapshot(&[
            "fn foo() {",
            "    bar();",
            "    extra();",
            "    baz();",
            "}",
        ]);
        // matched_line_count = 1, Delete at 3 (baz), gap has "extra()" → error
        let result = check_delete_adjacency_in_snapshot(&snapshot, 1, 3);
        assert!(result.is_err());
    }

    #[test]
    fn test_map_snapshot_index_to_block_index() {
        let block = make_block(&["aaa", "   bbb", "   ccc"], 1);
        let snapshot = make_snapshot(&["aaa", "   bbb", "   ccc"]);
        assert_eq!(
            map_snapshot_index_to_block_index(&block, &snapshot, 0),
            Some(0)
        );
        assert_eq!(
            map_snapshot_index_to_block_index(&block, &snapshot, 1),
            Some(1)
        );
        assert_eq!(
            map_snapshot_index_to_block_index(&block, &snapshot, 2),
            Some(2)
        );
    }

    #[test]
    fn test_map_snapshot_index_to_block_index_not_found() {
        let block = make_block(&["aaa", "bbb"], 1);
        let snapshot = make_snapshot(&["zzz"]);
        assert_eq!(
            map_snapshot_index_to_block_index(&block, &snapshot, 0),
            None
        );
    }
}
