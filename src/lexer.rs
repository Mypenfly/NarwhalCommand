//! 词法分析器 (Lexer)
//!
//! 负责将输入的 .ned 脚本内容扫描为 Token 流。
//!
//! ## 实现逻辑
//!
//! 1. 逐行读取脚本内容，识别 `//!@` 标识符作为命令起始
//! 2. 根据命令头（Open/Location/New/Delete/Raw/Off）切分不同的命令块
//! 3. 命令块内的内容按行收集，遇到分隔符 `...` 或下一命令时终止
//! 4. 输出有序的 Token 序列供 Parser 使用
//!
//! ## 对应文档
//!
//! 详见 INSTRUCTION.md 第 1.1 节 "架构总览"

use crate::model::LineNumber;

/// 词法分析器产出的 Token
#[derive(Debug, PartialEq)]
pub enum Token {
    /// Open 命令：打开目标文件
    Open {
        /// 目标文件路径
        file_path: String,
        /// Token 所在行号
        line: LineNumber,
    },
    /// Location 命令：定位代码位置
    Location {
        /// 是否为 Location:Block（Phase 3）
        block: bool,
        /// 定位内容的所有行（不含 `//!@` 前缀和 `...` 分隔符）
        lines: Vec<String>,
        /// Token 所在行号
        line: LineNumber,
    },
    /// New 命令：插入新内容
    New {
        /// 插入位置标识（"Normal" / "Start" / "End"）
        position: String,
        /// 插入内容的所有行（不含 `//!@` 前缀和 `...` 分隔符）
        lines: Vec<String>,
        /// Token 所在行号
        line: LineNumber,
    },
    /// Delete 命令：删除匹配内容
    Delete {
        /// 是否为 Delete:Block（Phase 3）
        block: bool,
        /// 匹配内容的所有行（不含 `//!@` 前缀和 `...` 分隔符）
        lines: Vec<String>,
        /// Token 所在行号
        line: LineNumber,
    },
    /// Off 命令：关闭当前作用域
    Off {
        /// 关闭目标（Open / Location / New）
        target: String,
        /// Token 所在行号
        line: LineNumber,
    },
    /// 分隔符 `...`：终止上一个命令的内容提取
    Separator {
        /// Token 所在行号
        line: LineNumber,
    },
}

/// 词法分析器
///
/// 接收 .ned 脚本全文，输出 Token 序列。
pub struct Lexer;

impl Lexer {
    /// 对脚本内容执行词法分析，返回 Token 流
    pub fn tokenize(script: &str) -> Vec<Token> {
        let mut tokens: Vec<Token> = Vec::new();
        let mut lines = script.lines().enumerate().peekable();

        while let Some((line_index, line)) = lines.next() {
            let trimmed = line.trim();
            let line_number = LineNumber::from_index(line_index);

            // 独立的分隔符 `...`，不依附于任何命令
            if trimmed == "..." {
                tokens.push(Token::Separator { line: line_number });
                continue;
            }

            if !trimmed.starts_with("//!@") {
                continue;
            }

            let command_part = trimmed.strip_prefix("//!@").unwrap_or(trimmed);

            if let Some(rest) = command_part.strip_prefix("Open:") {
                let file_path = rest.trim().to_string();
                tokens.push(Token::Open {
                    file_path,
                    line: line_number,
                });
            } else if command_part.starts_with("Location:") {
                let remaining = command_part.strip_prefix("Location:").unwrap_or("");
                // Location:Block — Block 是修饰符而非内容
                let is_block = remaining.trim() == "Block";
                let content_remaining = if is_block { "" } else { remaining };
                let content_lines = Self::extract_command_content(&mut lines, content_remaining);
                tokens.push(Token::Location {
                    block: is_block,
                    lines: content_lines,
                    line: line_number,
                });
            } else if command_part.starts_with("New:Start") {
                let content_lines = Self::extract_command_content(
                    &mut lines,
                    command_part.strip_prefix("New:Start").unwrap_or(""),
                );
                tokens.push(Token::New {
                    position: "Start".to_string(),
                    lines: content_lines,
                    line: line_number,
                });
            } else if command_part.starts_with("New:End") {
                let content_lines = Self::extract_command_content(
                    &mut lines,
                    command_part.strip_prefix("New:End").unwrap_or(""),
                );
                tokens.push(Token::New {
                    position: "End".to_string(),
                    lines: content_lines,
                    line: line_number,
                });
            } else if command_part.starts_with("New:") {
                let content_lines = Self::extract_command_content(
                    &mut lines,
                    command_part.strip_prefix("New:").unwrap_or(""),
                );
                tokens.push(Token::New {
                    position: "Normal".to_string(),
                    lines: content_lines,
                    line: line_number,
                });
            } else if command_part.starts_with("Delete:") {
                let remaining = command_part.strip_prefix("Delete:").unwrap_or("");
                // Delete:Block — Block 是修饰符而非内容（Phase 3）
                let is_block = remaining.trim() == "Block";
                let content_remaining = if is_block { "" } else { remaining };
                let content_lines = Self::extract_command_content(&mut lines, content_remaining);
                tokens.push(Token::Delete {
                    block: is_block,
                    lines: content_lines,
                    line: line_number,
                });
            } else if let Some(rest) = command_part.strip_prefix("Off:") {
                let target = rest.trim().to_string();
                tokens.push(Token::Off {
                    target,
                    line: line_number,
                });
            }
        }

        tokens
    }

