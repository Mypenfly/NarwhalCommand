//! 核心匹配算法 (Location Matcher)
//!
//! 负责根据 LocationContent 在 FileContent 中查找唯一匹配的代码位置。
//!
//! ## 实现逻辑
//!
//! 1. 取 LocationContent 首行去空白后，在 FileContent 中扫描匹配的候选起点
//! 2. 对每个候选起点，逐行比对 content（去空白）和 diff_taps
//! 3. 确认结果唯一性，否则返回详细的匹配错误
//!
//! ## 对应文档
//!
//! 详见 INSTRUCTION.md 第 3.1 节 "Location 匹配算法"

use crate::block::BlockParser;
use crate::error::MatchError;
use crate::model::{self, ContentBlock, FileContent, Line, LocationContent, MatchInfo};

/// Location 匹配器
///
/// 根据 LocationContent 在 FileContent 中执行精确匹配，返回唯一 ContentBlock。
pub struct LocationMatcher;

impl LocationMatcher {
    /// 在文件内容中执行 Location 匹配，返回唯一 ContentBlock
    ///
    /// 匹配过程：
    /// 1. 首行去空白匹配 → 收集候选起点
    /// 2. 逐行比对（content 去空白 + diff_taps）→ 筛选
    /// 3. 确认唯一性 → 返回 ContentBlock
    ///
    /// 若 `block` 为 true（Location:Block），使用 BlockParser 获取精确 Block 边界，
    /// 而非"从首行到文件末尾"的默认行为。
    pub fn find_unique_block(
        file: &FileContent,
        location: &LocationContent,
        block: bool,
    ) -> Result<ContentBlock, MatchError> {
        if location.lines.is_empty() {
            // 空 Location — 使用整个搜索范围作为 ContentBlock
            // 对应嵌套 Location 的场景：`//!@Location:` 后紧跟 `//!@Delete:`
            // 此时不缩小范围，后续 New/Delete 在整个文件/父Block 中操作
            let lines: Vec<Line> = file
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
            return Ok(ContentBlock {
                start_line: 1,
                end_line: file.lines.len(),
                lines,
                match_info: MatchInfo::Empty,
            });
        }
        let candidates = collect_first_line_matches(file, location);
        let filtered = filter_by_full_match(file, candidates, location);
        expect_single_match(file, filtered, location, block)
    }
}

/// 收集首行匹配的所有候选起点
///
/// 使用 FileContent 的 first_line_index 进行 O(1) 查找，
/// 避免每次匹配都全量扫描文件行。
fn collect_first_line_matches(file: &FileContent, location: &LocationContent) -> Vec<usize> {
    let target = model::stripped_content(&location.lines[0].content);
    file.first_line_index
        .get(&target)
        .cloned()
        .unwrap_or_default()
}

/// 对候选集进行逐行全量匹配筛选
fn filter_by_full_match(
    file: &FileContent,
    candidates: Vec<usize>,
    location: &LocationContent,
) -> Vec<usize> {
    candidates
        .into_iter()
        .filter(|&start_index| rows_match(file, start_index, location))
        .collect()
}

/// 逐行比对：content（去空白）+ diff_taps 双重校验
fn rows_match(file: &FileContent, start_index: usize, location: &LocationContent) -> bool {
    let loc_lines = &location.lines;
    let location_line_count = loc_lines.len();

    // 检查文件是否有足够的行
    if start_index + location_line_count > file.lines.len() {
        return false;
    }

    // 构建文件侧的候选行（计算 diff_taps 相对于首行）
    let file_slice = &file.lines[start_index..start_index + location_line_count];
    let base_taps = file_slice[0].taps;

    // 逐行比对（跳过双方的空行）
    let mut file_index: usize = 0;
    let mut loc_index: usize = 0;

    while file_index < file_slice.len() && loc_index < loc_lines.len() {
        let file_line = &file_slice[file_index];
        let loc_line = &loc_lines[loc_index];

        let file_is_empty = file_line.content.trim().is_empty();
        let loc_is_empty = loc_line.content.trim().is_empty();

        // 跳过双方都为空的行
        if file_is_empty && loc_is_empty {
            file_index += 1;
            loc_index += 1;
            continue;
        }

        // 若仅一侧为空，跳过空的那一侧
        if file_is_empty {
            file_index += 1;
            continue;
        }
        if loc_is_empty {
            loc_index += 1;
            continue;
        }

        // 比对去空白后的内容
        if file_line.stripped_content() != model::stripped_content(&loc_line.content) {
            return false;
        }

        // 比对 diff_taps
        let file_diff = file_line.taps.saturating_sub(base_taps);
        let loc_diff = loc_line.diff_taps.unwrap_or(0);

        if file_diff != loc_diff {
            return false;
        }

        file_index += 1;
        loc_index += 1;
    }

    true
}

