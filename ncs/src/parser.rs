//! 语法分析器 (Parser)
//!
//! 负责将 Lexer 输出的 Token 流组装为 AST（Command 序列）。
//!
//! ## 实现逻辑
//!
//! 1. 消费 Token 流，在 CommandRegistry 中查找命令定义
//! 2. 根据命令的模式注册表匹配模式，解析 args
//! 3. 缺失必要参数 → 报 ParamMissing 错误
//! 4. 多余参数 → 警告但继续执行
//! 5. 构建 Command AST
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §4.2 "语法分析", INSTRUCTION.md §2.4

use crate::error::ParseError;
use crate::lexer::Token;
use crate::model::{DeleteContent, DeleteLine, LocationContent, LocationLine, NewContent, NewLine};
use crate::registry::{normalize_command_name, CommandRegistry, ParamType};
use std::collections::HashMap;

/// 一条完整的命令语句（AST 节点）
#[derive(Debug, PartialEq)]
pub enum Command {
    /// Open 命令：打开目标文件或目录
    Open {
        /// 模式
        mode: OpenMode,
        /// 文件/目录路径
        path: String,
        /// 参数列表
        args: HashMap<String, String>,
    },
    /// Location 命令：定位代码位置
    Location {
        /// 模式
        mode: LocationMode,
        /// 定位内容
        content: Option<LocationContent>,
        /// 参数列表
        args: HashMap<String, String>,
    },
    /// New 命令：插入新内容
    New {
        /// 插入位置
        mode: NewMode,
        /// 待插入的内容
        content: NewContent,
    },
    /// Delete 命令：删除匹配内容
    Delete {
        /// 模式
        mode: DeleteMode,
        /// 用于匹配的删除内容
        content: Option<DeleteContent>,
    },
    /// Raw 命令：字面量内容（会被融入上一个 New/Delete）
    Raw {
        /// 字面量内容
        content: String,
    },
    /// Bash 命令：执行 bash 命令
    Bash {
        /// 要执行的命令字符串
        command: String,
    },
    /// Exec 命令：直连终端执行
    Exec {
        /// 要执行的命令字符串
        command: String,
    },
    /// Read 命令：读取文件内容并显示
    Read {
        /// 模式
        mode: ReadMode,
        /// 文件路径
        path: String,
        /// 参数列表
        args: HashMap<String, String>,
    },
    /// Write 命令：写入文件
    Write {
        /// 写入模式
        mode: WriteMode,
        /// 文件路径
        path: String,
        /// 写入内容
        content: Option<String>,
    },
    /// Include 命令：导入外部命令
    Include {
        /// 外部命令路径
        path: String,
        /// 参数列表
        args: HashMap<String, String>,
    },
    /// WorkPath 命令：设置工作路径
    WorkPath {
        /// 工作路径
        path: String,
    },
    /// Get 命令：从 pools 获取数据
    Get {
        /// pool 键名
        pool_name: String,
        /// 伪装为某个命令
        like: Option<String>,
    },
    /// 外部/动态注册命令（由 Include 导入）
    External {
        /// 命令名
        name: String,
        /// 位置参数列表
        positional_args: Vec<String>,
    },
    /// 关闭符号
    Close {
        /// 关闭的命令名
        name: String,
        /// Capture 管道: @/Open | Capture pool_name
        capture: Option<String>,
    },
    /// Capture 指令：捕获上一个命令的输出到 pools
    Capture {
        /// 存入 pools 的键名
        pool_name: String,
    },
}

impl Command {
    /// 返回命令名（大写）
    pub fn cmd_name(&self) -> String {
        match self {
            Command::Open { .. } => "OPEN".to_string(),
            Command::Location { .. } => "LOCATION".to_string(),
            Command::New { .. } => "NEW".to_string(),
            Command::Delete { .. } => "DELETE".to_string(),
            Command::Raw { .. } => "RAW".to_string(),
            Command::Bash { .. } => "BASH".to_string(),
            Command::Exec { .. } => "EXEC".to_string(),
            Command::Read { .. } => "READ".to_string(),
            Command::Write { .. } => "WRITE".to_string(),
            Command::Include { .. } => "INCLUDE".to_string(),
            Command::WorkPath { .. } => "WORKPATH".to_string(),
            Command::Get { .. } => "GET".to_string(),
            Command::Capture { .. } => "CAPTURE".to_string(),
            Command::External { name, .. } => name.to_uppercase(),
            Command::Close { .. } => "CLOSE".to_string(),
        }
    }

    /// 返回模式名
    pub fn mode_name(&self) -> String {
        match self {
            Command::Open { mode, .. } => format!("{:?}", mode),
            Command::Location { mode, .. } => format!("{:?}", mode),
            Command::New { mode, .. } => format!("{:?}", mode),
            Command::Delete { mode, .. } => format!("{:?}", mode),
            _ => "Normal".to_string(),
        }
    }

    // === Phase 3 三步流水线 ===