    /// 提取命令的后续内容行
    ///
    /// 检查命令行的剩余文本作为首行，
    /// 然后继续读行直到遇到 `...` 分隔符或下一个 `//!@` 命令。
    fn extract_command_content(
        lines: &mut std::iter::Peekable<std::iter::Enumerate<std::str::Lines<'_>>>,
        remaining: &str,
    ) -> Vec<String> {
        let mut content_lines: Vec<String> = Vec::new();
        let remaining = remaining.trim();
        if !remaining.is_empty() {
            content_lines.push(remaining.to_string());
        }
        while let Some((_, next_line)) = lines.peek() {
            let next_trimmed = next_line.trim();
            if next_trimmed.starts_with("//!@") {
                break;
            }
            if next_trimmed == "..." {
                // 不消费此行，由主循环作为 Separator token 处理
                break;
            }
            content_lines.push(next_line.to_string());
            lines.next();
        }
        content_lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================================
    // Token 结构测试
    // ============================================================

    #[test]
    fn test_token_open_creation() {
        let token = Token::Open {
            file_path: "./test.rs".to_string(),
            line: LineNumber::new(1),
        };
        match token {
            Token::Open { file_path, line } => {
                assert_eq!(file_path, "./test.rs");
                assert_eq!(line, 1);
            }
            _ => panic!("Expected Open token"),
        }
    }

    #[test]
    fn test_token_location_creation() {
        let token = Token::Location {
            block: false,
            lines: vec!["fn main() {".to_string(), "    let x = 1;".to_string()],
            line: LineNumber::new(3),
        };
        match token {
            Token::Location { lines, line, .. } => {
                assert_eq!(lines.len(), 2);
                assert_eq!(lines[0], "fn main() {");
                assert_eq!(line, 3);
            }
            _ => panic!("Expected Location token"),
        }
    }

    #[test]
    fn test_token_off_creation() {
        let token = Token::Off {
            target: "Open".to_string(),
            line: LineNumber::new(10),
        };
        match token {
            Token::Off { target, line } => {
                assert_eq!(target, "Open");
                assert_eq!(line, 10);
            }
            _ => panic!("Expected Off token"),
        }
    }

    // ============================================================
    // Lexer::tokenize 测试
    // ============================================================

    #[test]
    fn test_lexer_empty_script() {
        let tokens = Lexer::tokenize("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_lexer_single_open_token() {
        let script = "//!@Open: ./src/main.rs\n";
        let tokens = Lexer::tokenize(script);
        assert_eq!(tokens.len(), 1);
        assert_eq!(
            tokens[0],
            Token::Open {
                file_path: "./src/main.rs".to_string(),
                line: LineNumber::new(1),
            }
        );
    }

    #[test]
    fn test_lexer_single_off_open_token() {
        let script = "//!@Off:Open\n";
        let tokens = Lexer::tokenize(script);
        assert_eq!(tokens.len(), 1);
        assert_eq!(
            tokens[0],
            Token::Off {
                target: "Open".to_string(),
                line: LineNumber::new(1),
            }
        );
    }

    #[test]
    fn test_lexer_single_off_location_token() {
        let script = "//!@Off:Location\n";
        let tokens = Lexer::tokenize(script);
        assert_eq!(tokens.len(), 1);
        assert_eq!(
            tokens[0],
            Token::Off {
                target: "Location".to_string(),
                line: LineNumber::new(1),
            }
        );
    }

    #[test]
    fn test_lexer_location_with_content_until_separator() {
        let script = "\
//!@Location:
fn main() {
    let x = 1;
...
//!@Off:Open
";
        let tokens = Lexer::tokenize(script);
        assert_eq!(tokens.len(), 3);

        match &tokens[0] {
            Token::Location { lines, line, .. } => {
                assert_eq!(*line, 1);
                assert_eq!(lines.len(), 2);
                assert_eq!(lines[0], "fn main() {");
                assert_eq!(lines[1], "    let x = 1;");
            }
            _ => panic!("Expected Location token"),
        }

        assert_eq!(
            tokens[1],
            Token::Separator {
                line: LineNumber::new(4)
            }
        );

        assert_eq!(
            tokens[2],
            Token::Off {
                target: "Open".to_string(),
                line: LineNumber::new(5),
            }
        );
    }

    #[test]
    fn test_lexer_location_with_content_until_next_command() {
        let script = "\
//!@Location:
fn foo() {
    bar();
//!@Off:Location
";
        let tokens = Lexer::tokenize(script);
        assert_eq!(tokens.len(), 2);

        match &tokens[0] {
            Token::Location { lines, line, .. } => {
                assert_eq!(*line, 1);
                assert_eq!(lines.len(), 2);
                assert_eq!(lines[0], "fn foo() {");
                assert_eq!(lines[1], "    bar();");
            }
            _ => panic!("Expected Location token"),
        }
    }

    #[test]
    fn test_lexer_open_location_off_sequence() {
        let script = "\
//!@Open: ./test.rs
//!@Location:
fn main() {
...
//!@Off:Open
";
        let tokens = Lexer::tokenize(script);
        assert_eq!(tokens.len(), 4);

        assert_eq!(
            tokens[0],
            Token::Open {
                file_path: "./test.rs".to_string(),
                line: LineNumber::new(1),
            }
        );

        match &tokens[1] {
            Token::Location { lines, line, .. } => {
                assert_eq!(*line, 2);
                assert_eq!(lines.len(), 1);
                assert_eq!(lines[0], "fn main() {");
            }
            _ => panic!("Expected Location token"),
        }

        assert_eq!(
            tokens[2],
            Token::Separator {
                line: LineNumber::new(4)
            }
        );

        assert_eq!(
            tokens[3],
            Token::Off {
                target: "Open".to_string(),
                line: LineNumber::new(5),
            }
        );
    }

    #[test]
    fn test_lexer_ignores_non_command_lines() {
        let script = "\
Some text here
//!@Open: ./file.rs
More text
//!@Off:Open
";
        let tokens = Lexer::tokenize(script);
        assert_eq!(tokens.len(), 2);
    }

    #[test]
    fn test_lexer_line_numbers_correct() {
        let script = "\
// comment line 1
//!@Open: ./test.rs
// comment line 3
//!@Location:
match_me
...
//!@Off:Open
";
        let tokens = Lexer::tokenize(script);
        assert_eq!(tokens.len(), 4);

        assert_eq!(
            tokens[0],
            Token::Open {
                file_path: "./test.rs".to_string(),
                line: LineNumber::new(2),
            }
        );

        match &tokens[1] {
            Token::Location { line, .. } => {
                assert_eq!(*line, 4);
            }
            _ => panic!("Expected Location"),
        }

        assert_eq!(
            tokens[2],
            Token::Separator {
                line: LineNumber::new(6)
            }
        );

        assert_eq!(
            tokens[3],
            Token::Off {
                target: "Open".to_string(),
                line: LineNumber::new(7),
            }
        );
    }

    // ============================================================
    // New Token 测试
    // ============================================================

    #[test]
    fn test_lexer_new_normal_token() {
        let script = "\
//!@New:
    let x = 1;
...
//!@Off:Open
";
        let tokens = Lexer::tokenize(script);
        assert_eq!(tokens.len(), 3);

        match &tokens[0] {
            Token::New {
                position,
                lines,
                line,
            } => {
                assert_eq!(position, "Normal");
                assert_eq!(*line, 1);
                assert_eq!(lines.len(), 1);
                assert_eq!(lines[0], "    let x = 1;");
            }
            _ => panic!("Expected New token"),
        }

        assert_eq!(
            tokens[1],
            Token::Separator {
                line: LineNumber::new(3)
            }
        );

        assert_eq!(
            tokens[2],
            Token::Off {
                target: "Open".to_string(),
                line: LineNumber::new(4),
            }
        );
    }

    #[test]
    fn test_lexer_new_start_token() {
        let script = "//!@New:Start\n    let x = 1;\n...\n";
        let tokens = Lexer::tokenize(script);
        assert_eq!(tokens.len(), 2);

        match &tokens[0] {
            Token::New {
                position,
                lines,
                line,
            } => {
                assert_eq!(position, "Start");
                assert_eq!(*line, 1);
                assert_eq!(lines.len(), 1);
                assert_eq!(lines[0], "    let x = 1;");
            }
            _ => panic!("Expected New:Start token"),
        }

        assert_eq!(
            tokens[1],
            Token::Separator {
                line: LineNumber::new(3)
            }
        );
    }

    #[test]
    fn test_lexer_new_end_token() {
        let script = "//!@New:End\n    let x = 1;\n...\n";
        let tokens = Lexer::tokenize(script);
        assert_eq!(tokens.len(), 2);

        match &tokens[0] {
            Token::New {
                position,
                lines,
                line,
            } => {
                assert_eq!(position, "End");
                assert_eq!(*line, 1);
                assert_eq!(lines.len(), 1);
                assert_eq!(lines[0], "    let x = 1;");
            }
            _ => panic!("Expected New:End token"),
        }

        assert_eq!(
            tokens[1],
            Token::Separator {
                line: LineNumber::new(3)
            }
        );
    }

    #[test]
    fn test_lexer_new_content_until_next_command() {
        let script = "\
//!@New:
let a = 1;
let b = 2;
//!@Off:Location
";
        let tokens = Lexer::tokenize(script);
        assert_eq!(tokens.len(), 2);

        match &tokens[0] {
            Token::New {
                position, lines, ..
            } => {
                assert_eq!(position, "Normal");
                assert_eq!(lines.len(), 2);
                assert_eq!(lines[0], "let a = 1;");
                assert_eq!(lines[1], "let b = 2;");
            }
            _ => panic!("Expected New token"),
        }
    }

    #[test]
    fn test_lexer_new_empty_content() {
        let script = "//!@New:\n//!@Off:Location\n";
        let tokens = Lexer::tokenize(script);
        assert_eq!(tokens.len(), 2);

        match &tokens[0] {
            Token::New {
                position, lines, ..
            } => {
                assert_eq!(position, "Normal");
                assert_eq!(lines.len(), 0);
            }
            _ => panic!("Expected New token"),
        }
    }

    // ============================================================
    // Delete Token 测试
    // ============================================================

    #[test]
    fn test_lexer_delete_token() {
        let script = "\
//!@Delete:
let x = 1;
...
//!@Off:Open
";
        let tokens = Lexer::tokenize(script);
        assert_eq!(tokens.len(), 3);

        match &tokens[0] {
            Token::Delete { lines, line, .. } => {
                assert_eq!(*line, 1);
                assert_eq!(lines.len(), 1);
                assert_eq!(lines[0], "let x = 1;");
            }
            _ => panic!("Expected Delete token"),
        }

        assert_eq!(
            tokens[1],
            Token::Separator {
                line: LineNumber::new(3)
            }
        );

        assert_eq!(
            tokens[2],
            Token::Off {
                target: "Open".to_string(),
                line: LineNumber::new(4),
            }
        );
    }

    #[test]
    fn test_lexer_delete_content_until_next_command() {
        let script = "\
//!@Delete:
let a = 1;
let b = 2;
//!@Off:Location
";
        let tokens = Lexer::tokenize(script);
        assert_eq!(tokens.len(), 2);

        match &tokens[0] {
            Token::Delete { lines, .. } => {
                assert_eq!(lines.len(), 2);
                assert_eq!(lines[0], "let a = 1;");
                assert_eq!(lines[1], "let b = 2;");
            }
            _ => panic!("Expected Delete token"),
        }
    }

    // ============================================================
    // New + Delete 集成测试
    // ============================================================

    #[test]
    fn test_lexer_open_location_new_delete_off_sequence() {
        let script = "\
//!@Open: ./test.rs
//!@Location:
fn main() {
...
//!@New:
    let x = 1;
...
//!@Delete:
let y = 2;
...
//!@Off:Open
";
        let tokens = Lexer::tokenize(script);
        assert_eq!(tokens.len(), 8);
        assert!(matches!(tokens[0], Token::Open { .. }));
        assert!(matches!(tokens[1], Token::Location { .. }));
        assert!(matches!(tokens[2], Token::Separator { .. }));
        assert!(matches!(tokens[3], Token::New { .. }));
        assert!(matches!(tokens[4], Token::Separator { .. }));
        assert!(matches!(tokens[5], Token::Delete { .. }));
        assert!(matches!(tokens[6], Token::Separator { .. }));
        assert!(matches!(tokens[7], Token::Off { .. }));
    }

    // ============================================================
    // Location:Block / Delete:Block Token 测试 (Phase 3)
    // ============================================================

    #[test]
    fn test_lexer_location_block_token() {
        let script = "//!@Location:Block\nfn main() {\n...\n";
        let tokens = Lexer::tokenize(script);
        assert_eq!(tokens.len(), 2);

        match &tokens[0] {
            Token::Location { block, lines, line } => {
                assert!(block, "Location:Block should have block=true");
                assert_eq!(*line, 1);
                assert_eq!(lines.len(), 1);
                assert_eq!(lines[0], "fn main() {");
            }
            _ => panic!("Expected Location token"),
        }

        assert_eq!(
            tokens[1],
            Token::Separator {
                line: LineNumber::new(3)
            }
        );
    }

    #[test]
    fn test_lexer_delete_block_token() {
        let script = "//!@Delete:Block\nlet x = 1;\n...\n";
        let tokens = Lexer::tokenize(script);
        assert_eq!(tokens.len(), 2);

        match &tokens[0] {
            Token::Delete { block, lines, line } => {
                assert!(block, "Delete:Block should have block=true");
                assert_eq!(*line, 1);
                assert_eq!(lines.len(), 1);
                assert_eq!(lines[0], "let x = 1;");
            }
            _ => panic!("Expected Delete token"),
        }

        assert_eq!(
            tokens[1],
            Token::Separator {
                line: LineNumber::new(3)
            }
        );
    }

    #[test]
    fn test_lexer_location_normal_has_block_false() {
        let script = "//!@Location:\nfn main() {\n...\n";
        let tokens = Lexer::tokenize(script);

        match &tokens[0] {
            Token::Location { block, .. } => {
                assert!(!block, "Normal Location should have block=false");
            }
            _ => panic!("Expected Location token"),
        }
    }

    #[test]
    fn test_lexer_delete_normal_has_block_false() {
        let script = "//!@Delete:\nlet x = 1;\n...\n";
        let tokens = Lexer::tokenize(script);

        match &tokens[0] {
            Token::Delete { block, .. } => {
                assert!(!block, "Normal Delete should have block=false");
            }
            _ => panic!("Expected Delete token"),
        }
    }
}
