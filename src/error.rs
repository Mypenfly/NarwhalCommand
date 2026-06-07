//! 错误类型定义 (Error Types)
//!
//! 集中管理项目中所有错误类型。
//! 每个错误类型实现 Display + Error trait，
//! 并附带上下文信息用于构造用户友好的错误提示。
//!
//! ## 对应文档
//!
//! 详见 INSTRUCTION.md 第 5 节 "错误信息规范"

use std::error::Error;
use std::fmt;

/// 项目的根错误类型
///
/// 包含所有可能的错误变体，统一对外暴露。
#[derive(Debug)]
pub enum NEditError {
    /// 匹配相关错误
    Match(MatchError),
    /// 解析相关错误
    #[allow(dead_code)]
    Parse(ParseError),
    /// 文件 I/O 相关错误
    File(FileError),
    /// 引擎执行错误
    Engine(EngineError),
}

/// 匹配相关的错误
#[derive(Debug)]
pub enum MatchError {
    /// 未找到任何匹配
    NoMatch {
        /// 用于匹配的定位内容
        location_content: String,
    },
    /// 找到过多匹配
    TooManyMatches {
        /// 匹配到的候选数量
        count: usize,
        /// 候选列表（最多保留 3 个）
        candidates: Vec<String>,
        /// 用于匹配的定位内容
        location_content: String,
    },
    /// Delete 匹配失败
    DeleteMatchFailed {
        /// 被删除内容的首行
        delete_content: String,
        /// 所在的 ContentBlock 内容摘要
        block_snippet: String,
    },
    /// Delete 匹配位置与 Location 不紧邻（中间有未经定位的行）
    DeleteNotAdjacent {
        /// Location 最后一行
        location_last_line: String,
        /// Delete 首行
        delete_first_line: String,
        /// 中间隔了多少行
        gap_lines: usize,
    },
}

/// 命令解析错误
#[derive(Debug)]
pub enum ParseError {
    /// Open 命令缺少文件路径
    MissingFilePath,
    /// 无法识别的命令
    UnknownCommand {
        /// 无法识别的 Token 文本
        token: String,
        /// 所在行号
        line: usize,
    },
    /// New/Delete 命令前缺少 Location（或前一个 Token 是 `...` 产生歧义）
    MissingLocation {
        /// 命令类型（"New" / "Delete"）
        command: String,
        /// 所在行号
        line: usize,
    },
    /// 意外的分隔符
    #[allow(dead_code)]
    UnexpectedSeparator {
        /// 所在行号
        line: usize,
    },
}

/// 文件 I/O 相关错误
#[derive(Debug)]
pub enum FileError {
    /// 文件未找到
    NotFound {
        /// 文件路径
        path: String,
    },
    /// 无法打开文件
    CannotOpen {
        /// 文件路径
        path: String,
        /// 失败原因
        reason: String,
    },
    /// 写入失败
    WriteFailed {
        /// 文件路径
        path: String,
        /// 失败原因
        reason: String,
    },
}

/// 引擎执行错误
#[derive(Debug)]
pub enum EngineError {
    /// 执行 Open 命令时缺少前置 Location
    MissingLocationForNew,
    /// 执行 Delete:Block 时前一个 Location 未使用 Block
    #[allow(dead_code)]
    BlockRequiredForDelete,
    /// Block 栈为空时尝试弹出
    BlockStackEmpty,
    /// 隐式 Off 失败
    #[allow(dead_code)]
    ImplicitOffFailed {
        /// 失败原因
        reason: String,
    },
}