    /// 第一阶段：将上游 CmdContent 转换为内部 pipeline 状态
    ///
    /// 对于 New/Delete 命令，解析命令自身的内容为 CmdLine 列表。
    /// 对于 Open/Location，不附加额外数据（execute_core 直接从 self 读取）。
    pub fn convert(
        &self,
        mut input: crate::cmd_content::CmdContent,
    ) -> Result<crate::cmd_content::CmdContent, crate::error::NcsError> {
        match self {
            Command::Open { .. } => Ok(crate::cmd_content::CmdContent::empty()),
            Command::Location { .. } => Ok(input),
            Command::New {
                content: new_content,
                ..
            } => {
                let base_taps = new_content.base_taps;
                let cmd_lines: Vec<crate::cmd_content::CmdLine> = new_content
                    .lines
                    .iter()
                    .map(|nl| {
                        let full = if nl.is_raw {
                            nl.content.clone()
                        } else {
                            let actual_taps = base_taps + nl.diff_taps;
                            format!("{:indent$}{}", "", nl.content, indent = actual_taps)
                        };
                        crate::cmd_content::CmdLine {
                            line_num: 0,
                            content: full,
                        }
                    })
                    .collect();
                input.pending_new_lines = Some(cmd_lines);
                Ok(input)
            }
            Command::Delete { .. } => Ok(input),
            _ => Ok(input),
        }
    }
}

/// Open 命令的模式
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum OpenMode {
    /// 打开单个文本文件
    Normal,
    /// 打开目录，递归扫描
    Dir,
}

/// Location 命令的模式
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum LocationMode {
    /// 基于内容和 diff_taps 匹配
    Normal,
    /// 匹配后调用 BlockParser
    Block,
}

/// New 命令的插入位置
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum NewMode {
    /// 在 Location 匹配位置之后插入
    Normal,
    /// 在文件/Block 开头插入
    Start,
    /// 在文件/Block 末尾插入
    End,
}

/// Delete 命令的模式
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum DeleteMode {
    /// 在 ContentBlock 内匹配并删除连续行
    Normal,
    /// 删除整个 ContentBlock
    Block,
}

/// Read 命令的模式
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum ReadMode {
    /// 读取单个文本文件
    Normal,
    /// 读取目录，列出文件列表
    Dir,
}

/// Write 命令的模式
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum WriteMode {
    /// 块内容写入文件
    Normal,
    /// 从下一行到 EOF 全部原样写入
    Raw,
}

/// 语法分析器
pub struct Parser;

impl Parser {
    /// 将 Token 序列解析为 Command 序列
    ///
    /// # 参数
    ///
    /// * `tokens` — Lexer 输出的 Token 流
    /// * `registry` — 命令注册表，用于模式匹配和参数校验
    ///
    /// # 错误
    ///
    /// 遇到未知命令、缺失参数等情况时返回对应的 ParseError。
    pub fn parse(
        tokens: Vec<Token>,
        registry: &CommandRegistry,
    ) -> Result<Vec<Command>, ParseError> {
        let mut commands: Vec<Command> = Vec::new();

        for token in tokens {
            match token {
                Token::Command {
                    name,
                    mode,
                    args,
                    positional_args,
                    line,
                    content_lines,
                    is_block: _,
                } => {
                    let command = Self::parse_command_token(
                        &name,
                        &mode,
                        &args,
                        &positional_args,
                        line,
                        &content_lines,
                        registry,
                    )?;
                    commands.push(command);
                }
                Token::Close { name, capture, .. } => {
                    commands.push(Command::Close { name, capture });
                }
                Token::Capture { pool_name, .. } => {
                    commands.push(Command::Capture { pool_name });
                }
            }
        }

        // 后处理：将 Raw 命令融入前一个 New/Delete
        let commands = Self::merge_raw_commands(commands);

        Ok(commands)
    }

    /// 解析单个命令 Token 为 Command
    fn parse_command_token(
        cmd_name: &str,
        mode_str: &str,
        args: &HashMap<String, String>,
        positional_args: &[String],
        line: crate::model::LineNumber,
        content_lines: &[String],
        registry: &CommandRegistry,
    ) -> Result<Command, ParseError> {
        let normalized_name = normalize_command_name(cmd_name);
        let _ = (args, registry); // registry used in some branches

        match normalized_name.as_str() {
            "OPEN" => Self::parse_open(mode_str, args, positional_args, line),
            "LOCATION" => {
                Self::parse_location(mode_str, args, positional_args, line, content_lines)
            }
            "NEW" => Self::parse_new(mode_str, args, line, content_lines),
            "DELETE" => Self::parse_delete(mode_str, args, line, content_lines),
            "RAW" => {
                let content = positional_args.join(" ");
                Ok(Command::Raw { content })
            }
            "BASH" => Ok(Command::Bash {
                command: positional_args.join(" "),
            }),
            "EXEC" => Ok(Command::Exec {
                command: positional_args.join(" "),
            }),
            "READ" => Self::parse_read(mode_str, args, positional_args, line),
            "WRITE" => Self::parse_write(mode_str, args, positional_args, line, content_lines),
            "INCLUDE" => Self::parse_include(args, positional_args, line),
            "WORKPATH" => {
                let path = positional_args.first().cloned().unwrap_or_default();
                Ok(Command::WorkPath { path })
            }
            "GET" => Self::parse_get(args, positional_args, line),
            _ => Ok(Command::External {
                name: cmd_name.to_string(),
                positional_args: positional_args.to_vec(),
            }),
        }
    }

