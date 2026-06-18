//! 核心数据结构 (Data Model)
//!
//! 定义项目中所有共享的数据类型，包括文件内容表示、
//! 代码块表示、Location 匹配相关结构等。
//!
//! ## 迁移来源
//!
//! 从 n_edit/src/model.rs 迁移，保留约 90% 原始逻辑。
//! 详见 INSTRUCTION.md §2.5–§2.6
//!
//! ## 实现逻辑
//!
//! 1. `LineNumber` newtype 封装 1-based 行号，避免与 0-based 数组索引混淆
//! 2. `Line` 预计算 `stripped_content`（去空白后内容）和 `diff_taps`（缩进差异）加速匹配
//! 3. `ContentBlock` 维护 `first_line_index` HashMap 实现 O(1) 首行匹配
//! 4. `SearchScope` 统一 FileContent 和 ContentBlock 的搜索范围接口
//! 5. `LineRange` 支持行号范围定位的解析

use std::collections::HashMap;

// ============================================================
// Newtype 包装
// ============================================================

/// 文件中的行号（从 1 开始计数）
///
/// 与普通 `usize` 区分，避免与数组索引（从 0 开始）混淆。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LineNumber(usize);

impl LineNumber {
    /// 从 1-based 行号创建
    pub const fn new(line_number: usize) -> Self {
        LineNumber(line_number)
    }

    /// 从 0-based 数组索引转换为 1-based 行号
    pub fn from_index(index: usize) -> Self {
        LineNumber(index + 1)
    }

    /// 转换为裸 usize 值（1-based 行号）
    pub fn to_usize(self) -> usize {
        self.0
    }

    /// 转换为 0-based 数组索引
    pub fn to_index(self) -> usize {
        self.0.saturating_sub(1)
    }

    /// 饱和减法
    pub fn saturating_sub(self, rhs: usize) -> Self {
        LineNumber(self.0.saturating_sub(rhs).max(1))
    }
}

impl std::fmt::Display for LineNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::ops::Add<usize> for LineNumber {
    type Output = LineNumber;
    fn add(self, rhs: usize) -> LineNumber {
        LineNumber(self.0 + rhs)
    }
}

impl std::ops::Sub<usize> for LineNumber {
    type Output = LineNumber;
    fn sub(self, rhs: usize) -> LineNumber {
        LineNumber(self.0.saturating_sub(rhs).max(1))
    }
}

impl PartialEq<usize> for LineNumber {
    fn eq(&self, other: &usize) -> bool {
        self.0 == *other
    }
}

impl PartialOrd<usize> for LineNumber {
    fn partial_cmp(&self, other: &usize) -> Option<std::cmp::Ordering> {
        self.0.partial_cmp(other)
    }
}

// ============================================================
// 常量
// ============================================================

/// 错误信息中展示的代码块摘要最大行数
pub const BLOCK_SNIPPET_MAX_LINES: usize = 10;
/// 匹配错误中展示的候选数量上限
pub const MAX_CANDIDATE_DISPLAY: usize = 3;
/// 候选行内容截断长度
pub const CANDIDATE_SNIPPET_MAX_LEN: usize = 60;
/// 花括号语言检测时向前探查的行数
pub const LANGUAGE_DETECT_WINDOW: usize = 5;
/// 缩进语言检测时向前探查的行数
pub const INDENT_DETECT_WINDOW: usize = 20;

// ============================================================
// 工具函数
// ============================================================

