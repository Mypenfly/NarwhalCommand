//! Block 解析器 (Block)
//!
//! 负责解析代码块边界（花括号块 / 缩进块），
//! 用于 Location:Block 指令的精确定位。
//!
//! ## 实现逻辑
//!
//! 1. detect_language: 判断代码语言类型（花括号 / 缩进 / Unknown）
//! 2. parse_brace_block: 逐字符扫描，处理 depth/in_string/in_comment
//! 3. parse_indent_block: 基于 taps 层级判断边界
//!
//! ## 对应文档
//!
//! 详见 INSTRUCTION.md §3.2 "Block 解析算法", n_edit_dev.md Location:Block 章节
//!
//! ## 迁移来源
//!
//! 从 n_edit/src/block.rs 迁移。

use crate::error::MatchError;
use crate::model::{SearchScope, INDENT_DETECT_WINDOW, LANGUAGE_DETECT_WINDOW};

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
/// 根据匹配到的位置和搜索范围，识别代码块边界。
pub struct BlockParser;

impl BlockParser {
    /// 检测目标语言类型
    ///
    /// 按优先级依次检查：
    /// 1. 检查首行及上下文是否包含 `{` `}` 结构 → 花括号语言
    /// 2. 检查内容 diff_taps 是否不全为 0（有缩进层级）→ 缩进语言
    /// 3. 其他 → Block 不可解析
    pub fn detect_language(scope: &SearchScope, start_index: usize) -> Language {
        let scoped_lines = scope.lines();
        let check_end = (start_index + LANGUAGE_DETECT_WINDOW).min(scoped_lines.len());
        for line in scoped_lines.iter().take(check_end).skip(start_index) {
            let content = &line.content;
            let trimmed = content.trim();
            if trimmed.starts_with("//") || trimmed.starts_with('#') {
                continue;
            }
            if content.contains('{') || content.contains('}') {
                return Language::Brace;
            }
        }

        let base_taps = scoped_lines[start_index].taps;
        let indent_end = (start_index + INDENT_DETECT_WINDOW).min(scoped_lines.len());
        for line in scoped_lines.iter().take(indent_end).skip(start_index + 1) {
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
    /// 内部使用 BraceScanner 封装修复扫描状态，逐行处理。
    ///
    /// 返回的索引相对于搜索范围（scope.lines()）。
    pub fn parse_brace_block(
        scope: &SearchScope,
        start_index: usize,
    ) -> Result<(usize, usize), MatchError> {
        let scoped_lines = scope.lines();
        let mut scanner = BraceScanner::new();

        for (line_idx, line) in scoped_lines.iter().enumerate().skip(start_index) {
            let block_ended = scanner.scan_line(&line.content, line_idx);

            if block_ended {
                if let Some(start) = scanner.block_start_line {
                    return Ok((start, line_idx));
                }
            }
        }

        if scanner.block_start_line.is_none() {
            let snippet = scoped_lines
                .get(start_index)
                .map(|l| l.content.as_str())
                .unwrap_or("");
            return Err(MatchError::BlockNotParseable {
                location_content: snippet.to_string(),
            });
        }

        let start = scanner.block_start_line.unwrap_or(start_index);
        Ok((start, scoped_lines.len().saturating_sub(1)))
    }

    /// 缩进语言 — 解析代码块边界
    ///
    /// 从 start_index 行开始，以首行的 taps 为基准缩进量，
    /// 向后逐行扫描，跳过空行和纯注释行。
    /// taps > base_taps → 在 Block 内
    /// taps <= base_taps → Block 结束
    ///
    /// 返回的索引相对于搜索范围（scope.lines()）。
    pub fn parse_indent_block(scope: &SearchScope, start_index: usize) -> (usize, usize) {
        let scoped_lines = scope.lines();
        let base_taps = scoped_lines[start_index].taps;
        let mut end_index = scoped_lines.len().saturating_sub(1);

        for (line_idx, line) in scoped_lines.iter().enumerate().skip(start_index + 1) {
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
    ///
    /// 返回的索引相对于搜索范围（scope.lines()）。
    pub fn parse_block(
        scope: &SearchScope,
        start_index: usize,
    ) -> Result<(usize, usize), MatchError> {
        match Self::detect_language(scope, start_index) {
            Language::Brace => Self::parse_brace_block(scope, start_index),
            Language::Indent => Ok(Self::parse_indent_block(scope, start_index)),
            Language::Unknown => {
                let snippet = scope
                    .lines()
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

/// 花括号扫描器的内部状态
///
/// 维护逐字符扫描时的 depth、字符串/注释上下文等状态信息。
struct BraceScanner {
    /// 括号嵌套深度（0 = 在 block 外，1+ = 在 block 内）
    depth: i32,
    /// 是否在字符串字面量内部
    in_string: bool,
    /// 是否在块注释 `/* */` 内部
    in_block_comment: bool,
    /// 第一个 `{` 所在的行索引
    block_start_line: Option<usize>,
}

impl BraceScanner {
    fn new() -> Self {
        BraceScanner {
            depth: 0,
            in_string: false,
            in_block_comment: false,
            block_start_line: None,
        }
    }

    /// 扫描一行内容，更新内部状态
    ///
    /// 若当前行首次出现 `{`（depth 从 0 变为正数），记录 block_start_line。
    /// 返回 true 表示 depth 归零（block 结束被触发）。
    fn scan_line(&mut self, content: &str, line_idx: usize) -> bool {
        let chars: Vec<char> = content.chars().collect();
        let mut i = 0;
        let mut in_line_comment = false;
        let mut depth_reached_zero = false;

        while i < chars.len() {
            let c = chars[i];

            if in_line_comment {
                i += 1;
                continue;
            }
            if self.in_block_comment {
                if self.try_consume_comment_end(&chars, &mut i) {
                    continue;
                }
                i += 1;
                continue;
            }
            if self.in_string {
                self.consume_string_char(&chars, &mut i);
                continue;
            }

            if self.try_consume_comment_start(&chars, &mut i, &mut in_line_comment) {
                continue;
            }
            if c == '"' {
                self.in_string = true;
                i += 1;
                continue;
            }

            if c == '{' {
                if self.depth == 0 {
                    self.depth = 1;
                    self.block_start_line = Some(line_idx);
                } else {
                    self.depth += 1;
                }
            } else if c == '}' {
                self.depth -= 1;
                if self.depth == 0 {
                    depth_reached_zero = true;
                }
            }

            i += 1;
        }

        depth_reached_zero
    }

    /// 尝试消费字符串内的字符（处理 `\"` 和 `\\` 转义）
    fn consume_string_char(&mut self, chars: &[char], i: &mut usize) {
        if chars[*i] == '\\' && *i + 1 < chars.len() {
            *i += 2;
            return;
        }
        if chars[*i] == '"' {
            self.in_string = false;
        }
        *i += 1;
    }

    /// 尝试消费行注释 `//` 或块注释 `/*` 的起始
    ///
    /// 返回 true 表示消费了注释起始标记。
    fn try_consume_comment_start(
        &mut self,
        chars: &[char],
        i: &mut usize,
        in_line_comment: &mut bool,
    ) -> bool {
        if chars[*i] == '/' && *i + 1 < chars.len() {
            match chars[*i + 1] {
                '/' => {
                    *in_line_comment = true;
                    *i += 2;
                    return true;
                }
                '*' => {
                    self.in_block_comment = true;
                    *i += 2;
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    /// 尝试消费块注释的结束 `*/`
    ///
    /// 返回 true 表示消费了注释结束标记。
    fn try_consume_comment_end(&mut self, chars: &[char], i: &mut usize) -> bool {
        if chars[*i] == '*' && *i + 1 < chars.len() && chars[*i + 1] == '/' {
            self.in_block_comment = false;
            *i += 2;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{self, FileContent, Line, LineNumber, SearchScope};
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
    // detect_language 测试
    // ============================================================

    #[test]
    fn test_detect_language_brace() {
        let file = make_file(&["fn example() {", "    let x = 1;", "}"]);
        let scope = SearchScope::File(&file);
        assert_eq!(BlockParser::detect_language(&scope, 0), Language::Brace);
    }

    #[test]
    fn test_detect_language_brace_with_struct() {
        let file = make_file(&["struct Foo {", "    x: i32,", "    y: i32,", "}"]);
        let scope = SearchScope::File(&file);
        assert_eq!(BlockParser::detect_language(&scope, 0), Language::Brace);
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
        let scope = SearchScope::File(&file);
        assert_eq!(BlockParser::detect_language(&scope, 0), Language::Indent);
    }

    #[test]
    fn test_detect_language_indent_yaml() {
        let file = make_file(&["key:", "  sub1: value1", "  sub2: value2"]);
        let scope = SearchScope::File(&file);
        assert_eq!(BlockParser::detect_language(&scope, 0), Language::Indent);
    }

    #[test]
    fn test_detect_language_unknown_plain_text() {
        let file = make_file(&["# Title", "## Section", "Some text."]);
        let scope = SearchScope::File(&file);
        assert_eq!(BlockParser::detect_language(&scope, 0), Language::Unknown);
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
        let scope = SearchScope::File(&file);
        let result = BlockParser::parse_brace_block(&scope, 0);
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
        let scope = SearchScope::File(&file);
        let result = BlockParser::parse_brace_block(&scope, 0);
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
        let scope = SearchScope::File(&file);
        let result = BlockParser::parse_brace_block(&scope, 0);
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
        let scope = SearchScope::File(&file);
        let result = BlockParser::parse_brace_block(&scope, 0);
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
        let scope = SearchScope::File(&file);
        let result = BlockParser::parse_brace_block(&scope, 0);
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
        let scope = SearchScope::File(&file);
        let result = BlockParser::parse_brace_block(&scope, 0);
        assert!(result.is_ok());
        let (start, end) = result.unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, 4);
    }

    #[test]
    fn test_parse_brace_block_no_brace_returns_error() {
        let file = make_file(&["# Title", "Some text without braces"]);
        let scope = SearchScope::File(&file);
        let result = BlockParser::parse_brace_block(&scope, 0);
        assert!(result.is_err());
        match result.unwrap_err() {
            MatchError::BlockNotParseable { .. } => {}
            _ => panic!("Expected BlockNotParseable"),
        }
    }

    #[test]
    fn test_parse_brace_block_start_on_second_line() {
        let file = make_file(&["fn example()", "{", "    do_stuff();", "}"]);
        let scope = SearchScope::File(&file);
        let result = BlockParser::parse_brace_block(&scope, 0);
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
        let scope = SearchScope::File(&file);
        let result = BlockParser::parse_brace_block(&scope, 0);
        assert!(result.is_ok());
        let (start, end) = result.unwrap();
        assert_eq!(start, 0);
        assert_eq!(end, 2);
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
        let scope = SearchScope::File(&file);
        let (start, end) = BlockParser::parse_indent_block(&scope, 0);
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
        let scope = SearchScope::File(&file);
        let (start, end) = BlockParser::parse_indent_block(&scope, 0);
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
        let scope = SearchScope::File(&file);
        let result = BlockParser::parse_block(&scope, 0);
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
        let scope = SearchScope::File(&file);
        let result = BlockParser::parse_block(&scope, 0);
        assert!(result.is_err());
        match result.unwrap_err() {
            MatchError::BlockNotParseable { .. } => {}
            _ => panic!("Expected BlockNotParseable for plain text"),
        }
    }

    #[test]
    fn test_parse_brace_block_within_middle_of_file() {
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
        let scope = SearchScope::File(&file);
        let result = BlockParser::parse_brace_block(&scope, 3);
        assert!(result.is_ok());
        let (start, end) = result.unwrap();
        assert_eq!(start, 6);
        assert_eq!(end, 11);
    }
}