    /// 解析 Open 命令
    fn parse_open(
        mode_str: &str,
        args: &HashMap<String, String>,
        positional_args: &[String],
        line: crate::model::LineNumber,
    ) -> Result<Command, ParseError> {
        let path = positional_args.first().cloned().unwrap_or_default();

        let mode = if mode_str.is_empty() {
            // 无显式模式时，根据路径自动检测
            auto_detect_open_mode(&path)
        } else {
            match normalize_command_name(mode_str).as_str() {
                "NORMAL" => OpenMode::Normal,
                "DIR" => OpenMode::Dir,
                _ => OpenMode::Normal,
            }
        };

        // 校验必要参数
        Self::validate_params(
            "Open",
            mode_str,
            args,
            line,
            &[
                ("start", ParamType::Number, false),
                ("end", ParamType::Number, false),
                ("depth", ParamType::Number, false),
                ("ignore", ParamType::String, false),
                ("filter", ParamType::String, false),
            ],
        )?;

        Ok(Command::Open {
            mode,
            path,
            args: args.clone(),
        })
    }

    /// 解析 Location 命令
    fn parse_location(
        mode_str: &str,
        _args: &HashMap<String, String>,
        _positional_args: &[String],
        _line: crate::model::LineNumber,
        content_lines: &[String],
    ) -> Result<Command, ParseError> {
        let mode = resolve_mode(
            mode_str,
            &["NORMAL", "BLOCK"],
            LocationMode::Normal,
            |m| match m {
                "NORMAL" => LocationMode::Normal,
                "BLOCK" => LocationMode::Block,
                _ => LocationMode::Normal,
            },
        );

        let content = if content_lines.is_empty() {
            None
        } else {
            Some(parse_location_content(content_lines))
        };

        Ok(Command::Location {
            mode,
            content,
            args: HashMap::new(),
        })
    }

    /// 解析 New 命令
    fn parse_new(
        mode_str: &str,
        _args: &HashMap<String, String>,
        _line: crate::model::LineNumber,
        content_lines: &[String],
    ) -> Result<Command, ParseError> {
        let mode = resolve_mode(
            mode_str,
            &["NORMAL", "START", "END"],
            NewMode::Normal,
            |m| match m {
                "NORMAL" => NewMode::Normal,
                "START" => NewMode::Start,
                "END" => NewMode::End,
                _ => NewMode::Normal,
            },
        );

        let content = parse_new_content(content_lines);

        Ok(Command::New { mode, content })
    }

    /// 解析 Delete 命令
    fn parse_delete(
        mode_str: &str,
        _args: &HashMap<String, String>,
        _line: crate::model::LineNumber,
        content_lines: &[String],
    ) -> Result<Command, ParseError> {
        let mode = resolve_mode(
            mode_str,
            &["NORMAL", "BLOCK"],
            DeleteMode::Normal,
            |m| match m {
                "NORMAL" => DeleteMode::Normal,
                "BLOCK" => DeleteMode::Block,
                _ => DeleteMode::Normal,
            },
        );

        let content = if content_lines.is_empty() {
            None
        } else {
            Some(parse_delete_content(content_lines))
        };

        Ok(Command::Delete { mode, content })
    }

    /// 解析 Read 命令
    fn parse_read(
        mode_str: &str,
        args: &HashMap<String, String>,
        positional_args: &[String],
        line: crate::model::LineNumber,
    ) -> Result<Command, ParseError> {
        let path = positional_args.first().cloned().unwrap_or_default();

        let mode = if mode_str.is_empty() {
            // 无显式模式时，根据路径自动检测
            auto_detect_read_mode(&path)
        } else {
            match normalize_command_name(mode_str).as_str() {
                "NORMAL" => ReadMode::Normal,
                "DIR" => ReadMode::Dir,
                _ => ReadMode::Normal,
            }
        };

        // Read 的模式和参数与 Open 一致
        Self::validate_params(
            "Read",
            mode_str,
            args,
            line,
            &[
                ("depth", ParamType::Number, false),
                ("ignore", ParamType::String, false),
                ("filter", ParamType::String, false),
            ],
        )?;
        Ok(Command::Read {
            mode,
            path,
            args: args.clone(),
        })
    }

    /// 解析 Write 命令
    fn parse_write(
        mode_str: &str,
        _args: &HashMap<String, String>,
        positional_args: &[String],
        _line: crate::model::LineNumber,
        content_lines: &[String],
    ) -> Result<Command, ParseError> {
        let mode = resolve_mode(
            mode_str,
            &["NORMAL", "RAW"],
            WriteMode::Normal,
            |m| match m {
                "NORMAL" => WriteMode::Normal,
                "RAW" => WriteMode::Raw,
                _ => WriteMode::Normal,
            },
        );

        let path = positional_args.first().cloned().unwrap_or_default();

        let content = if content_lines.is_empty() {
            None
        } else {
            Some(content_lines.join("\n"))
        };

        Ok(Command::Write {
            mode,
            path,
            content,
        })
    }

