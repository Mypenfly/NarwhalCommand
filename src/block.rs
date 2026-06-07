//! Block 解析器 (Block Parser)
//!
//! 负责通过逐字符扫描构建代码层级树，准确识别代码块边界。
//!
//! ## 实现逻辑
//!
//! 1. 判断目标语言类型（花括号语言 / 缩进语言）
//! 2. 花括号语言：逐字符扫描，维护 depth/in_string/in_comment 状态
//! 3. 缩进语言：基于缩进层级判断 Block 边界
//!
//! ## 对应文档
//!
//! 详见 INSTRUCTION.md 第 3.2 节 "Block 解析算法"

use crate::error::MatchError;
use crate::model::FileContent;

/// 目标语言类型
#[derive(Debug, PartialEq)]
pub enum Language {
    /// 花括号语言（Rust, C, JS, Java 等）
    Brace,
    /// 缩进语言（Python, YAML 等）
    Indent,
    /// 无法识别的语言类型
    Unknown,
}

/// Block 解析器
///
/// 根据匹配到的位置和文件内容，识别代码块边界。
pub struct BlockParser;

impl BlockParser {
    /// 检测目标语言类型
    ///
    /// 按优先级依次检查：
    /// 1. 检查首行及上下文是否包含 `{` `}` 结构 → 花括号语言
    /// 2. 检查内容 diff_taps 是否不全为 0（有缩进层级）→ 缩进语言
    /// 3. 其他 → Block 不可解析
    pub fn detect_language(file: &FileContent, start_index: usize) -> Language {
        let check_end = (start_index + 5).min(file.lines.len());
        for line_idx in start_index..check_end {
            let content = &file.lines[line_idx].content;
            let trimmed = content.trim();
            // 跳过注释行
            if trimmed.starts_with("//") || trimmed.starts_with('#') {
                continue;
            }
            if content.contains('{') || content.contains('}') {
                return Language::Brace;
            }
        }

        // 检查是否有缩进层级
        let base_taps = file.lines[start_index].taps;
        let indent_end = (start_index + 20).min(file.lines.len());
        for line_idx in (start_index + 1)..indent_end {
            let line = &file.lines[line_idx];
            let trimmed = line.content.trim();
            if !trimmed.is_empty()
                && !trimmed.starts_with("//")
                && !trimmed.starts_with('#')
                && line.taps > base_taps
            {
                return Language::Indent;
            }
        }

        Language::Unknown
    }