/// 确认匹配结果唯一，否则构造详细错误信息
fn expect_single_match(
    file: &FileContent,
    candidates: Vec<usize>,
    location: &LocationContent,
    block: bool,
) -> Result<ContentBlock, MatchError> {
    match candidates.len() {
        0 => Err(MatchError::NoMatch {
            location_content: format_location_for_error(location),
        }),
        1 => build_content_block(file, candidates[0], location.lines.len(), block),
        n => {
            let candidate_descriptions: Vec<String> = candidates
                .iter()
                .take(3)
                .map(|&idx| {
                    let line_num = idx + 1;
                    let snippet = &file.lines[idx].content;
                    let truncated: String = if snippet.len() > 60 {
                        format!("{}...", &snippet[..57])
                    } else {
                        snippet.clone()
                    };
                    format!("  L{}: {}", line_num, truncated)
                })
                .collect();
            Err(MatchError::TooManyMatches {
                count: n,
                candidates: candidate_descriptions,
                location_content: format_location_for_error(location),
            })
        }
    }
}

/// 从匹配起点构建 ContentBlock
///
/// 若 `block` 为 false（普通 Location）：block 边界为从 start_index 到文件末尾。
/// 若 `block` 为 true（Location:Block）：使用 BlockParser 获取精确的代码块边界。
fn build_content_block(
    file: &FileContent,
    start_index: usize,
    matched_line_count: usize,
    block: bool,
) -> Result<ContentBlock, MatchError> {
    let (block_start, block_end) = if block {
        // Phase 3: 使用 BlockParser 获取精确 Block 边界
        BlockParser::parse_block(file, start_index)?
    } else {
        (start_index, file.lines.len() - 1)
    };

    let start_line = block_start + 1;
    let end_line = block_end + 1;
    let lines: Vec<Line> = file.lines[block_start..=block_end]
        .iter()
        .map(|line| Line {
            line_num: line.line_num,
            taps: line.taps,
            diff_taps: line.diff_taps,
            content: line.content.clone(),
            stripped_content: line.stripped_content.clone(),
        })
        .collect();

    // Location:Block 时应以整个 Block 的行数为匹配行数，
    // 这样 New:Normal 会插入到 Block 之后而非 Block 内部
    let effective_matched = if block {
        lines.len()
    } else {
        matched_line_count
    };

    let match_info = if effective_matched == 0 {
        MatchInfo::Empty
    } else {
        MatchInfo::Location {
            matched_line_count: effective_matched,
        }
    };

    Ok(ContentBlock {
        start_line,
        end_line,
        lines,
        match_info,
    })
}