/// 去除字符串中的所有空白字符，返回新字符串
///
/// 用于纯字符匹配：将源码行中的空格、tab 等全部移除后比对。
pub fn stripped_content(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 计算行首的 ASCII 空格数量（tab 不计入）
pub fn count_leading_spaces(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ').count()
}

// ============================================================
// 核心数据结构
// ============================================================

/// 逐行解析后的行数据
///
/// 每一行保留原始内容的同时，预计算缩进信息以加速匹配。
#[derive(Debug, PartialEq, Clone)]
pub struct Line {
    /// 在文件中的行号（从 1 开始计数）
    pub line_num: LineNumber,
    /// 行首空格数量（只计 ASCII 0x20，tab 按配置折算）
    pub taps: usize,
    /// 相对于所在 ContentBlock 首行的缩进差异
    pub diff_taps: usize,
    /// 该行的原始文本内容
    pub content: String,
    /// 预计算的去空白版本，用于快速匹配（避免每次匹配时重复分配）
    pub stripped_content: String,
}

impl Line {
    /// 返回去除所有空白字符后的内容，用于纯字符匹配
    pub fn stripped_content(&self) -> &str {
        &self.stripped_content
    }
}

/// Location 命令中用户提供的定位内容的一行
#[derive(Debug, PartialEq)]
pub struct LocationLine {
    /// 从 0 开始的序号（第一行为 0）
    pub index: usize,
    /// 缩进差异量（以 index=0 行为基准）
    pub diff_taps: Option<usize>,
    /// 原始内容（保留缩进和空格）
    pub content: String,
    /// 对应原文行号，未解析时为 None
    pub line_num: Option<LineNumber>,
}

/// Location 命令后提取的定位内容
#[derive(Debug, PartialEq)]
pub struct LocationContent {
    /// 定位内容的所有行
    pub lines: Vec<LocationLine>,
}

impl LocationContent {
    /// 提取定位内容的第一行（去除空白后用于首行匹配）
    pub fn stripped_first_line(&self) -> String {
        self.lines[0]
            .content
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect()
    }
}

/// 从文件内容中匹配到的一行
#[derive(Debug, PartialEq)]
pub struct MatchLine {
    /// 原文行号
    pub line_num: LineNumber,
    /// 该行在原文中的缩进量（空格数）
    pub taps: usize,
    /// 缩进差异（以本组第一行为基准）
    pub diff_taps: usize,
    /// 原始内容
    pub content: String,
}

/// 第一行纯字符匹配后得到的候选结果
#[derive(Debug, PartialEq)]
pub struct FirstMatchContent {
    /// 匹配到的首行在原文中的行号
    pub start_line: LineNumber,
    /// 从 start_line 起向后取与 LocationContent 等行数的内容
    pub lines: Vec<MatchLine>,
}

/// 定位信息的来源
///
/// 用于确定 New:Normal 的插入位置。
#[derive(Debug, PartialEq)]
pub enum MatchInfo {
    /// 空 Location（无匹配内容），New 插入到 Block 末尾
    Empty,
    /// Location 匹配到的行数，New 插入到匹配行之后
    Location { matched_line_count: usize },
    /// Delete 操作后记录的删除起始位置，New 插入到此位置替换
    DeleteAt { position: usize },
}

/// 一个代码块（可能为整个文件、一个方法、一个循环体等）
#[derive(Debug, PartialEq)]
pub struct ContentBlock {
    /// Block 在文件中的起始行号（1-based）
    pub start_line: LineNumber,
    /// Block 在文件中的结束行号（1-based），用于精确替换
    pub end_line: LineNumber,
    /// Block 内包含的所有行
    pub lines: Vec<Line>,
    /// 首行哈希索引：stripped_content → 行索引列表，用于嵌套 Location 的 O(1) 首行匹配
    pub first_line_index: HashMap<String, Vec<usize>>,
    /// 定位信息来源，用于确定 New:Normal 的插入位置
    pub match_info: MatchInfo,
}

/// Open 命令解析文件后得到的完整文件内容
#[derive(Debug, PartialEq)]
pub struct FileContent {
    /// 文件的所有行
    pub lines: Vec<Line>,
    /// 首行哈希索引：stripped_content → 行索引列表，用于 O(1) 首行匹配
    pub first_line_index: HashMap<String, Vec<usize>>,
}

/// New 命令中用户提供的新增内容的一行
#[derive(Debug, PartialEq)]
pub struct NewLine {
    /// 相对于插入位置的缩进差异
    pub diff_taps: usize,
    /// 去除首部缩进后的内容（保留内部空格）
    pub content: String,
    /// 是否为 Raw 命令指定的字面量（此时 diff_taps 被忽略）
    pub is_raw: bool,
}

/// New 命令后提取的新增内容
#[derive(Debug, PartialEq)]
pub struct NewContent {
    /// 新增内容的所有行
    pub lines: Vec<NewLine>,
}

/// Delete 命令中用户提供的删除内容的一行
#[derive(Debug, PartialEq)]
pub struct DeleteLine {
    /// 用于匹配的原始文本
    pub content: String,
    /// 是否为 Raw 命令指定的字面量
    pub is_raw: bool,
}

/// Delete 命令后提取的匹配内容（到 `...` 分隔符或下一个命令为止）
#[derive(Debug, PartialEq)]
pub struct DeleteContent {
    /// 删除匹配内容的所有行
    pub lines: Vec<DeleteLine>,
}

// ============================================================
// ContentBlock 方法
// ============================================================

impl ContentBlock {
    /// 在 ContentBlock 内重新计算所有行的 line_num、diff_taps 和 first_line_index
    ///
    /// 以 block 首行为基准，递增分配行号，重算缩进差异，
    /// 并重建首行哈希索引供嵌套 Location 使用。
    ///
    /// 注意：end_line 不被修改，因为它代表文件中的原始替换范围，
    /// 由 apply_block_to_file 使用。
    pub fn reindex(&mut self) {
        if self.lines.is_empty() {
            self.first_line_index.clear();
            return;
        }
        let base_taps = self.lines[0].taps;
        let base_line_num = self.start_line;
        for (index, line) in self.lines.iter_mut().enumerate() {
            line.line_num = base_line_num + index;
            line.diff_taps = line.taps.saturating_sub(base_taps);
        }

        let mut index: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, line) in self.lines.iter().enumerate() {
            index
                .entry(line.stripped_content.clone())
                .or_default()
                .push(i);
        }
        self.first_line_index = index;
    }
}

