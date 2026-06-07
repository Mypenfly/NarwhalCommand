//! 语法分析器 (Parser)
//!
//! 负责将 Lexer 输出的 Token 流组装为 AST（Command 序列）。
//!
//! ## 实现逻辑
//!
//! 1. 消费 Token 流，按命令类型构建对应的 Command 枚举变体
//! 2. 处理 LocationContent 中的 diff_taps 计算
//! 3. 执行语法校验（如 Off 命令的 target 是否合法）
//!
//! ## 对应文档
//!
//! 详见 INSTRUCTION.md 第 1.1 节 "架构总览"

use crate::error::ParseError;
use crate::lexer::Token;
use crate::model::{DeleteContent, DeleteLine, LocationContent, LocationLine, NewContent, NewLine};

/// 一条完整的命令语句（AST 节点）
#[derive(Debug, PartialEq)]
pub enum Command {
    /// Open 命令：打开目标文件
    Open {
        /// 目标文件路径
        file_path: String,
    },
    /// Location 命令：定位代码位置
    Location {
        /// 是否为 Location:Block（Phase 3）
        block: bool,
        /// 用于匹配的定位内容
        content: LocationContent,
    },
    /// New 命令：插入新内容
    New {
        /// 插入位置（Normal / Start / End）
        position: NewPosition,
        /// 待插入的内容
        content: NewContent,
    },
    /// Delete 命令：删除匹配内容
    Delete {
        /// 是否为 Delete:Block（Phase 3）
        block: bool,
        /// 用于匹配的删除内容（非 Block 时有内容）
        content: Option<DeleteContent>,
    },
    /// Off 命令：关闭当前作用域
    Off {
        /// 关闭目标
        target: OffTarget,
    },
}

/// New 命令的插入位置
#[derive(Debug, PartialEq)]
pub enum NewPosition {
    /// 插入到 Location 最后一行之后
    Normal,
    /// 插入到文件开头
    Start,
    /// 插入到文件末尾
    End,
}

/// Off 命令的关闭目标
#[derive(Debug, PartialEq)]
pub enum OffTarget {
    /// 关闭 Open 作用域（最终写回文件）
    Open,
    /// 关闭 Location 作用域（写回上层 block）
    Location,
    /// 关闭 New 作用域（写回上层 block 并出栈）
    New,
}

/// 语法分析器
///
/// 消费 Token 流，输出 Command 序列。
pub struct Parser;

impl Parser {
    /// 将 Token 序列解析为 Command 序列
    ///
    /// 对每个 Token 进行语法校验，构建对应的 Command AST 节点。
    /// 同时校验 New/Delete 必须在 Location 之后（且之间不能有 `...` 分隔符）。
    pub fn parse(tokens: Vec<Token>) -> Result<Vec<Command>, ParseError> {
        let mut commands: Vec<Command> = Vec::new();
        let mut last_was_location = false;
        // Phase 3: 追踪上一个 Location 是否使用了 Block 指令
        let mut last_location_was_block = false;

        for token in tokens {
            match token {
                Token::Open { file_path, line: _ } => {
                    if file_path.is_empty() {
                        return Err(ParseError::MissingFilePath);
                    }
                    commands.push(Command::Open { file_path });
                    last_was_location = false;
                    last_location_was_block = false;
                }
                Token::Location {
                    block,
                    lines,
                    line: _,
                } => {
                    let location_content = build_location_content(lines);
                    commands.push(Command::Location {
                        block,
                        content: location_content,
                    });
                    last_was_location = true;
                    last_location_was_block = block;
                }
                Token::Separator { .. } => {
                    // `...` 分隔符重置 Location 追踪状态：
                    // 后续的 New/Delete 再也不知道在何处操作
                    last_was_location = false;
                    last_location_was_block = false;
                }
                Token::New {
                    position,
                    lines,
                    line,
                } => {
                    let new_position = match position.as_str() {
                        "Normal" => {
                            if !last_was_location {
                                return Err(ParseError::MissingLocation {
                                    command: "New".to_string(),
                                    line,
                                });
                            }
                            NewPosition::Normal
                        }
                        "Start" => NewPosition::Start,
                        "End" => NewPosition::End,
                        _ => {
                            return Err(ParseError::UnknownCommand {
                                token: format!("New:{}", position),
                                line,
                            });
                        }
                    };
                    let new_content = build_new_content(lines);
                    commands.push(Command::New {
                        position: new_position,
                        content: new_content,
                    });
                }
                Token::Delete { block, lines, line } => {
                    if !last_was_location {
                        return Err(ParseError::MissingLocation {
                            command: "Delete".to_string(),
                            line,
                        });
                    }
                    // Phase 3 语法校验：Delete:Block 要求前一个 Location 也使用 Block
                    if block && !last_location_was_block {
                        return Err(ParseError::BlockRequiredForDelete { line });
                    }
                    let delete_content = build_delete_content(lines);
                    commands.push(Command::Delete {
                        block,
                        content: Some(delete_content),
                    });
                }
                Token::Off { target, line } => {
                    let off_target = match target.as_str() {
                        "Open" => OffTarget::Open,
                        "Location" => OffTarget::Location,
                        "New" => OffTarget::New,
                        _ => {
                            return Err(ParseError::UnknownCommand {
                                token: format!("Off:{}", target),
                                line,
                            });
                        }
                    };
                    commands.push(Command::Off { target: off_target });
                }
            }
        }

        Ok(commands)
    }
}

