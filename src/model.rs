//! 核心数据结构 (Data Model)
//!
//! 定义项目中所有共享的数据类型，包括文件内容表示、
//! 代码块表示、Location 匹配相关结构等。
//!
//! ## 对应文档
//!
//! 详见 INSTRUCTION.md 第 2 节 "详细数据结构"

use std::collections::HashMap;

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

/// 逐行解析后的行数据
///
/// 每一行保留原始内容的同时，预计算缩进信息以加速匹配。
#[derive(Debug, PartialEq)]
pub struct Line {
    /// 在文件中的行号（从 1 开始计数）
    pub line_num: usize,
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
    pub line_num: Option<usize>,
}

/// Location 命令后提取的定位内容
#[derive(Debug, PartialEq)]
pub struct LocationContent {
    /// 定位内容的所有行
    pub lines: Vec<LocationLine>,
}

impl LocationContent {
    /// 提取定位内容的第一行（去除空白后用于首行匹配）
    #[allow(dead_code)]
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
#[allow(dead_code)]
pub struct MatchLine {
    /// 原文行号
    pub line_num: usize,
    /// 该行在原文中的缩进量（空格数）
    pub taps: usize,
    /// 缩进差异（以本组第一行为基准）
    pub diff_taps: usize,
    /// 原始内容
    pub content: String,
}

/// 第一行纯字符匹配后得到的候选结果
#[derive(Debug, PartialEq)]
#[allow(dead_code)]
pub struct FirstMatchContent {
    /// 匹配到的首行在原文中的行号
    pub start_line: usize,
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
    pub start_line: usize,
    /// Block 在文件中的结束行号（1-based），用于精确替换
    pub end_line: usize,
    /// Block 内包含的所有行
    pub lines: Vec<Line>,
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

impl ContentBlock {
    /// 在 ContentBlock 内重新计算所有行的 line_num 和 diff_taps
    ///
    /// 以 block 首行为基准，递增分配行号，重算缩进差异。
    pub fn reindex(&mut self) {
        if self.lines.is_empty() {
            return;
        }
        let base_taps = self.lines[0].taps;
        let base_line_num = self.start_line;
        for (index, line) in self.lines.iter_mut().enumerate() {
            line.line_num = base_line_num + index;
            line.diff_taps = line.taps.saturating_sub(base_taps);
        }
    }
}

impl FileContent {
    /// 从文件路径读取并构建 FileContent
    ///
    /// 逐行解析文件内容，计算每行的 taps（行首空格数），
    /// 预计算 stripped_content，构建首行哈希索引。
    /// diff_taps 暂设为 0（Phase 3 再精确计算）。
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
                line_num: index + 1,
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

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================
    // Line 测试
    // ============================================================

    #[test]
    fn test_line_stripped_content_removes_spaces() {
        let content = "    let x = 1;".to_string();
        let stripped = stripped_content(&content);
        let line = Line {
            line_num: 1,
            taps: 4,
            diff_taps: 0,
            content,
            stripped_content: stripped,
        };
        assert_eq!(line.stripped_content(), "letx=1;");
    }

    #[test]
    fn test_line_stripped_content_removes_tabs() {
        let content = "\t\tfn foo()".to_string();
        let stripped = stripped_content(&content);
        let line = Line {
            line_num: 1,
            taps: 2,
            diff_taps: 0,
            content,
            stripped_content: stripped,
        };
        assert_eq!(line.stripped_content(), "fnfoo()");
    }

    #[test]
    fn test_line_stripped_content_removes_all_whitespace() {
        let content = "  a b   c  ".to_string();
        let stripped = stripped_content(&content);
        let line = Line {
            line_num: 1,
            taps: 0,
            diff_taps: 0,
            content,
            stripped_content: stripped,
        };
        assert_eq!(line.stripped_content(), "abc");
    }

    #[test]
    fn test_line_stripped_content_empty_string() {
        let line = Line {
            line_num: 1,
            taps: 0,
            diff_taps: 0,
            content: String::new(),
            stripped_content: String::new(),
        };
        assert_eq!(line.stripped_content(), "");
    }

    // ============================================================
    // LocationLine 测试
    // ============================================================

    #[test]
    fn test_location_line_creation() {
        let loc_line = LocationLine {
            index: 0,
            diff_taps: Some(0),
            content: "fn main() {".to_string(),
            line_num: None,
        };
        assert_eq!(loc_line.index, 0);
        assert_eq!(loc_line.diff_taps, Some(0));
        assert_eq!(loc_line.content, "fn main() {");
        assert_eq!(loc_line.line_num, None);
    }

    // ============================================================
    // LocationContent 测试
    // ============================================================

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

    #[test]
    fn test_location_content_line_count() {
        let loc = LocationContent {
            lines: vec![
                LocationLine {
                    index: 0,
                    diff_taps: Some(0),
                    content: "fn main() {".to_string(),
                    line_num: None,
                },
                LocationLine {
                    index: 1,
                    diff_taps: Some(4),
                    content: "    let x = 1;".to_string(),
                    line_num: None,
                },
            ],
        };
        assert_eq!(loc.lines.len(), 2);
    }

    // ============================================================
    // MatchLine 测试
    // ============================================================

    #[test]
    fn test_match_line_creation() {
        let match_line = MatchLine {
            line_num: 5,
            taps: 4,
            diff_taps: 0,
            content: "    let x = 1;".to_string(),
        };
        assert_eq!(match_line.line_num, 5);
        assert_eq!(match_line.taps, 4);
        assert_eq!(match_line.diff_taps, 0);
        assert_eq!(match_line.content, "    let x = 1;");
    }

    // ============================================================
    // FirstMatchContent 测试
    // ============================================================