// ============================================================
// FileContent 方法
// ============================================================

impl FileContent {
    /// 从文件路径读取并构建 FileContent
    ///
    /// 逐行解析文件内容，计算每行的 taps（行首空格数），
    /// 预计算 stripped_content，构建首行哈希索引。
    pub fn from_path(path: &str) -> Result<Self, crate::error::FileError> {
        let content =
            std::fs::read_to_string(path).map_err(|e| crate::error::FileError::CannotOpen {
                path: path.to_string(),
                reason: e.to_string(),
            })?;

        let mut lines: Vec<Line> = Vec::new();
        let mut first_line_index: HashMap<String, Vec<usize>> = HashMap::new();

        for (index, line_content) in content.lines().enumerate() {
            let taps = count_leading_spaces(line_content);
            let stripped = stripped_content(line_content);

            first_line_index
                .entry(stripped.clone())
                .or_default()
                .push(index);

            lines.push(Line {
                line_num: LineNumber::from_index(index),
                taps,
                diff_taps: 0,
                content: line_content.to_string(),
                stripped_content: stripped,
            });
        }

        Ok(FileContent {
            lines,
            first_line_index,
        })
    }

    /// 将 FileContent 按行写回文件
    ///
    /// 每行末尾追加换行符。
    pub fn write_back(&self, path: &str) -> Result<(), crate::error::FileError> {
        let content: String = self
            .lines
            .iter()
            .map(|line| line.content.as_str())
            .collect::<Vec<&str>>()
            .join("\n");
        let content = content + "\n";

        std::fs::write(path, content).map_err(|e| crate::error::FileError::WriteFailed {
            path: path.to_string(),
            reason: e.to_string(),
        })
    }
}

// ============================================================
// SearchScope
// ============================================================

/// Location 的搜索范围
///
/// 顶层 Location → 搜索范围 = FileContent
/// 嵌套 Location → 搜索范围 = 当前 ContentBlock（block_stack 栈顶）
///
/// 提供统一的 lines() 和 first_line_index() 访问接口，
/// 使 LocationMatcher 无需区分搜索范围的具体来源。
pub enum SearchScope<'a> {
    /// 搜索范围为完整文件内容
    File(&'a FileContent),
    /// 搜索范围为当前 ContentBlock（嵌套 Location）
    Block(&'a ContentBlock),
}

impl<'a> SearchScope<'a> {
    /// 返回搜索范围内的所有行
    pub fn lines(&self) -> &[Line] {
        match self {
            SearchScope::File(f) => &f.lines,
            SearchScope::Block(b) => &b.lines,
        }
    }

    /// 返回首行哈希索引
    pub fn first_line_index(&self) -> &HashMap<String, Vec<usize>> {
        match self {
            SearchScope::File(f) => &f.first_line_index,
            SearchScope::Block(b) => &b.first_line_index,
        }
    }

    /// 返回搜索范围内的行数
    pub fn len(&self) -> usize {
        self.lines().len()
    }

    /// 搜索范围是否为空
    pub fn is_empty(&self) -> bool {
        self.lines().is_empty()
    }
}

// ============================================================
// LineRange
// ============================================================

/// 行号范围，用于行号定位
///
/// start 和 end 均为 1-based 行号，end >= start。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRange {
    /// 起始行号（1-based）
    pub start: usize,
    /// 结束行号（1-based），end >= start
    pub end: usize,
}