/// 从 Lexer 产出的原始文本行构建 LocationContent
///
/// 计算每行的 diff_taps（相对于首行的缩进差异）。
/// diff_taps 为相对值：后行 taps 减去首行 taps，若为负则取 0。
fn build_location_content(raw_lines: Vec<String>) -> LocationContent {
    if raw_lines.is_empty() {
        return LocationContent { lines: vec![] };
    }

    let base_taps = crate::model::count_leading_spaces(&raw_lines[0]);

    let lines: Vec<LocationLine> = raw_lines
        .into_iter()
        .enumerate()
        .map(|(index, content)| {
            let line_taps = crate::model::count_leading_spaces(&content);
            let diff_taps = Some(line_taps.saturating_sub(base_taps));
            LocationLine {
                index,
                diff_taps,
                content,
                line_num: None,
            }
        })
        .collect();

    LocationContent { lines }
}

/// 从 Lexer 产出的原始文本行构建 NewContent
///
/// 计算每行的 diff_taps（绝对缩进量），内容为去除首部缩进后的文本。
fn build_new_content(raw_lines: Vec<String>) -> NewContent {
    if raw_lines.is_empty() {
        return NewContent { lines: vec![] };
    }

    let lines: Vec<NewLine> = raw_lines
        .into_iter()
        .map(|content| {
            let line_taps = crate::model::count_leading_spaces(&content);
            let stripped_content = content[line_taps..].to_string();
            NewLine {
                diff_taps: line_taps,
                content: stripped_content,
                is_raw: false,
            }
        })
        .collect();

    NewContent { lines }
}