/// 将 LocationContent 格式化为错误信息中的字符串
fn format_location_for_error(location: &LocationContent) -> String {
    location
        .lines
        .iter()
        .map(|line| line.content.as_str())
        .collect::<Vec<&str>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::MatchError;
    use crate::model::{self, FileContent, Line, LocationContent, LocationLine};
    use std::collections::HashMap;

    /// 辅助函数：根据字符串切片构建简单的 FileContent
    fn make_file_content(lines: &[&str]) -> FileContent {
        let mut file_lines: Vec<Line> = Vec::new();
        let mut index: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, content) in lines.iter().enumerate() {
            let taps = model::count_leading_spaces(content);
            let stripped = model::stripped_content(content);
            index.entry(stripped.clone()).or_default().push(i);
            file_lines.push(Line {
                line_num: i + 1,
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

    /// 辅助函数：根据字符串切片构建 LocationContent
    fn make_location_content(lines: &[&str]) -> LocationContent {
        if lines.is_empty() {
            return LocationContent { lines: vec![] };
        }
        let base_taps = model::count_leading_spaces(lines[0]);
        let loc_lines: Vec<LocationLine> = lines
            .iter()
            .enumerate()
            .map(|(i, content)| {
                let line_taps = model::count_leading_spaces(content);
                let diff_taps = Some(line_taps.saturating_sub(base_taps));
                LocationLine {
                    index: i,
                    diff_taps,
                    content: content.to_string(),
                    line_num: None,
                }
            })
            .collect();
        LocationContent { lines: loc_lines }
    }

    // ============================================================
    // find_unique_block — 基本匹配测试
    // ============================================================

    #[test]
    fn test_find_unique_block_exact_match() {
        let file = make_file_content(&[
            "// comment",
            "fn main() {",
            "    let x = 1;",
            "    println!(\"{}\", x);",
            "}",
        ]);

        let location = make_location_content(&["fn main() {", "    let x = 1;"]);

        let result = LocationMatcher::find_unique_block(&file, &location, false);
        assert!(result.is_ok());
        let block = result.unwrap();
        assert_eq!(block.start_line, 2);
        assert_eq!(block.lines.len(), 4); // from line 2 to end of file
        assert_eq!(block.lines[0].content, "fn main() {");
        assert_eq!(block.lines[1].content, "    let x = 1;");
    }

    #[test]
    fn test_find_unique_block_single_line_location() {
        let file = make_file_content(&["fn foo() {}", "fn bar() {}", "fn baz() {}"]);

        let location = make_location_content(&["fn bar() {}"]);

        let result = LocationMatcher::find_unique_block(&file, &location, false);
        assert!(result.is_ok());
        let block = result.unwrap();
        assert_eq!(block.start_line, 2);
        assert_eq!(block.lines[0].content, "fn bar() {}");
    }

    #[test]
    fn test_find_unique_block_no_match() {
        let file = make_file_content(&["fn foo() {}", "fn bar() {}"]);

        let location = make_location_content(&["fn nonexistent() {}"]);

        let result = LocationMatcher::find_unique_block(&file, &location, false);
        assert!(result.is_err());
        match result.unwrap_err() {
            MatchError::NoMatch { .. } => {} // expected
            _ => panic!("Expected NoMatch error"),
        }
    }

    #[test]
    fn test_find_unique_block_too_many_matches() {
        let file = make_file_content(&[
            "fn foo() {",
            "    bar();",
            "}",
            "",
            "fn foo() {",
            "    baz();",
            "}",
        ]);

        let location = make_location_content(&["fn foo() {"]);

        let result = LocationMatcher::find_unique_block(&file, &location, false);
        assert!(result.is_err());
        match result.unwrap_err() {
            MatchError::TooManyMatches { count, .. } => {
                assert_eq!(count, 2);
            }
            _ => panic!("Expected TooManyMatches error"),
        }
    }

    // ============================================================
    // find_unique_block — 去空白匹配测试
    // ============================================================

    #[test]
    fn test_find_unique_block_stripped_content_match() {
        let file = make_file_content(&[
            "// file starts",
            "    fn main() {",
            "        let x = 1;",
            "    }",
        ]);

        let location = make_location_content(&["fn main() {", "    let x = 1;"]);

        let result = LocationMatcher::find_unique_block(&file, &location, false);
        assert!(result.is_ok());
        let block = result.unwrap();
        assert_eq!(block.start_line, 2);
    }

    #[test]
    fn test_find_unique_block_disambiguates_by_second_line() {
        let file = make_file_content(&[
            "fn foo() {",
            "    let a = 1;",
            "}",
            "",
            "fn foo() {",
            "    let b = 2;",
            "}",
        ]);

        let location = make_location_content(&["fn foo() {", "    let b = 2;"]);

        let result = LocationMatcher::find_unique_block(&file, &location, false);
        assert!(result.is_ok());
        let block = result.unwrap();
        assert_eq!(block.start_line, 5);
    }

    // ============================================================
    // find_unique_block — Block 边界测试 (Phase 1: 到文件末尾)
    // ============================================================

    #[test]
    fn test_find_unique_block_boundary_to_end_of_file() {
        let file = make_file_content(&[
            "// header",
            "mod utils;",
            "",
            "fn process() {",
            "    do_work();",
            "}",
            "",
            "fn main() {",
            "    process();",
            "}",
        ]);

        let location = make_location_content(&["fn process() {"]);

        let result = LocationMatcher::find_unique_block(&file, &location, false);
        assert!(result.is_ok());
        let block = result.unwrap();
        assert_eq!(block.start_line, 4);
        assert_eq!(block.lines.len(), 7); // lines 4-10
    }

    // ============================================================
    // find_unique_block — 空行处理测试
    // ============================================================

    #[test]
    fn test_find_unique_block_skips_empty_lines_in_location() {
        let file = make_file_content(&["fn main() {", "", "    let x = 1;", "}"]);

        let location = make_location_content(&["fn main() {", "", "    let x = 1;"]);

        let result = LocationMatcher::find_unique_block(&file, &location, false);
        assert!(result.is_ok());
        let block = result.unwrap();
        assert_eq!(block.start_line, 1);
    }
}