impl fmt::Display for MatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MatchError::NoMatch { location_content } => {
                write!(
                    f,
                    "Location 命令未找到任何匹配，请检查定位内容：\n{}",
                    location_content
                )
            }
            MatchError::TooManyMatches {
                count,
                candidates,
                location_content,
            } => {
                write!(
                    f,
                    "Location 命令匹配到 {} 个结果（期望 1 个）\n\
                     Location 内容:\n{}\n\
                     匹配候选:\n{}",
                    count,
                    location_content,
                    candidates.join("\n")
                )
            }
            MatchError::DeleteMatchFailed {
                delete_content,
                block_snippet,
            } => {
                write!(
                    f,
                    "Delete 命令未能在当前 Block 中找到匹配内容:\n\
                     删除内容首行: {}\n\
                     Block 内容:\n{}",
                    delete_content, block_snippet
                )
            }
            MatchError::DeleteNotAdjacent {
                location_last_line,
                delete_first_line,
                gap_lines,
            } => {
                write!(
                    f,
                    "Delete 匹配位置与 Location 不紧邻（中间隔了 {} 行未经定位的内容），\n\
                     这意味着 Delete 可能删除了错误位置的内容。\n\
                     Location 最后一行: {}\n\
                     Delete 首行: {}\n\
                     建议：在 Delete 之前使用嵌套 Location 精确定位到要删除的内容",
                    gap_lines, location_last_line, delete_first_line
                )
            }
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::MissingFilePath => {
                write!(f, "Open 命令缺少文件路径参数")
            }
            ParseError::UnknownCommand { token, line } => {
                write!(
                    f,
                    "第 {} 行出现无法识别的命令: {}",
                    line, token
                )
            }
            ParseError::MissingLocation { command, line } => {
                write!(
                    f,
                    "第 {} 行: {} 命令前缺少 Location 定位（或 `...` 分隔符导致插入位置不明确），\n\
                     请在 {} 之前使用 Location 明确指定操作位置",
                    line, command, command
                )
            }
            ParseError::UnexpectedSeparator { line } => {
                write!(f, "第 {} 行出现意外的分隔符 ...", line)
            }
        }
    }
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileError::NotFound { path } => {
                write!(f, "文件未找到: {}", path)
            }
            FileError::CannotOpen { path, reason } => {
                write!(f, "无法打开文件 {}: {}", path, reason)
            }
            FileError::WriteFailed { path, reason } => {
                write!(f, "写入文件 {} 失败: {}", path, reason)
            }
        }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::MissingLocationForNew => {
                write!(f, "New/Delete 命令之前必须存在 Location 命令")
            }
            EngineError::BlockRequiredForDelete => {
                write!(f, "Delete:Block 要求前一个 Location 也使用 Block 指令")
            }
            EngineError::BlockStackEmpty => {
                write!(f, "Block 栈为空，无法执行 Off 操作")
            }
            EngineError::ImplicitOffFailed { reason } => {
                write!(f, "隐式 Off:Open 执行失败: {}", reason)
            }
        }
    }
}

impl fmt::Display for NEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NEditError::Match(e) => write!(f, "{}", e),
            NEditError::Parse(e) => write!(f, "{}", e),
            NEditError::File(e) => write!(f, "{}", e),
            NEditError::Engine(e) => write!(f, "{}", e),
        }
    }
}

impl Error for NEditError {}
impl Error for MatchError {}
impl Error for ParseError {}
impl Error for FileError {}
impl Error for EngineError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_match_error_display() {
        let err = MatchError::NoMatch {
            location_content: "fn main()".to_string(),
        };
        let display = format!("{}", err);
        assert!(display.contains("未找到任何匹配"));
        assert!(display.contains("fn main()"));
    }

    #[test]
    fn test_too_many_matches_error_display() {
        let err = MatchError::TooManyMatches {
            count: 3,
            candidates: vec!["L12: fn foo".to_string(), "L45: fn foo".to_string()],
            location_content: "fn foo".to_string(),
        };
        let display = format!("{}", err);
        assert!(display.contains("3"));
        assert!(display.contains("L12"));
    }

    #[test]
    fn test_parse_error_missing_file_path_display() {
        let err = ParseError::MissingFilePath;
        let display = format!("{}", err);
        assert!(display.contains("文件路径"));
    }

    #[test]
    fn test_parse_error_unknown_command_display() {
        let err = ParseError::UnknownCommand {
            token: "BadCmd".to_string(),
            line: 5,
        };
        let display = format!("{}", err);
        assert!(display.contains("BadCmd"));
        assert!(display.contains("5"));
    }

    #[test]
    fn test_file_error_not_found_display() {
        let err = FileError::NotFound {
            path: "/tmp/test.rs".to_string(),
        };
        let display = format!("{}", err);
        assert!(display.contains("文件未找到"));
        assert!(display.contains("/tmp/test.rs"));
    }

    #[test]
    fn test_engine_error_missing_location_new() {
        let err = EngineError::MissingLocationForNew;
        let display = format!("{}", err);
        assert!(display.contains("Location"));
    }

    #[test]
    fn test_nedit_error_wraps_sub_errors() {
        let err = NEditError::Parse(ParseError::MissingFilePath);
        let display = format!("{}", err);
        assert!(display.contains("文件路径"));
    }

    #[test]
    fn test_delete_match_failed_error_display() {
        let err = MatchError::DeleteMatchFailed {
            delete_content: "let x = 1;".to_string(),
            block_snippet: "fn main() {".to_string(),
        };
        let display = format!("{}", err);
        assert!(display.contains("Delete 命令未能在当前 Block 中找到匹配内容"));
        assert!(display.contains("let x = 1;"));
    }
}