/// 从 Lexer 产出的原始文本行构建 DeleteContent
///
/// 直接保留原始行内容，用于后续逐行匹配。
fn build_delete_content(raw_lines: Vec<String>) -> DeleteContent {
    let lines: Vec<DeleteLine> = raw_lines
        .into_iter()
        .map(|content| DeleteLine {
            content,
            is_raw: false,
        })
        .collect();

    DeleteContent { lines }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Token;
    use crate::model::LineNumber;

    // ============================================================
    // Command / OffTarget 创建测试
    // ============================================================

    #[test]
    fn test_command_open_creation() {
        let command = Command::Open {
            file_path: "./test.rs".to_string(),
        };
        match command {
            Command::Open { file_path } => assert_eq!(file_path, "./test.rs"),
            _ => panic!("Expected Open command"),
        }
    }

    #[test]
    fn test_command_location_creation() {
        let content = LocationContent {
            lines: vec![LocationLine {
                index: 0,
                diff_taps: Some(0),
                content: "fn main() {".to_string(),
                line_num: None,
            }],
        };
        let command = Command::Location {
            block: false,
            content,
        };
        match command {
            Command::Location { block, content } => {
                assert!(!block);
                assert_eq!(content.lines.len(), 1);
            }
            _ => panic!("Expected Location command"),
        }
    }

    #[test]
    fn test_command_off_creation() {
        let command = Command::Off {
            target: OffTarget::Open,
        };
        match command {
            Command::Off { target } => assert_eq!(target, OffTarget::Open),
            _ => panic!("Expected Off command"),
        }
    }

    #[test]
    fn test_off_target_location() {
        let target = OffTarget::Location;
        assert_eq!(target, OffTarget::Location);
    }

    // ============================================================
    // Parser::parse 测试
    // ============================================================

    #[test]
    fn test_parse_single_open() {
        let tokens = vec![Token::Open {
            file_path: "./test.rs".to_string(),
            line: LineNumber::new(1),
        }];
        let commands = Parser::parse(tokens).unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            Command::Open { file_path } => assert_eq!(file_path, "./test.rs"),
            _ => panic!("Expected Open"),
        }
    }

    #[test]
    fn test_parse_single_off_open() {
        let tokens = vec![Token::Off {
            target: "Open".to_string(),
            line: LineNumber::new(1),
        }];
        let commands = Parser::parse(tokens).unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            Command::Off { target } => assert_eq!(*target, OffTarget::Open),
            _ => panic!("Expected Off"),
        }
    }

    #[test]
    fn test_parse_single_off_location() {
        let tokens = vec![Token::Off {
            target: "Location".to_string(),
            line: LineNumber::new(1),
        }];
        let commands = Parser::parse(tokens).unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            Command::Off { target } => assert_eq!(*target, OffTarget::Location),
            _ => panic!("Expected Off"),
        }
    }

    #[test]
    fn test_parse_location_with_content() {
        let tokens = vec![Token::Location {
            block: false,
            lines: vec!["fn main() {".to_string(), "    let x = 1;".to_string()],
            line: LineNumber::new(2),
        }];
        let commands = Parser::parse(tokens).unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            Command::Location { content, .. } => {
                assert_eq!(content.lines.len(), 2);
                assert_eq!(content.lines[0].content, "fn main() {");
                assert_eq!(content.lines[0].index, 0);
                assert_eq!(content.lines[1].content, "    let x = 1;");
                assert_eq!(content.lines[1].index, 1);
            }
            _ => panic!("Expected Location"),
        }
    }

    #[test]
    fn test_parse_location_calculates_diff_taps() {
        let tokens = vec![Token::Location {
            block: false,
            lines: vec![
                "fn main() {".to_string(),
                "    let x = 1;".to_string(),
                "        let y = 2;".to_string(),
            ],
            line: LineNumber::new(2),
        }];
        let commands = Parser::parse(tokens).unwrap();
        match &commands[0] {
            Command::Location { content, .. } => {
                assert_eq!(content.lines[0].diff_taps, Some(0));
                assert_eq!(content.lines[1].diff_taps, Some(4));
                assert_eq!(content.lines[2].diff_taps, Some(8));
            }
            _ => panic!("Expected Location"),
        }
    }

    #[test]
    fn test_parse_location_diff_taps_relative_to_first_line() {
        let tokens = vec![Token::Location {
            block: false,
            lines: vec![
                "        deep indent".to_string(),
                "    less indent".to_string(),
                "            deeper".to_string(),
            ],
            line: LineNumber::new(2),
        }];
        let commands = Parser::parse(tokens).unwrap();
        match &commands[0] {
            Command::Location { content, .. } => {
                assert_eq!(content.lines[0].diff_taps, Some(0));
                assert_eq!(content.lines[1].diff_taps, Some(0)); // less than first = 0
                assert_eq!(content.lines[2].diff_taps, Some(4)); // 12 - 8 = 4
            }
            _ => panic!("Expected Location"),
        }
    }

    #[test]
    fn test_parse_open_location_off_sequence() {
        let tokens = vec![
            Token::Open {
                file_path: "./test.rs".to_string(),
                line: LineNumber::new(1),
            },
            Token::Location {
                block: false,
                lines: vec!["fn main() {".to_string()],
                line: LineNumber::new(2),
            },
            Token::Off {
                target: "Open".to_string(),
                line: LineNumber::new(4),
            },
        ];
        let commands = Parser::parse(tokens).unwrap();
        assert_eq!(commands.len(), 3);
        match &commands[0] {
            Command::Open { file_path } => assert_eq!(file_path, "./test.rs"),
            _ => panic!("Expected Open"),
        }
        match &commands[1] {
            Command::Location { .. } => {}
            _ => panic!("Expected Location"),
        }
        match &commands[2] {
            Command::Off { target } => assert_eq!(*target, OffTarget::Open),
            _ => panic!("Expected Off"),
        }
    }

    #[test]
    fn test_parse_open_missing_file_path() {
        let tokens = vec![Token::Open {
            file_path: "".to_string(),
            line: LineNumber::new(1),
        }];
        let result = Parser::parse(tokens);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_off_invalid_target() {
        let tokens = vec![Token::Off {
            target: "Invalid".to_string(),
            line: LineNumber::new(1),
        }];
        let result = Parser::parse(tokens);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_empty_tokens() {
        let tokens = vec![];
        let commands = Parser::parse(tokens).unwrap();
        assert!(commands.is_empty());
    }

    // ============================================================
    // New 命令解析测试
    // ============================================================

    #[test]
    fn test_parse_new_normal() {
        let tokens = vec![
            Token::Location {
                block: false,
                lines: vec!["fn main() {".to_string()],
                line: LineNumber::new(2),
            },
            Token::New {
                position: "Normal".to_string(),
                lines: vec!["    let x = 1;".to_string()],
                line: LineNumber::new(3),
            },
        ];
        let commands = Parser::parse(tokens).unwrap();
        assert_eq!(commands.len(), 2);
        match &commands[1] {
            Command::New { position, content } => {
                assert_eq!(*position, NewPosition::Normal);
                assert_eq!(content.lines.len(), 1);
                assert_eq!(content.lines[0].diff_taps, 4);
                assert_eq!(content.lines[0].content, "let x = 1;");
            }
            _ => panic!("Expected New command"),
        }
    }

    #[test]
    fn test_parse_new_start() {
        let tokens = vec![Token::New {
            position: "Start".to_string(),
            lines: vec!["// SPDX-License".to_string()],
            line: LineNumber::new(1),
        }];
        let commands = Parser::parse(tokens).unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            Command::New { position, content } => {
                assert_eq!(*position, NewPosition::Start);
                assert_eq!(content.lines.len(), 1);
                assert_eq!(content.lines[0].diff_taps, 0);
                assert_eq!(content.lines[0].content, "// SPDX-License");
            }
            _ => panic!("Expected New:Start command"),
        }
    }

    #[test]
    fn test_parse_new_end() {
        let tokens = vec![Token::New {
            position: "End".to_string(),
            lines: vec!["// END".to_string()],
            line: LineNumber::new(5),
        }];
        let commands = Parser::parse(tokens).unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            Command::New { position, content } => {
                assert_eq!(*position, NewPosition::End);
                assert_eq!(content.lines.len(), 1);
            }
            _ => panic!("Expected New:End command"),
        }
    }

    #[test]
    fn test_parse_new_content_calculates_diff_taps() {
        let tokens = vec![
            Token::Location {
                block: false,
                lines: vec!["fn main() {".to_string()],
                line: LineNumber::new(2),
            },
            Token::New {
                position: "Normal".to_string(),
                lines: vec![
                    "fn foo() {".to_string(),
                    "    bar();".to_string(),
                    "        baz();".to_string(),
                ],
                line: LineNumber::new(3),
            },
        ];
        let commands = Parser::parse(tokens).unwrap();
        match &commands[1] {
            Command::New { content, .. } => {
                assert_eq!(content.lines.len(), 3);
                assert_eq!(content.lines[0].diff_taps, 0);
                assert_eq!(content.lines[0].content, "fn foo() {");
                assert_eq!(content.lines[1].diff_taps, 4);
                assert_eq!(content.lines[1].content, "bar();");
                assert_eq!(content.lines[2].diff_taps, 8);
                assert_eq!(content.lines[2].content, "baz();");
            }
            _ => panic!("Expected New command"),
        }
    }

    // ============================================================
    // Delete 命令解析测试
    // ============================================================

    #[test]
    fn test_parse_delete() {
        let tokens = vec![
            Token::Location {
                block: false,
                lines: vec!["fn main() {".to_string()],
                line: LineNumber::new(2),
            },
            Token::Delete {
                block: false,
                lines: vec!["    let x = 1;".to_string()],
                line: LineNumber::new(4),
            },
        ];
        let commands = Parser::parse(tokens).unwrap();
        assert_eq!(commands.len(), 2);
        match &commands[1] {
            Command::Delete { block, content } => {
                assert!(!block);
                assert!(content.is_some());
                let del_content = content.as_ref().unwrap();
                assert_eq!(del_content.lines.len(), 1);
                assert_eq!(del_content.lines[0].content, "    let x = 1;");
            }
            _ => panic!("Expected Delete command"),
        }
    }

    #[test]
    fn test_parse_delete_multiple_lines() {
        let tokens = vec![
            Token::Location {
                block: false,
                lines: vec!["fn main() {".to_string()],
                line: LineNumber::new(2),
            },
            Token::Delete {
                block: false,
                lines: vec!["line1".to_string(), "line2".to_string()],
                line: LineNumber::new(5),
            },
        ];
        let commands = Parser::parse(tokens).unwrap();
        match &commands[1] {
            Command::Delete { content, .. } => {
                let del_content = content.as_ref().unwrap();
                assert_eq!(del_content.lines.len(), 2);
            }
            _ => panic!("Expected Delete command"),
        }
    }

    // ============================================================
    // Off:New 解析测试
    // ============================================================

    #[test]
    fn test_parse_off_new_target() {
        let tokens = vec![Token::Off {
            target: "New".to_string(),
            line: LineNumber::new(6),
        }];
        let commands = Parser::parse(tokens).unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            Command::Off { target } => assert_eq!(*target, OffTarget::New),
            _ => panic!("Expected Off"),
        }
    }

    #[test]
    fn test_off_target_new_value() {
        let target = OffTarget::New;
        assert_eq!(target, OffTarget::New);
    }

    // ============================================================
    // Phase 3: Location:Block / Delete:Block 解析测试
    // ============================================================

    #[test]
    fn test_parse_location_block() {
        let tokens = vec![
            Token::Open {
                file_path: "./test.rs".to_string(),
                line: LineNumber::new(1),
            },
            Token::Location {
                block: true,
                lines: vec!["fn example() {".to_string()],
                line: LineNumber::new(2),
            },
        ];
        let commands = Parser::parse(tokens).unwrap();
        assert_eq!(commands.len(), 2);
        match &commands[1] {
            Command::Location { block, content } => {
                assert!(block, "Location:Block should set block=true");
                assert_eq!(content.lines.len(), 1);
                assert_eq!(content.lines[0].content, "fn example() {");
            }
            _ => panic!("Expected Location command"),
        }
    }

    #[test]
    fn test_parse_delete_block() {
        let tokens = vec![
            Token::Open {
                file_path: "./test.rs".to_string(),
                line: LineNumber::new(1),
            },
            Token::Location {
                block: true,
                lines: vec!["fn example() {".to_string()],
                line: LineNumber::new(2),
            },
            Token::Delete {
                block: true,
                lines: vec![],
                line: LineNumber::new(3),
            },
        ];
        let commands = Parser::parse(tokens).unwrap();
        assert_eq!(commands.len(), 3);
        match &commands[2] {
            Command::Delete { block, content } => {
                assert!(block, "Delete:Block should set block=true");
                assert!(content.is_some());
            }
            _ => panic!("Expected Delete command"),
        }
    }

    #[test]
    fn test_parse_delete_block_requires_location_block() {
        // Delete:Block after non-Block Location should fail
        let tokens = vec![
            Token::Open {
                file_path: "./test.rs".to_string(),
                line: LineNumber::new(1),
            },
            Token::Location {
                block: false,
                lines: vec!["fn example() {".to_string()],
                line: LineNumber::new(2),
            },
            Token::Delete {
                block: true,
                lines: vec![],
                line: LineNumber::new(3),
            },
        ];
        let result = Parser::parse(tokens);
        assert!(result.is_err());
        match result.unwrap_err() {
            ParseError::BlockRequiredForDelete { line } => {
                assert_eq!(line, 3);
            }
            _ => panic!("Expected BlockRequiredForDelete error"),
        }
    }

    #[test]
    fn test_parse_location_normal_has_block_false() {
        let tokens = vec![Token::Location {
            block: false,
            lines: vec!["fn main() {".to_string()],
            line: LineNumber::new(1),
        }];
        let commands = Parser::parse(tokens).unwrap();
        match &commands[0] {
            Command::Location { block, .. } => {
                assert!(!block, "Normal Location should have block=false");
            }
            _ => panic!("Expected Location"),
        }
    }
}