    /// 解析 Include 命令
    fn parse_include(
        args: &HashMap<String, String>,
        positional_args: &[String],
        line: crate::model::LineNumber,
    ) -> Result<Command, ParseError> {
        // 所有位置参数拼接为外部命令的完整执行指令
        let path = positional_args.join(" ");

        // 校验 alias 是必要参数
        if let Some(alias) = args.get("alias") {
            if alias.is_empty() {
                return Err(ParseError::ParamMissing {
                    cmd_name: "Include".to_string(),
                    mode_name: "Normal".to_string(),
                    param_name: "alias".to_string(),
                    line,
                });
            }
        } else {
            return Err(ParseError::ParamMissing {
                cmd_name: "Include".to_string(),
                mode_name: "Normal".to_string(),
                param_name: "alias".to_string(),
                line,
            });
        }

        Ok(Command::Include {
            path,
            args: args.clone(),
        })
    }

    /// 解析 Get 命令
    fn parse_get(
        args: &HashMap<String, String>,
        positional_args: &[String],
        _line: crate::model::LineNumber,
    ) -> Result<Command, ParseError> {
        let pool_name = positional_args.first().cloned().unwrap_or_default();
        let like = args.get("like").cloned();

        Ok(Command::Get { pool_name, like })
    }

    /// 校验参数是否满足要求
    ///
    /// 检查 args 中是否有所有必填的 param_defs，缺失时报 ParamMissing。
    fn validate_params(
        cmd_name: &str,
        mode_name: &str,
        args: &HashMap<String, String>,
        line: crate::model::LineNumber,
        param_defs: &[(&str, ParamType, bool)],
    ) -> Result<(), ParseError> {
        for (param_name, _param_type, required) in param_defs {
            if *required && !args.contains_key(*param_name) {
                return Err(ParseError::ParamMissing {
                    cmd_name: cmd_name.to_string(),
                    mode_name: mode_name.to_string(),
                    param_name: param_name.to_string(),
                    line,
                });
            }
        }
        Ok(())
    }

    /// 后处理：将 Raw 命令融入前一个 New/Delete
    ///
    /// `!@Raw` Token 的内容被融入上一个 New 或 Delete 命令的 ContentLines，
    /// 对应行标记 `is_raw = true`。
    fn merge_raw_commands(commands: Vec<Command>) -> Vec<Command> {
        let mut result: Vec<Command> = Vec::new();

        for command in commands {
            match command {
                Command::Raw { content } => {
                    let mut merged = false;
                    for last in result.iter_mut().rev() {
                        match last {
                            Command::New {
                                content: new_content,
                                ..
                            } => {
                                new_content.lines.push(NewLine {
                                    diff_taps: 0,
                                    content: content.clone(),
                                    is_raw: true,
                                });
                                merged = true;
                                break;
                            }
                            Command::Delete {
                                content: Some(delete_content),
                                ..
                            } => {
                                delete_content.lines.push(DeleteLine {
                                    content: content.clone(),
                                    is_raw: true,
                                });
                                merged = true;
                                break;
                            }
                            Command::Close { .. } => continue,
                            _ => break,
                        }
                    }
                    if !merged {
                        result.push(Command::Raw { content });
                    }
                }
                other => result.push(other),
            }
        }

        result
    }
}

/// 解析模式字符串为对应的枚举值
fn resolve_mode<T: Copy>(
    mode_str: &str,
    valid_modes: &[&str],
    default: T,
    mapping: impl Fn(&str) -> T,
) -> T {
    if mode_str.is_empty() {
        return default;
    }
    let normalized = normalize_command_name(mode_str);
    for valid in valid_modes {
        if normalize_command_name(valid) == normalized {
            return mapping(valid);
        }
    }
    default
}

/// 根据路径自动检测 Open 模式（文件 → Normal，目录 → Dir）
fn auto_detect_open_mode(path: &str) -> OpenMode {
    let p = std::path::Path::new(path);
    // 先检查原始路径，再尝试相对于 ncs crate 目录
    if p.is_dir() {
        return OpenMode::Dir;
    }
    let manifest_relative = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    if manifest_relative.is_dir() {
        return OpenMode::Dir;
    }
    OpenMode::Normal
}

/// 根据路径自动检测 Read 模式（文件 → Normal，目录 → Dir）
fn auto_detect_read_mode(path: &str) -> ReadMode {
    let p = std::path::Path::new(path);
    if p.is_dir() {
        return ReadMode::Dir;
    }
    let manifest_relative = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    if manifest_relative.is_dir() {
        return ReadMode::Dir;
    }
    ReadMode::Normal
}

/// 从 content_lines 解析 LocationContent
fn parse_location_content(content_lines: &[String]) -> LocationContent {
    let base_taps = content_lines
        .first()
        .map(|first| count_leading_spaces(first))
        .unwrap_or(0);

    let lines: Vec<LocationLine> = content_lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let taps = count_leading_spaces(line);
            let diff_taps = if index == 0 {
                Some(0)
            } else {
                Some(taps.saturating_sub(base_taps))
            };
            LocationLine {
                index,
                diff_taps,
                content: line.clone(),
                line_num: None,
            }
        })
        .collect();

    LocationContent { lines }
}