impl LineRange {
    /// 从字符串解析行号范围，例如 "@66,120" 或 "@66"
    ///
    /// 支持格式：
    /// - `@66` → LineRange { start: 66, end: 66 }
    /// - `@66,120` → LineRange { start: 66, end: 120 }
    ///
    /// 验证 start > 0, end >= start。
    pub fn parse(input: &str) -> Result<Self, String> {
        let nums = input.trim_start_matches('@').trim();
        let parts: Vec<&str> = nums.split(',').collect();

        match parts.len() {
            1 => {
                let start: usize = parts[0]
                    .trim()
                    .parse()
                    .map_err(|_| format!("无效的行号: {}", parts[0]))?;
                if start == 0 {
                    return Err(format!("行号必须大于 0，实际值: {}", start));
                }
                Ok(LineRange { start, end: start })
            }
            2 => {
                let start: usize = parts[0]
                    .trim()
                    .parse()
                    .map_err(|_| format!("无效的起始行号: {}", parts[0]))?;
                let end: usize = parts[1]
                    .trim()
                    .parse()
                    .map_err(|_| format!("无效的结束行号: {}", parts[1]))?;
                if start == 0 {
                    return Err(format!("起始行号必须大于 0，实际值: {}", start));
                }
                if end < start {
                    return Err(format!("结束行号 {} 不能小于起始行号 {}", end, start));
                }
                Ok(LineRange { start, end })
            }
            _ => Err(format!("无效的行号格式: {}", input)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================
    // LineNumber 测试
    // ============================================================

    #[test]
    fn test_line_number_new() {
        let ln = LineNumber::new(1);
        assert_eq!(ln.to_usize(), 1);
        assert_eq!(ln.to_index(), 0);
    }

    #[test]
    fn test_line_number_from_index() {
        let ln = LineNumber::from_index(0);
        assert_eq!(ln.to_usize(), 1);
        let ln = LineNumber::from_index(5);
        assert_eq!(ln.to_usize(), 6);
    }

    #[test]
    fn test_line_number_add() {
        let ln = LineNumber::new(3);
        assert_eq!((ln + 2).to_usize(), 5);
    }

    #[test]
    fn test_line_number_sub() {
        let ln = LineNumber::new(5);
        assert_eq!((ln - 2).to_usize(), 3);
    }

    #[test]
    fn test_line_number_sub_saturates() {
        let ln = LineNumber::new(1);
        assert_eq!((ln - 5).to_usize(), 1);
    }

    #[test]
    fn test_line_number_eq_usize() {
        let ln = LineNumber::new(3);
        assert_eq!(ln, 3);
        assert_ne!(ln, 4);
    }

    #[test]
    fn test_line_number_display() {
        let ln = LineNumber::new(42);
        assert_eq!(format!("{}", ln), "42");
    }

    // ============================================================
    // Line 测试
    // ============================================================

    #[test]
    fn test_line_stripped_content_removes_spaces() {
        let content = "    let x = 1;".to_string();
        let stripped = stripped_content(&content);
        let line = Line {
            line_num: LineNumber::new(1),
            taps: 4,
            diff_taps: 0,
            content,
            stripped_content: stripped,
        };
        assert_eq!(line.stripped_content(), "letx=1;");
    }

    #[test]
    fn test_line_stripped_content_empty_string() {
        let line = Line {
            line_num: LineNumber::new(1),
            taps: 0,
            diff_taps: 0,
            content: String::new(),
            stripped_content: String::new(),
        };
        assert_eq!(line.stripped_content(), "");
    }

    // ============================================================
    // ContentBlock::reindex 测试
    // ============================================================

    #[test]
    fn test_content_block_reindex_updates_line_numbers() {
        let mut block = ContentBlock {
            start_line: LineNumber::new(5),
            end_line: LineNumber::new(6),
            first_line_index: HashMap::new(),
            match_info: MatchInfo::Location {
                matched_line_count: 1,
            },
            lines: vec![
                Line {
                    line_num: LineNumber::new(5),
                    taps: 4,
                    diff_taps: 0,
                    content: "    a();".to_string(),
                    stripped_content: stripped_content("    a();"),
                },
                Line {
                    line_num: LineNumber::new(5),
                    taps: 8,
                    diff_taps: 0,
                    content: "        b();".to_string(),
                    stripped_content: stripped_content("        b();"),
                },
            ],
        };
        block.reindex();
        assert_eq!(block.lines[0].line_num, 5);
        assert_eq!(block.lines[1].line_num, 6);
        assert_eq!(block.lines[0].diff_taps, 0);
        assert_eq!(block.lines[1].diff_taps, 4);
    }

    #[test]
    fn test_content_block_reindex_empty_block() {
        let mut block = ContentBlock {
            start_line: LineNumber::new(1),
            end_line: LineNumber::new(1),
            first_line_index: HashMap::new(),
            match_info: MatchInfo::Empty,
            lines: vec![],
        };
        block.reindex();
        assert!(block.lines.is_empty());
    }

    // ============================================================
    // FileContent 测试
    // ============================================================

    #[test]
    fn test_file_content_from_path_line_numbers() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "line one\nline two\nline three\n").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let file = FileContent::from_path(&path).unwrap();
        assert_eq!(file.lines.len(), 3);
        assert_eq!(file.lines[0].line_num, 1);
        assert_eq!(file.lines[0].content, "line one");
        assert_eq!(file.lines[1].line_num, 2);
        assert_eq!(file.lines[1].content, "line two");
    }

    #[test]
    fn test_file_content_from_path_calculates_taps() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "no indent\n    four spaces\n").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let file = FileContent::from_path(&path).unwrap();
        assert_eq!(file.lines[0].taps, 0);
        assert_eq!(file.lines[1].taps, 4);
    }

    #[test]
    fn test_file_content_from_path_nonexistent_file_returns_error() {
        let result = FileContent::from_path("/nonexistent/path/file.rs");
        assert!(result.is_err());
    }

    #[test]
    fn test_file_content_write_back_round_trip() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        let original = "line one\nline two\nline three\n";
        write!(tmp, "{}", original).unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let file = FileContent::from_path(&path).unwrap();
        file.write_back(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, original);
    }

    // ============================================================
    // LineRange 测试
    // ============================================================

    #[test]
    fn test_line_range_parse_single_line() {
        let range = LineRange::parse("@66").unwrap();
        assert_eq!(range.start, 66);
        assert_eq!(range.end, 66);
    }

    #[test]
    fn test_line_range_parse_range() {
        let range = LineRange::parse("@66,120").unwrap();
        assert_eq!(range.start, 66);
        assert_eq!(range.end, 120);
    }

    #[test]
    fn test_line_range_parse_start_zero() {
        let result = LineRange::parse("@0,10");
        assert!(result.is_err());
    }

    #[test]
    fn test_line_range_parse_end_less_than_start() {
        let result = LineRange::parse("@10,5");
        assert!(result.is_err());
    }

    #[test]
    fn test_line_range_parse_invalid_format() {
        let result = LineRange::parse("@10,20,30");
        assert!(result.is_err());
    }

    // ============================================================
    // SearchScope 测试
    // ============================================================

    #[test]
    fn test_search_scope_file_basics() {
        let file = FileContent {
            lines: vec![],
            first_line_index: HashMap::new(),
        };
        let scope = SearchScope::File(&file);
        assert!(scope.is_empty());
        assert_eq!(scope.len(), 0);
    }

    // ============================================================
    // NewLine / DeleteLine 测试
    // ============================================================

    #[test]
    fn test_new_line_creation() {
        let new_line = NewLine {
            diff_taps: 4,
            content: "let x = 1;".to_string(),
            is_raw: false,
        };
        assert_eq!(new_line.diff_taps, 4);
        assert_eq!(new_line.content, "let x = 1;");
        assert!(!new_line.is_raw);
    }

    #[test]
    fn test_new_line_is_raw() {
        let new_line = NewLine {
            diff_taps: 0,
            content: "...".to_string(),
            is_raw: true,
        };
        assert!(new_line.is_raw);
    }

    #[test]
    fn test_delete_line_creation() {
        let delete_line = DeleteLine {
            content: "let x = 1;".to_string(),
            is_raw: false,
        };
        assert_eq!(delete_line.content, "let x = 1;");
        assert!(!delete_line.is_raw);
    }

    #[test]
    fn test_location_content_stripped_first_line() {
        let loc = LocationContent {
            lines: vec![LocationLine {
                index: 0,
                diff_taps: Some(0),
                content: "    fn main() {".to_string(),
                line_num: None,
            }],
        };
        assert_eq!(stripped_content(&loc.lines[0].content), "fnmain(){");
    }
}