    #[test]
    fn test_first_match_content_creation() {
        let fmc = FirstMatchContent {
            start_line: 10,
            lines: vec![
                MatchLine {
                    line_num: 10,
                    taps: 0,
                    diff_taps: 0,
                    content: "fn main() {".to_string(),
                },
                MatchLine {
                    line_num: 11,
                    taps: 4,
                    diff_taps: 4,
                    content: "    let x = 1;".to_string(),
                },
            ],
        };
        assert_eq!(fmc.start_line, 10);
        assert_eq!(fmc.lines.len(), 2);
    }

    // ============================================================
    // ContentBlock 测试
    // ============================================================

    #[test]
    fn test_content_block_creation() {
        let block = ContentBlock {
            start_line: 5,
            end_line: 0,
            match_info: MatchInfo::Location {
                matched_line_count: 2,
            },
            lines: vec![
                Line {
                    line_num: 5,
                    taps: 0,
                    diff_taps: 0,
                    content: "fn foo() {".to_string(),
                    stripped_content: stripped_content("fn foo() {"),
                },
                Line {
                    line_num: 6,
                    taps: 4,
                    diff_taps: 4,
                    content: "    bar();".to_string(),
                    stripped_content: stripped_content("    bar();"),
                },
            ],
        };
        assert_eq!(block.start_line, 5);
        assert_eq!(block.lines.len(), 2);
        assert_eq!(block.lines[0].line_num, 5);
    }

    // ============================================================
    // FileContent 测试
    // ============================================================

    #[test]
    fn test_file_content_creation() {
        let mut index = HashMap::new();
        index.insert(stripped_content("// comment"), vec![0]);
        index.insert(stripped_content("fn main() {}"), vec![1]);
        let file = FileContent {
            lines: vec![
                Line {
                    line_num: 1,
                    taps: 0,
                    diff_taps: 0,
                    content: "// comment".to_string(),
                    stripped_content: stripped_content("// comment"),
                },
                Line {
                    line_num: 2,
                    taps: 0,
                    diff_taps: 0,
                    content: "fn main() {}".to_string(),
                    stripped_content: stripped_content("fn main() {}"),
                },
            ],
            first_line_index: index,
        };
        assert_eq!(file.lines.len(), 2);
        assert_eq!(file.lines[0].line_num, 1);
        assert_eq!(file.lines[1].line_num, 2);
    }

    // ============================================================
    // FileContent::from_path 测试
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
        assert_eq!(file.lines[2].line_num, 3);
        assert_eq!(file.lines[2].content, "line three");
    }

    #[test]
    fn test_file_content_from_path_calculates_taps() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "no indent\n    four spaces\n\t\ttwo tabs\n").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let file = FileContent::from_path(&path).unwrap();
        assert_eq!(file.lines[0].taps, 0);
        assert_eq!(file.lines[1].taps, 4);
        assert_eq!(file.lines[2].taps, 0); // tabs not counted as spaces
    }

    #[test]
    fn test_file_content_from_path_diff_taps_initially_zero() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "fn foo() {{\n    bar();\n}}\n").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let file = FileContent::from_path(&path).unwrap();
        // diff_taps 暂设为 0（Phase 1 不计算 diff_taps）
        assert_eq!(file.lines[0].diff_taps, 0);
        assert_eq!(file.lines[1].diff_taps, 0);
        assert_eq!(file.lines[2].diff_taps, 0);
    }

    #[test]
    fn test_file_content_from_path_nonexistent_file_returns_error() {
        let result = FileContent::from_path("/nonexistent/path/file.rs");
        assert!(result.is_err());
    }

    // ============================================================
    // FileContent::write_back 测试
    // ============================================================

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

    #[test]
    fn test_file_content_write_back_with_modified_content() {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "original\n").unwrap();
        let path = tmp.path().to_str().unwrap().to_string();

        let mut file = FileContent::from_path(&path).unwrap();
        file.lines[0].content = "modified".to_string();
        file.write_back(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "modified\n");
    }

    // ============================================================
    // NewLine / NewContent 测试
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
    fn test_new_content_creation() {
        let content = NewContent {
            lines: vec![
                NewLine {
                    diff_taps: 0,
                    content: "let a = 1;".to_string(),
                    is_raw: false,
                },
                NewLine {
                    diff_taps: 0,
                    content: "let b = 2;".to_string(),
                    is_raw: false,
                },
            ],
        };
        assert_eq!(content.lines.len(), 2);
    }

    // ============================================================
    // DeleteLine / DeleteContent 测试
    // ============================================================

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
    fn test_delete_content_creation() {
        let content = DeleteContent {
            lines: vec![
                DeleteLine {
                    content: "let a = 1;".to_string(),
                    is_raw: false,
                },
                DeleteLine {
                    content: "let b = 2;".to_string(),
                    is_raw: false,
                },
            ],
        };
        assert_eq!(content.lines.len(), 2);
    }

    // ============================================================
    // ContentBlock::reindex 测试
    // ============================================================

    #[test]
    fn test_content_block_reindex_updates_line_numbers() {
        let mut block = ContentBlock {
            start_line: 5,
            end_line: 0,
            match_info: MatchInfo::Location {
                matched_line_count: 1,
            },
            lines: vec![
                Line {
                    line_num: 5,
                    taps: 4,
                    diff_taps: 0,
                    content: "    a();".to_string(),
                    stripped_content: stripped_content("    a();"),
                },
                Line {
                    line_num: 5,
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
            start_line: 1,
            end_line: 0,
            match_info: MatchInfo::Empty,
            lines: vec![],
        };
        block.reindex();
        assert!(block.lines.is_empty());
    }
}