/// 从 content_lines 解析 NewContent
///
/// 若行前缀为 `!@Raw `，则标记为 is_raw 并去除前缀。
fn parse_new_content(content_lines: &[String]) -> NewContent {
    let base_taps = content_lines
        .first()
        .map(|first| count_leading_spaces(first))
        .unwrap_or(0);

    let lines: Vec<NewLine> = content_lines
        .iter()
        .map(|line| {
            if let Some(rest) = line.strip_prefix("!@Raw ") {
                // Raw 行：标记 is_raw，去除前缀
                NewLine {
                    diff_taps: 0,
                    content: rest.to_string(),
                    is_raw: true,
                }
            } else if let Some(rest) = line.strip_prefix("!@Get ") {
                // Get 行：作为普通行但标记（Phase 2+ 由 Engine 展开）
                NewLine {
                    diff_taps: 0,
                    content: rest.to_string(),
                    is_raw: false,
                }
            } else {
                let taps = count_leading_spaces(line);
                let content = line[taps..].to_string();
                let diff_taps = taps.saturating_sub(base_taps);
                NewLine {
                    diff_taps,
                    content,
                    is_raw: false,
                }
            }
        })
        .collect();

    NewContent { lines, base_taps }
}

/// 从 content_lines 解析 DeleteContent
///
/// 若行前缀为 `!@Raw `，则标记为 is_raw 并去除前缀。
fn parse_delete_content(content_lines: &[String]) -> DeleteContent {
    let lines: Vec<DeleteLine> = content_lines
        .iter()
        .map(|line| {
            if let Some(rest) = line.strip_prefix("!@Raw ") {
                DeleteLine {
                    content: rest.to_string(),
                    is_raw: true,
                }
            } else {
                DeleteLine {
                    content: line.clone(),
                    is_raw: false,
                }
            }
        })
        .collect();

    DeleteContent { lines }
}

