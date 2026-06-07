//! 终端输出格式化 (Output)
//!
//! 提供彩色终端输出和格式化功能。
//!
//! ## 实现逻辑
//!
//! 1. 检测终端能力（`is_terminal`），管道/重定向时自动关闭颜色
//! 2. 新增行前加绿色 `+`，删除行前加红色 `-`
//! 3. ContentBlock 输出带行号前缀
//!
//! ## 对应文档
//!
//! 详见 INSTRUCTION.md 第 5.2 节 "输出格式"

use colored::Colorize;
use std::io::IsTerminal;

/// 输出行的差异状态
#[derive(Debug, PartialEq)]
pub enum DiffLineKind {
    /// 新增的行
    Added,
    /// 删除的行
    Deleted,
    /// 未变更的行
    #[allow(dead_code)]
    Unchanged,
}

/// 一条带差异标记的输出行
#[derive(Debug, PartialEq)]
pub struct DiffLine {
    /// 差异状态
    pub kind: DiffLineKind,
    /// 行号（可选的）
    pub line_number: Option<usize>,
    /// 内容文本
    pub content: String,
}

/// 终端输出格式化器
///
/// 负责将差异行列表格式化为彩色终端输出。
pub struct OutputFormatter {
    /// 是否启用彩色输出
    use_color: bool,
}

impl OutputFormatter {
    /// 创建新的输出格式化器实例
    ///
    /// 自动检测当前输出是否为终端，决定是否启用彩色。
    pub fn new() -> Self {
        let use_color = std::io::stdout().is_terminal();
        OutputFormatter { use_color }
    }
}

impl Default for OutputFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputFormatter {
    /// 创建强制启用/禁用颜色的格式化器
    #[allow(dead_code)]
    pub fn with_color(use_color: bool) -> Self {
        OutputFormatter { use_color }
    }

    /// 格式化差异行列表为字符串输出
    ///
    /// 每行格式为: `[前缀] [行号]: [内容]`
    /// 新增行绿色 `+`，删除行红色 `-`，未变更行无前缀。
    pub fn format_diff_lines(&self, lines: &[DiffLine]) -> String {
        let mut output = String::new();

        for line in lines {
            match line.kind {
                DiffLineKind::Added => {
                    let prefix = if self.use_color {
                        "+".green().to_string()
                    } else {
                        "+".to_string()
                    };
                    if let Some(line_num) = line.line_number {
                        output.push_str(&format!("{} L{}: {}\n", prefix, line_num, line.content));
                    } else {
                        output.push_str(&format!("{} {}\n", prefix, line.content));
                    }
                }
                DiffLineKind::Deleted => {
                    let prefix = if self.use_color {
                        "-".red().to_string()
                    } else {
                        "-".to_string()
                    };
                    if let Some(line_num) = line.line_number {
                        output.push_str(&format!("{} L{}: {}\n", prefix, line_num, line.content));
                    } else {
                        output.push_str(&format!("{} {}\n", prefix, line.content));
                    }
                }
                DiffLineKind::Unchanged => {
                    if let Some(line_num) = line.line_number {
                        output.push_str(&format!("  L{}: {}\n", line_num, line.content));
                    } else {
                        output.push_str(&format!("  {}\n", line.content));
                    }
                }
            }
        }

        output
    }

    /// 格式化 ContentBlock 为带行号的纯文本输出（无颜色、无差异标记）
    #[allow(dead_code)]
    pub fn format_block(&self, block: &crate::model::ContentBlock) -> String {
        let lines: Vec<DiffLine> = block
            .lines
            .iter()
            .map(|line| DiffLine {
                kind: DiffLineKind::Unchanged,
                line_number: Some(line.line_num),
                content: line.content.clone(),
            })
            .collect();
        self.format_diff_lines(&lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_formatter_no_color_creates_correct_prefixes() {
        let formatter = OutputFormatter::with_color(false);
        let lines = vec![
            DiffLine {
                kind: DiffLineKind::Added,
                line_number: Some(3),
                content: "let x = 1;".to_string(),
            },
            DiffLine {
                kind: DiffLineKind::Deleted,
                line_number: Some(4),
                content: "old_code();".to_string(),
            },
            DiffLine {
                kind: DiffLineKind::Unchanged,
                line_number: Some(5),
                content: "fn main() {".to_string(),
            },
        ];
        let output = formatter.format_diff_lines(&lines);
        assert!(output.contains("+ L3: let x = 1;"));
        assert!(output.contains("- L4: old_code();"));
        assert!(output.contains("  L5: fn main() {"));
    }

    #[test]
    fn test_output_formatter_no_line_number() {
        let formatter = OutputFormatter::with_color(false);
        let lines = vec![
            DiffLine {
                kind: DiffLineKind::Added,
                line_number: None,
                content: "new line".to_string(),
            },
            DiffLine {
                kind: DiffLineKind::Deleted,
                line_number: None,
                content: "deleted line".to_string(),
            },
        ];
        let output = formatter.format_diff_lines(&lines);
        assert!(output.contains("+ new line"));
        assert!(output.contains("- deleted line"));
    }

    #[test]
    fn test_output_formatter_format_block() {
        use crate::model::{ContentBlock, Line, MatchInfo};
        let block = ContentBlock {
            start_line: 10,
            end_line: 11,
            match_info: MatchInfo::Location {
                matched_line_count: 1,
            },
            lines: vec![
                Line {
                    line_num: 10,
                    taps: 0,
                    diff_taps: 0,
                    content: "fn foo() {".to_string(),
                    stripped_content: crate::model::stripped_content("fn foo() {"),
                },
                Line {
                    line_num: 11,
                    taps: 4,
                    diff_taps: 4,
                    content: "    bar();".to_string(),
                    stripped_content: crate::model::stripped_content("    bar();"),
                },
            ],
        };
        let formatter = OutputFormatter::with_color(false);
        let output = formatter.format_block(&block);
        assert!(output.contains("fn foo() {"));
        assert!(output.contains("bar();"));
    }

    #[test]
    fn test_output_formatter_empty_lines() {
        let formatter = OutputFormatter::with_color(false);
        let output = formatter.format_diff_lines(&[]);
        assert_eq!(output, "");
    }
}