    /// 花括号语言 — 解析代码块边界
    ///
    /// 从 start_index 行开始逐字符扫描，找到第一个 `{`（作为 block 起始），
    /// 然后追踪其匹配的 `}`，返回 (start_line_index, end_line_index)。
    ///
    /// 维护 `depth`（括号深度）、`in_string`、`in_comment` 状态，
    /// 正确处理 `\"`、`\\` 转义，`//` 行注释和 `/* */` 块注释。
    pub fn parse_brace_block(
        file: &FileContent,
        start_index: usize,
    ) -> Result<(usize, usize), MatchError> {
        let mut depth: i32 = 0;
        let mut in_string = false;
        let mut in_block_comment = false;
        let mut block_start_line: Option<usize> = None;

        for line_idx in start_index..file.lines.len() {
            let content = &file.lines[line_idx].content;
            let chars: Vec<char> = content.chars().collect();
            let mut i = 0;
            let mut in_line_comment = false;

            while i < chars.len() {
                let c = chars[i];

                if in_line_comment {
                    i += 1;
                    continue;
                }
                if in_block_comment {
                    if c == '*' && i + 1 < chars.len() && chars[i + 1] == '/' {
                        in_block_comment = false;
                        i += 2;
                        continue;
                    }
                    i += 1;
                    continue;
                }
                if in_string {
                    if c == '\\' && i + 1 < chars.len() {
                        i += 2;
                        continue;
                    }
                    if c == '"' {
                        in_string = false;
                    }
                    i += 1;
                    continue;
                }

                if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
                    in_line_comment = true;
                    i += 2;
                    continue;
                }
                if c == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
                    in_block_comment = true;
                    i += 2;
                    continue;
                }
                if c == '"' {
                    in_string = true;
                    i += 1;
                    continue;
                }

                if c == '{' {
                    if depth == 0 {
                        depth = 1;
                        block_start_line = Some(line_idx);
                    } else {
                        depth += 1;
                    }
                } else if c == '}' {
                    depth -= 1;
                    if depth == 0 {
                        if let Some(start) = block_start_line {
                            return Ok((start, line_idx));
                        }
                    }
                }

                i += 1;
            }
        }

        if block_start_line.is_none() {
            let snippet = file
                .lines
                .get(start_index)
                .map(|l| l.content.as_str())
                .unwrap_or("");
            return Err(MatchError::BlockNotParseable {
                location_content: snippet.to_string(),
            });
        }

        // 括号未闭合（文件末尾），返回到文件末尾
        let start = match block_start_line {
            Some(s) => s,
            None => return Ok((start_index, file.lines.len() - 1)),
        };
        Ok((start, file.lines.len() - 1))
    }

    /// 缩进语言 — 解析代码块边界
    ///
    /// 从 start_index 行开始，以首行的 taps 为基准缩进量，
    /// 向后逐行扫描，跳过空行和纯注释行。
    /// taps > base_taps → 在 Block 内
    /// taps <= base_taps → Block 结束
    pub fn parse_indent_block(file: &FileContent, start_index: usize) -> (usize, usize) {
        let base_taps = file.lines[start_index].taps;
        let mut end_index = file.lines.len() - 1;

        for line_idx in (start_index + 1)..file.lines.len() {
            let line = &file.lines[line_idx];
            let trimmed = line.content.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
                continue;
            }
            if line.taps <= base_taps {
                end_index = line_idx.saturating_sub(1);
                break;
            }
        }

        (start_index, end_index)
    }

    /// 解析代码块边界，自动检测语言类型
    ///
    /// 根据语言类型分派到对应的解析方法。
    /// 若语言不可识别则返回 BlockNotParseable 错误。
    pub fn parse_block(
        file: &FileContent,
        start_index: usize,
    ) -> Result<(usize, usize), MatchError> {
        match Self::detect_language(file, start_index) {
            Language::Brace => Self::parse_brace_block(file, start_index),
            Language::Indent => Ok(Self::parse_indent_block(file, start_index)),
            Language::Unknown => {
                let snippet = file
                    .lines
                    .get(start_index)
                    .map(|l| l.content.as_str())
                    .unwrap_or("");
                Err(MatchError::BlockNotParseable {
                    location_content: snippet.to_string(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{self, FileContent, Line};
    use std::collections::HashMap;

    /// 辅助函数：根据字符串切片构建 FileContent
    fn make_file(lines: &[&str]) -> FileContent {
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

    // ============================================================
    // detect_language 测试
    // ============================================================

    #[test]
    fn test_detect_language_brace() {
        let file = make_file(&["fn example() {", "    let x = 1;", "}"]);
        assert_eq!(BlockParser::detect_language(&file, 0), Language::Brace);
    }

    #[test]
    fn test_detect_language_brace_with_struct() {
        let file = make_file(&["struct Foo {", "    x: i32,", "    y: i32,", "}"]);
        assert_eq!(BlockParser::detect_language(&file, 0), Language::Brace);
    }

    #[test]
    fn test_detect_language_indent_python() {
        let file = make_file(&[
            "def example():",
            "    x = 1",
            "    y = 2",
            "",
            "def other():",
            "    pass",
        ]);
        assert_eq!(BlockParser::detect_language(&file, 0), Language::Indent);
    }

    #[test]
    fn test_detect_language_indent_yaml() {
        let file = make_file(&["key:", "  sub1: value1", "  sub2: value2"]);
        assert_eq!(BlockParser::detect_language(&file, 0), Language::Indent);
    }

    #[test]
    fn test_detect_language_unknown_plain_text() {
        let file = make_file(&["# Title", "## Section", "Some text."]);
        assert_eq!(BlockParser::detect_language(&file, 0), Language::Unknown);
    }

    // ============================================================
    // parse_brace_block 测试
    // ============================================================

    #[test]
    fn test_parse_brace_block_simple() {
        let file = make_file(&[
            "fn main() {",
            "    println!(\"hi\");",
            "}",
            "",
            "fn other() {",
            "    stuff();",
            "}",
        ]);
        let result = BlockParser::parse_brace_block(&file, 0);
        assert!(result.is_ok());
        let (start, end) = result.unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, 2);
    }

    #[test]
    fn test_parse_brace_block_nested() {
        let file = make_file(&[
            "fn outer() {",
            "    if true {",
            "        inner();",
            "    }",
            "    more();",
            "}",
            "",
            "fn next() {}",
        ]);
        let result = BlockParser::parse_brace_block(&file, 0);
        assert!(result.is_ok());
        let (start, end) = result.unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, 5);
    }

    #[test]
    fn test_parse_brace_block_with_strings() {
        let file = make_file(&[
            "fn main() {",
            "    let s = \"this has { braces } inside\";",
            "    let t = \"and } more\";",
            "    println!(\"{}\", s);",
            "}",
        ]);
        let result = BlockParser::parse_brace_block(&file, 0);
        assert!(result.is_ok());
        let (start, end) = result.unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, 4);
    }

    #[test]
    fn test_parse_brace_block_with_escaped_quotes() {
        let file = make_file(&[
            "fn main() {",
            "    let s = \"escaped \\\" quote\";",
            "    let t = \"backslash \\\\ end\";",
            "}",
        ]);
        let result = BlockParser::parse_brace_block(&file, 0);
        assert!(result.is_ok());
        let (start, end) = result.unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, 3);
    }

    #[test]
    fn test_parse_brace_block_with_line_comments() {
        let file = make_file(&[
            "fn main() {",
            "    // this { is a comment",
            "    let x = 1;",
            "    // another } comment",
            "}",
        ]);
        let result = BlockParser::parse_brace_block(&file, 0);
        assert!(result.is_ok());
        let (start, end) = result.unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, 4);
    }

    #[test]
    fn test_parse_brace_block_with_block_comments() {
        let file = make_file(&[
            "fn main() {",
            "    /* this { is",
            "       a block } comment */",
            "    let x = 1;",
            "}",
        ]);
        let result = BlockParser::parse_brace_block(&file, 0);
        assert!(result.is_ok());
        let (start, end) = result.unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, 4);
    }

    #[test]
    fn test_parse_brace_block_no_brace_returns_error() {
        let file = make_file(&["# Title", "Some text without braces"]);
        let result = BlockParser::parse_brace_block(&file, 0);
        assert!(result.is_err());
        match result.unwrap_err() {
            MatchError::BlockNotParseable { .. } => {}
            _ => panic!("Expected BlockNotParseable"),
        }
    }

    #[test]
    fn test_parse_brace_block_start_on_second_line() {
        // The brace is not on the start_index line, but on the next line
        let file = make_file(&["fn example()", "{", "    do_stuff();", "}"]);
        let result = BlockParser::parse_brace_block(&file, 0);
        assert!(result.is_ok());
        let (start, end) = result.unwrap();
        assert_eq!(start, 1);
        assert_eq!(end, 3);
    }

    #[test]
    fn test_parse_brace_block_unclosed_at_eof() {
        let file = make_file(&[
            "fn main() {",
            "    unfinished();",
            "    // no closing brace",
        ]);
        let result = BlockParser::parse_brace_block(&file, 0);
        assert!(result.is_ok());
        let (start, end) = result.unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, 2); // extends to end of file
    }

    // ============================================================
    // parse_indent_block 测试
    // ============================================================

    #[test]
    fn test_parse_indent_block_simple() {
        let file = make_file(&[
            "def example():",
            "    x = 1",
            "    y = 2",
            "",
            "def other():",
            "    pass",
        ]);
        let (start, end) = BlockParser::parse_indent_block(&file, 0);
        assert_eq!(start, 0);
        assert_eq!(end, 3);
    }

    #[test]
    fn test_parse_indent_block_with_comments_and_blanks() {
        let file = make_file(&[
            "def process_data(items):",
            "    # Validate input",
            "    if not items:",
            "        return []",
            "",
            "    # Process each item",
            "    results = []",
            "    for item in items:",
            "        # Skip empty items",
            "        if item:",
            "            results.append(item.upper())",
            "",
            "    return results",
            "",
            "# This is a top-level comment",
            "def other_func():",
            "    pass",
        ]);
        let (start, end) = BlockParser::parse_indent_block(&file, 0);
        assert_eq!(start, 0);
        assert_eq!(end, 14);
    }

    // ============================================================
    // 非 Block 可解析 — 拒绝测试
    // ============================================================

    #[test]
    fn test_parse_block_markdown_rejected() {
        let file = make_file(&[
            "# Project Title",
            "",
            "## Installation",
            "",
            "Run the following command:",
            "",
            "```bash",
            "cargo build --release",
            "```",
            "",
            "## Usage",
            "",
            "See the [documentation](docs/).",
        ]);
        let result = BlockParser::parse_block(&file, 0);
        assert!(result.is_err());
        match result.unwrap_err() {
            MatchError::BlockNotParseable { .. } => {}
            _ => panic!("Expected BlockNotParseable for markdown"),
        }
    }

    #[test]
    fn test_parse_block_plain_text_rejected() {
        let file = make_file(&[
            "This is a plain text file.",
            "It has no code structure.",
            "No braces, no indentation.",
        ]);
        let result = BlockParser::parse_block(&file, 0);
        assert!(result.is_err());
        match result.unwrap_err() {
            MatchError::BlockNotParseable { .. } => {}
            _ => panic!("Expected BlockNotParseable for plain text"),
        }
    }

    #[test]
    fn test_parse_brace_block_within_middle_of_file() {
        // Test that block detection works correctly when the block
        // doesn't start at file index 0
        let file = make_file(&[
            "// Header comment",
            "use std::collections::HashMap;",
            "",
            "pub fn get_or_insert<K, V>(map: &mut HashMap<K, V>, key: K, default: V) -> &mut V",
            "where",
            "    K: Eq + Hash,",
            "{",
            "    if !map.contains_key(&key) {",
            "        map.insert(key, default);",
            "    }",
            "    map.get_mut(&key).unwrap()",
            "}",
            "",
            "// Another function",
            "pub fn clear_cache() {",
            "    // ...",
            "}",
        ]);
        let result = BlockParser::parse_brace_block(&file, 3);
        assert!(result.is_ok());
        let (start, end) = result.unwrap();
        assert_eq!(start, 6); // first '{' is on line index 6
        assert_eq!(end, 11); // matching '}'
    }
}