/// 计算行首的 ASCII 空格数量
fn count_leading_spaces(line: &str) -> usize {
    line.chars().take_while(|c| *c == ' ').count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::registry::CommandRegistry;

    fn test_registry() -> CommandRegistry {
        CommandRegistry::init()
    }

    /// 辅助函数：lex + parse 一步完成
    fn lex_and_parse(script: &str) -> Result<Vec<Command>, ParseError> {
        let registry = test_registry();
        let tokens = Lexer::tokenize(script, &registry)?;
        Parser::parse(tokens, &registry)
    }

    // ============================================================
    // 空输入
    // ============================================================

    #[test]
    fn test_empty_tokens_returns_empty_commands() {
        let commands = lex_and_parse("").unwrap();
        assert!(commands.is_empty());
    }

    // ============================================================
    // Open 命令
    // ============================================================

    #[test]
    fn test_parse_open_normal() {
        let commands = lex_and_parse("!@Open ./test.rs").unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0],
            Command::Open {
                mode: OpenMode::Normal,
                path: "./test.rs".to_string(),
                args: HashMap::new(),
            }
        );
    }

    #[test]
    fn test_parse_open_dir() {
        let commands = lex_and_parse("!@Open Dir ./mydir depth=5").unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            Command::Open { mode, path, args } => {
                assert_eq!(*mode, OpenMode::Dir);
                assert_eq!(path, "./mydir");
                assert_eq!(args.get("depth"), Some(&"5".to_string()));
            }
            _ => panic!("Expected Open command"),
        }
    }

    #[test]
    fn test_parse_open_with_kv_only() {
        let commands = lex_and_parse("!@Open start=10 ./test.rs").unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            Command::Open { mode, path, args } => {
                assert_eq!(*mode, OpenMode::Normal);
                assert_eq!(path, "./test.rs");
                assert_eq!(args.get("start"), Some(&"10".to_string()));
            }
            _ => panic!("Expected Open command"),
        }
    }

    // ============================================================
    // Close 命令
    // ============================================================

    #[test]
    fn test_parse_close() {
        let commands = lex_and_parse("@/Open").unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0],
            Command::Close {
                capture: None,
                name: "Open".to_string()
            }
        );
    }

    #[test]
    fn test_parse_multiple_closes() {
        let commands = lex_and_parse("@/Location\n@/Open").unwrap();
        assert_eq!(commands.len(), 2);
        assert_eq!(
            commands[0],
            Command::Close {
                capture: None,
                name: "Location".to_string()
            }
        );
        assert_eq!(
            commands[1],
            Command::Close {
                capture: None,
                name: "Open".to_string()
            }
        );
    }

    // ============================================================
    // Location 命令
    // ============================================================

    #[test]
    fn test_parse_location_normal() {
        let commands = lex_and_parse("!@Location\nfn main() {\n  x();\n}\n@/Location").unwrap();
        assert_eq!(commands.len(), 2);
        match &commands[0] {
            Command::Location { mode, content, .. } => {
                assert_eq!(*mode, LocationMode::Normal);
                let loc = content.as_ref().unwrap();
                assert_eq!(loc.lines.len(), 3);
                assert_eq!(loc.lines[0].content, "fn main() {");
                assert_eq!(loc.lines[0].diff_taps, Some(0));
            }
            _ => panic!("Expected Location command"),
        }
        assert_eq!(
            commands[1],
            Command::Close {
                capture: None,
                name: "Location".to_string()
            }
        );
    }

    #[test]
    fn test_parse_location_block() {
        let commands = lex_and_parse("!@Location Block\nfn example() {\n@/Location").unwrap();
        assert_eq!(commands.len(), 2);
        match &commands[0] {
            Command::Location { mode, content, .. } => {
                assert_eq!(*mode, LocationMode::Block);
                assert!(content.is_some());
            }
            _ => panic!("Expected Location command"),
        }
    }

    #[test]
    fn test_parse_location_without_content() {
        let commands = lex_and_parse("!@Location Block").unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            Command::Location { mode, content, .. } => {
                assert_eq!(*mode, LocationMode::Block);
                assert!(content.is_none());
            }
            _ => panic!("Expected Location command"),
        }
    }

    // ============================================================
    // New 命令
    // ============================================================

    #[test]
    fn test_parse_new_normal() {
        let commands = lex_and_parse("!@New\n  let x = 1;\n  let y = 2;\n@/New").unwrap();
        assert_eq!(commands.len(), 2);
        match &commands[0] {
            Command::New { mode, content } => {
                assert_eq!(*mode, NewMode::Normal);
                assert_eq!(content.lines.len(), 2);
                assert_eq!(content.lines[0].content, "let x = 1;");
                assert_eq!(content.lines[0].diff_taps, 0);
                assert_eq!(content.lines[1].diff_taps, 0);
            }
            _ => panic!("Expected New command"),
        }
    }

    #[test]
    fn test_parse_new_start() {
        let commands = lex_and_parse("!@New Start\n  header();\n@/New").unwrap();
        assert_eq!(commands.len(), 2);
        match &commands[0] {
            Command::New { mode, content } => {
                assert_eq!(*mode, NewMode::Start);
                assert_eq!(content.lines.len(), 1);
            }
            _ => panic!("Expected New command"),
        }
    }

    #[test]
    fn test_parse_new_end() {
        let commands = lex_and_parse("!@New End\n  footer();\n@/New").unwrap();
        assert_eq!(commands.len(), 2);
        match &commands[0] {
            Command::New { mode, .. } => {
                assert_eq!(*mode, NewMode::End);
            }
            _ => panic!("Expected New command"),
        }
    }

    #[test]
    fn test_parse_new_with_raw_line() {
        let commands =
            lex_and_parse("!@New\n  normal line\n!@Raw raw content\n  another line\n@/New")
                .unwrap();
        assert_eq!(commands.len(), 2);
        match &commands[0] {
            Command::New { content, .. } => {
                assert_eq!(content.lines.len(), 3);
                assert_eq!(content.lines[0].content, "normal line");
                assert!(!content.lines[0].is_raw);
                assert_eq!(content.lines[1].content, "raw content");
                assert!(content.lines[1].is_raw);
                assert_eq!(content.lines[2].content, "another line");
                assert!(!content.lines[2].is_raw);
            }
            _ => panic!("Expected New command"),
        }
    }

    // ============================================================
    // Delete 命令
    // ============================================================

    #[test]
    fn test_parse_delete_normal() {
        let commands = lex_and_parse("!@Delete\nold_code();\n@/Delete").unwrap();
        assert_eq!(commands.len(), 2);
        match &commands[0] {
            Command::Delete { mode, content } => {
                assert_eq!(*mode, DeleteMode::Normal);
                let del = content.as_ref().unwrap();
                assert_eq!(del.lines.len(), 1);
                assert_eq!(del.lines[0].content, "old_code();");
            }
            _ => panic!("Expected Delete command"),
        }
    }

    #[test]
    fn test_parse_delete_block() {
        let commands = lex_and_parse("!@Delete Block\nfn dead_code() {\n}\n@/Delete").unwrap();
        assert_eq!(commands.len(), 2);
        match &commands[0] {
            Command::Delete { mode, .. } => {
                assert_eq!(*mode, DeleteMode::Block);
            }
            _ => panic!("Expected Delete command"),
        }
    }

    #[test]
    fn test_parse_delete_empty_content() {
        let commands = lex_and_parse("!@Delete Block").unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            Command::Delete { mode, content } => {
                assert_eq!(*mode, DeleteMode::Block);
                assert!(content.is_none());
            }
            _ => panic!("Expected Delete command"),
        }
    }

    // ============================================================
    // Bash / Exec / Read / Write
    // ============================================================

    #[test]
    fn test_parse_bash() {
        let commands = lex_and_parse("!@Bash echo hello world").unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0],
            Command::Bash {
                command: "echo hello world".to_string()
            }
        );
    }

    #[test]
    fn test_parse_exec() {
        let commands = lex_and_parse("!@Exec cargo build").unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0],
            Command::Exec {
                command: "cargo build".to_string()
            }
        );
    }

    #[test]
    fn test_parse_read() {
        let commands = lex_and_parse("!@Read ./test.rs").unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            Command::Read { mode, path, args } => {
                assert_eq!(*mode, ReadMode::Normal);
                assert_eq!(path, "./test.rs");
                assert!(args.is_empty());
            }
            _ => panic!("Expected Read command"),
        }
    }

    #[test]
    fn test_parse_read_auto_detect_dir_mode() {
        // 不指定模式时，自动检测路径类型
        // 路径相对于 ncs crate 根目录 (CARGO_MANIFEST_DIR)
        let commands = lex_and_parse("!@Read tests/data").unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            Command::Read { mode, path, .. } => {
                assert_eq!(*mode, ReadMode::Dir, "应该自动检测为 Dir 模式");
                assert_eq!(path, "tests/data");
            }
            _ => panic!("Expected Read command"),
        }
    }

    #[test]
    fn test_parse_read_auto_detect_normal_mode() {
        let commands = lex_and_parse("!@Read tests/data/plain.txt").unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            Command::Read { mode, path, .. } => {
                assert_eq!(*mode, ReadMode::Normal, "应该自动检测为 Normal 模式");
                assert_eq!(path, "tests/data/plain.txt");
            }
            _ => panic!("Expected Read command"),
        }
    }

    #[test]
    fn test_parse_read_explicit_dir_mode_overrides_auto_detect() {
        let commands = lex_and_parse("!@Read Dir tests/data/plain.txt").unwrap();
        match &commands[0] {
            Command::Read { mode, .. } => {
                assert_eq!(*mode, ReadMode::Dir, "显式 Dir 模式应保留");
            }
            _ => panic!("Expected Read command"),
        }
    }

    #[test]
    fn test_parse_open_auto_detect_dir_mode() {
        let commands = lex_and_parse("!@Open tests/data").unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            Command::Open { mode, path, .. } => {
                assert_eq!(*mode, OpenMode::Dir, "应该自动检测为 Dir 模式");
                assert_eq!(path, "tests/data");
            }
            _ => panic!("Expected Open command"),
        }
    }

    #[test]
    fn test_parse_open_auto_detect_normal_mode() {
        let commands = lex_and_parse("!@Open tests/data/plain.txt").unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            Command::Open { mode, path, .. } => {
                assert_eq!(*mode, OpenMode::Normal, "应该自动检测为 Normal 模式");
                assert_eq!(path, "tests/data/plain.txt");
            }
            _ => panic!("Expected Open command"),
        }
    }

    #[test]
    fn test_parse_open_explicit_normal_overrides_auto_detect() {
        let commands = lex_and_parse("!@Open Normal tests/data").unwrap();
        match &commands[0] {
            Command::Open { mode, .. } => {
                assert_eq!(*mode, OpenMode::Normal, "显式 Normal 模式应保留");
            }
            _ => panic!("Expected Open command"),
        }
    }

    #[test]
    fn test_parse_write_normal() {
        let commands =
            lex_and_parse("!@Write ./output.rs\ncontent line 1\ncontent line 2\n@/Write").unwrap();
        assert_eq!(commands.len(), 2);
        match &commands[0] {
            Command::Write {
                mode,
                path,
                content,
            } => {
                assert_eq!(*mode, WriteMode::Normal);
                assert_eq!(path, "./output.rs");
                assert_eq!(content, &Some("content line 1\ncontent line 2".to_string()));
            }
            _ => panic!("Expected Write command"),
        }
    }

    #[test]
    fn test_parse_write_raw() {
        // Write Raw 提取从下一行到 EOF 的全部内容，
        // 包括 @/Write、!@Open 等原本会触发终止的标记，
        // 全部作为原始文本保存
        let commands = lex_and_parse(
            "!@Write Raw ./output.ncs\neverything here\nis raw\n@/Write\n!@New\nmore stuff",
        )
        .unwrap();
        assert_eq!(
            commands.len(),
            1,
            "Write Raw produces only 1 command (no Close)"
        );
        match &commands[0] {
            Command::Write {
                mode,
                path,
                content,
                ..
            } => {
                assert_eq!(*mode, WriteMode::Raw);
                assert_eq!(path, "./output.ncs");
                let c = content.as_ref().expect("Write Raw should have content");
                assert!(c.contains("everything here"));
                assert!(c.contains("is raw"));
                assert!(c.contains("@/Write"));
                assert!(c.contains("!@New"));
                assert!(c.contains("more stuff"));
            }
            _ => panic!("Expected Write command"),
        }
    }

    // ============================================================
    // WorkPath / Include / Get
    // ============================================================

    #[test]
    fn test_parse_work_path() {
        let commands = lex_and_parse("!@WorkPath /tmp/work").unwrap();
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0],
            Command::WorkPath {
                path: "/tmp/work".to_string()
            }
        );
    }

    #[test]
    fn test_parse_include() {
        let commands = lex_and_parse("!@Include /usr/bin/mytool alias=MyTool block=true").unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            Command::Include { path, args } => {
                assert_eq!(path, "/usr/bin/mytool");
                assert_eq!(args.get("alias"), Some(&"MyTool".to_string()));
                assert_eq!(args.get("block"), Some(&"true".to_string()));
            }
            _ => panic!("Expected Include command"),
        }
    }

    #[test]
    fn test_parse_include_missing_alias_error() {
        let result = lex_and_parse("!@Include /usr/bin/mytool");
        assert!(result.is_err());
        match result.unwrap_err() {
            ParseError::ParamMissing {
                cmd_name,
                param_name,
                ..
            } => {
                assert_eq!(cmd_name, "Include");
                assert_eq!(param_name, "alias");
            }
            _ => panic!("Expected ParamMissing error"),
        }
    }

    #[test]
    fn test_parse_get() {
        let commands = lex_and_parse("!@Get my_pool like=Bash").unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            Command::Get { pool_name, like } => {
                assert_eq!(pool_name, "my_pool");
                assert_eq!(like, &Some("Bash".to_string()));
            }
            _ => panic!("Expected Get command"),
        }
    }

    #[test]
    fn test_parse_get_without_like() {
        let commands = lex_and_parse("!@Get my_pool").unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            Command::Get { pool_name, like } => {
                assert_eq!(pool_name, "my_pool");
                assert_eq!(like, &None);
            }
            _ => panic!("Expected Get command"),
        }
    }

    // ============================================================
    // Raw 融入 New/Delete
    // ============================================================

    #[test]
    fn test_standalone_raw_merged_into_previous_new() {
        let commands = lex_and_parse("!@New\n  code();\n@/New\n!@Raw extra raw content").unwrap();
        assert_eq!(commands.len(), 2);
        match &commands[0] {
            Command::New { content, .. } => {
                assert_eq!(content.lines.len(), 2);
                // First line from !@New content
                assert!(!content.lines[0].is_raw);
                // Second line merged from !@Raw
                assert_eq!(content.lines[1].content, "extra raw content");
                assert!(content.lines[1].is_raw);
            }
            _ => panic!("Expected New command (first)"),
        }
        // Close token
        assert_eq!(
            commands[1],
            Command::Close {
                capture: None,
                name: "New".to_string()
            }
        );
    }

    // ============================================================
    // 未知命令
    // ============================================================

    #[test]
    fn test_unknown_command_becomes_external() {
        // 未知命令不再报错，而是转为 Command::External
        let commands = lex_and_parse("!@UnknownCmd foo bar").unwrap();
        assert_eq!(commands.len(), 1);
        match &commands[0] {
            Command::External {
                name,
                positional_args,
            } => {
                assert_eq!(name, "UnknownCmd");
                assert_eq!(positional_args, &vec!["foo".to_string(), "bar".to_string()]);
            }
            _ => panic!("Expected External command"),
        }
    }

    // ============================================================
    // 完整脚本
    // ============================================================

    #[test]
    fn test_full_script_open_location_new_close() {
        let script = "!@Open ./test.rs\n!@Location\nfn main() {}\n@/Location\n!@New\n  extra();\n@/New\n@/Open";
        let commands = lex_and_parse(script).unwrap();
        assert_eq!(commands.len(), 6);
        // Open
        assert!(matches!(commands[0], Command::Open { .. }));
        // Location
        assert!(matches!(commands[1], Command::Location { .. }));
        // Close Location
        assert_eq!(
            commands[2],
            Command::Close {
                capture: None,
                name: "Location".to_string()
            }
        );
        // New
        assert!(matches!(commands[3], Command::New { .. }));
        // Close New
        assert_eq!(
            commands[4],
            Command::Close {
                capture: None,
                name: "New".to_string()
            }
        );
        // Close Open
        assert_eq!(
            commands[5],
            Command::Close {
                capture: None,
                name: "Open".to_string()
            }
        );
    }

    // ============================================================
    // Phase 3: convert/execute_core/out 三步流水线
    // ============================================================

    use crate::cmd_content::CmdContent;

    fn make_empty_content() -> CmdContent {
        CmdContent::empty()
    }

    #[test]
    fn test_convert_open_returns_empty() {
        let cmd = Command::Open {
            mode: OpenMode::Normal,
            path: "./test.rs".to_string(),
            args: HashMap::new(),
        };
        let input = make_empty_content();
        let result = cmd.convert(input).unwrap();
        assert!(result.lines.is_empty());
        assert!(result.raw_content.is_empty());
    }

    #[test]
    fn test_convert_location_passes_input() {
        let cmd = Command::Location {
            mode: LocationMode::Normal,
            content: Some(LocationContent {
                lines: vec![LocationLine {
                    index: 0,
                    diff_taps: Some(0),
                    content: "fn main() {".to_string(),
                    line_num: None,
                }],
            }),
            args: HashMap::new(),
        };
        let input = make_empty_content();
        let result = cmd.convert(input).unwrap();
        assert!(result.lines.is_empty());
    }

    #[test]
    fn test_convert_new_stores_new_lines() {
        let cmd = Command::New {
            mode: NewMode::Normal,
            content: NewContent {
                base_taps: 0,
                lines: vec![NewLine {
                    diff_taps: 0,
                    content: "new_code();".to_string(),
                    is_raw: false,
                }],
            },
        };
        let input = make_empty_content();
        let result = cmd.convert(input).unwrap();
        assert!(result.pending_new_lines.is_some());
        let lines = result.pending_new_lines.unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].content, "new_code();");
    }

    #[test]
    fn test_convert_delete_passes_through() {
        let cmd = Command::Delete {
            mode: DeleteMode::Normal,
            content: Some(DeleteContent {
                lines: vec![DeleteLine {
                    content: "old_code();".to_string(),
                    is_raw: false,
                }],
            }),
        };
        let input = make_empty_content();
        let result = cmd.convert(input).unwrap();
        assert!(result.lines.is_empty());
    }

    #[test]
    fn test_convert_raw_passes_through() {
        let cmd = Command::Raw {
            content: "raw content".to_string(),
        };
        let mut input = CmdContent::empty();
        input.raw_content = "upstream".to_string();
        let result = cmd.convert(input).unwrap();
        assert_eq!(result.raw_content, "upstream");
    }
}
