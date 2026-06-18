//! 错误类型定义 (Error Types)
//!
//! 集中管理项目中所有错误类型。
//! 每个错误类型实现 Display + Error trait，
//! 并附带上下文信息用于构造用户友好的错误提示。
//!
//! ## 实现逻辑
//!
//! 1. `NcsError` 为根错误类型，包含所有子错误变体
//! 2. 每个子错误实现 `title()` / `detail()` / `hints()` 方法
//! 3. 错误格式遵循统一规范：标题 + 详情 + 修复建议
//! 4. 终端输出使用颜色区分：Error 红色、标题黄色、详情灰色、Hint 绿色
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §7 "错误处理体系", INSTRUCTION.md §5

use crate::model::LineNumber;
use std::error::Error;
use std::fmt;

/// 项目的根错误类型
///
/// 包含所有可能的错误变体，统一对外暴露。
#[derive(Debug)]
pub enum NcsError {
    /// 词法/语法解析错误
    Parse(ParseError),
    /// 匹配相关错误
    Match(MatchError),
    /// 文件 I/O 相关错误
    File(FileError),
    /// 引擎执行错误
    Engine(EngineError),
    /// 命令注册表错误
    Registry(RegistryError),
    /// 命令执行错误（Bash、Exec 等外部命令失败）
    CommandExec(CommandExecError),
}

impl NcsError {
    /// 返回错误标题（简短描述）
    pub fn title(&self) -> String {
        match self {
            NcsError::Parse(e) => e.title(),
            NcsError::Match(e) => e.title(),
            NcsError::File(e) => e.title(),
            NcsError::Engine(e) => e.title(),
            NcsError::Registry(e) => e.title(),
            NcsError::CommandExec(e) => e.title(),
        }
    }

    /// 返回错误详情（多行文本）
    pub fn detail(&self) -> String {
        match self {
            NcsError::Parse(e) => e.detail(),
            NcsError::Match(e) => e.detail(),
            NcsError::File(e) => e.detail(),
            NcsError::Engine(e) => e.detail(),
            NcsError::Registry(e) => e.detail(),
            NcsError::CommandExec(e) => e.detail(),
        }
    }

    /// 返回修复建议列表
    pub fn hints(&self) -> Vec<&str> {
        match self {
            NcsError::Parse(e) => e.hints(),
            NcsError::Match(e) => e.hints(),
            NcsError::File(e) => e.hints(),
            NcsError::Engine(e) => e.hints(),
            NcsError::Registry(e) => e.hints(),
            NcsError::CommandExec(e) => e.hints(),
        }
    }
}

// ============================================================
// ParseError — 词法/语法解析错误
// ============================================================

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
        line: LineNumber,
    },
    /// New/Delete 命令前缺少 Location
    MissingLocation {
        /// 命令类型（"New" / "Delete"）
        command: String,
        /// 所在行号
        line: LineNumber,
    },
    /// 意外的分隔符
    UnexpectedSeparator {
        /// 所在行号
        line: LineNumber,
    },
    /// Delete:Block 要求前一个 Location 也使用 Block 指令
    BlockRequiredForDelete {
        /// 所在行号
        line: LineNumber,
    },
    /// 缺少必要的参数
    ParamMissing {
        /// 命令名
        cmd_name: String,
        /// 模式名
        mode_name: String,
        /// 缺失的参数名
        param_name: String,
        /// 所在行号
        line: LineNumber,
    },
}

impl ParseError {
    pub fn title(&self) -> String {
        match self {
            ParseError::MissingFilePath => "Open 命令缺少文件路径参数".to_string(),
            ParseError::UnknownCommand { token, line } => {
                format!("第 {} 行出现无法识别的命令: {}", line, token)
            }
            ParseError::MissingLocation { command, line } => {
                format!("第 {} 行: {} 命令前缺少 Location 定位", line, command)
            }
            ParseError::UnexpectedSeparator { line } => {
                format!("第 {} 行出现意外的分隔符", line)
            }
            ParseError::BlockRequiredForDelete { line } => {
                format!(
                    "第 {} 行: Delete:Block 要求前一个 Location 也使用 Block 指令",
                    line
                )
            }
            ParseError::ParamMissing {
                cmd_name,
                mode_name,
                param_name,
                line,
            } => {
                format!(
                    "第 {} 行: {} 命令的 {} 模式缺少必要参数: {}",
                    line, cmd_name, mode_name, param_name
                )
            }
        }
    }

    pub fn detail(&self) -> String {
        match self {
            ParseError::MissingFilePath => {
                "Open 命令需要一个文件路径参数".to_string()
            }
            ParseError::UnknownCommand { token, .. } => {
                format!("无法识别的命令: \"{}\"，请检查命令拼写", token)
            }
            ParseError::MissingLocation { .. } => {
                "`...` 分隔符导致了插入/删除位置不明确。请在此命令之前使用 Location 明确指定操作位置。"
                    .to_string()
            }
            ParseError::UnexpectedSeparator { .. } => {
                "分隔符出现在非预期位置，这可能破坏了命令流。".to_string()
            }
            ParseError::BlockRequiredForDelete { .. } => {
                "使用 Delete:Block 时，前一个 Location 也必须指定 Block 指令（Location:Block），以确保删除的是整个代码块而非不确定的范围。".to_string()
            }
            ParseError::ParamMissing { .. } => {
                "该参数是该命令模式的必要参数，执行前必须提供。".to_string()
            }
        }
    }

    pub fn hints(&self) -> Vec<&str> {
        match self {
            ParseError::MissingFilePath => {
                vec!["在 Open 后添加文件路径"]
            }
            ParseError::UnknownCommand { .. } => {
                vec![
                    "支持的命令: Open, Location, New, Delete, Raw, Bash, Exec, Read, Write, Include, WorkPath, Get",
                    "命令不区分大小写，检查是否有拼写错误",
                ]
            }
            ParseError::MissingLocation { command, .. } => {
                if command == "New" {
                    vec![
                        "在 New 之前添加 !@Location ... 来指定操作范围",
                        "或者使用 New:Start / New:End 直接在文件首尾插入",
                    ]
                } else {
                    vec![
                        "在 Delete 之前添加 !@Location ... 来指定操作范围",
                        "可以先使用嵌套 Location 精确定位到要删除的内容",
                    ]
                }
            }
            ParseError::UnexpectedSeparator { .. } => {
                vec!["检查分隔符是否正确放置在命令内容之后"]
            }
            ParseError::BlockRequiredForDelete { .. } => {
                vec![
                    "将前一个 Location 改为 Location:Block",
                    "或移除 Delete 的 Block 修饰符",
                ]
            }
            ParseError::ParamMissing { .. } => {
                vec!["请为命令提供所有必要的参数"]
            }
        }
    }
}

// ============================================================
// MatchError — 匹配相关错误
// ============================================================

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
    /// Delete 匹配位置与 Location 不紧邻
    DeleteNotAdjacent {
        /// Location 最后一行
        location_last_line: String,
        /// Delete 首行
        delete_first_line: String,
        /// 中间隔了多少行
        gap_lines: usize,
    },
    /// Block 不可解析
    BlockNotParseable {
        /// 用于定位的内容
        location_content: String,
    },
}

impl MatchError {
    pub fn title(&self) -> String {
        match self {
            MatchError::NoMatch { .. } => "Location 命令未找到任何匹配".to_string(),
            MatchError::TooManyMatches { count, .. } => {
                format!("Location 命令匹配到 {} 个结果（期望 1 个）", count)
            }
            MatchError::DeleteMatchFailed { .. } => {
                "Delete 命令未能在当前 Block 中找到匹配内容".to_string()
            }
            MatchError::DeleteNotAdjacent { gap_lines, .. } => {
                format!(
                    "Delete 匹配位置与 Location 不紧邻（中间隔了 {} 行未经定位的内容）",
                    gap_lines
                )
            }
            MatchError::BlockNotParseable { .. } => {
                "Location:Block 指定但提供内容无法解析为一个 Block".to_string()
            }
        }
    }

    pub fn detail(&self) -> String {
        match self {
            MatchError::NoMatch { location_content } => {
                format!("定位内容:\n{}", location_content)
            }
            MatchError::TooManyMatches {
                candidates,
                location_content,
                ..
            } => {
                let mut detail = format!("Location 内容:\n{}\n匹配候选:\n", location_content);
                for c in candidates {
                    detail.push_str(c);
                    detail.push('\n');
                }
                detail
            }
            MatchError::DeleteMatchFailed {
                delete_content,
                block_snippet,
            } => {
                format!(
                    "删除内容首行: {}\nBlock 内容:\n{}",
                    delete_content, block_snippet
                )
            }
            MatchError::DeleteNotAdjacent {
                location_last_line,
                delete_first_line,
                ..
            } => {
                format!(
                    "Location 最后一行: {}\nDelete 首行: {}",
                    location_last_line, delete_first_line
                )
            }
            MatchError::BlockNotParseable { location_content } => {
                format!("定位内容:\n{}", location_content)
            }
        }
    }

    pub fn hints(&self) -> Vec<&str> {
        match self {
            MatchError::NoMatch { .. } => {
                vec!["请检查定位内容的字符拼写是否与目标文件中的内容一致（忽略空格差异）"]
            }
            MatchError::TooManyMatches { .. } => {
                vec![
                    "请提供更多的上下文行来消除歧义，使匹配结果唯一",
                    "如果定位内容的结构重复出现，可以先用外层 Location 定位到更大范围，再用嵌套 Location 精确定位",
                ]
            }
            MatchError::DeleteMatchFailed { .. } => {
                vec![
                    "请确认删除内容与当前 ContentBlock 中的内容精确匹配（忽略空格差异）",
                    "建议在 Delete 之前使用嵌套 Location 精确定位到要删除的内容",
                ]
            }
            MatchError::DeleteNotAdjacent { .. } => {
                vec![
                    "建议在 Delete 之前使用嵌套 Location 精确定位到要删除的代码块",
                    "确保 Delete 紧随 Location 的最后一行，中间不应有其他代码",
                ]
            }
            MatchError::BlockNotParseable { .. } => {
                vec![
                    "对于纯文本或 Markdown 等不适用大括号/缩进块的语言，请使用不带 Block 的 Location",
                ]
            }
        }
    }
}

// ============================================================
// FileError — 文件 I/O 错误
// ============================================================

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

impl FileError {
    pub fn title(&self) -> String {
        match self {
            FileError::NotFound { path } => format!("文件未找到: {}", path),
            FileError::CannotOpen { path, .. } => format!("无法打开文件: {}", path),
            FileError::WriteFailed { path, .. } => format!("写入文件失败: {}", path),
        }
    }

    pub fn detail(&self) -> String {
        match self {
            FileError::NotFound { .. } => "请确认文件路径是否正确，文件是否存在。".to_string(),
            FileError::CannotOpen { reason, .. } => {
                format!("原因: {}", reason)
            }
            FileError::WriteFailed { reason, .. } => {
                format!("原因: {}", reason)
            }
        }
    }

    pub fn hints(&self) -> Vec<&str> {
        match self {
            FileError::NotFound { .. } => {
                vec!["检查路径拼写是否正确", "使用相对路径时确认当前工作目录"]
            }
            FileError::CannotOpen { .. } => {
                vec!["检查文件权限是否正确", "确认文件没有被其他程序占用"]
            }
            FileError::WriteFailed { .. } => {
                vec!["检查目标目录的写入权限", "确认磁盘空间充足"]
            }
        }
    }
}

// ============================================================
// EngineError — 引擎执行错误
// ============================================================

/// 引擎执行错误
#[derive(Debug)]
pub enum EngineError {
    /// 执行 New/Delete 时缺少前置 Location
    MissingLocationForNew,
    /// Delete:Block 时前一个 Location 未使用 Block
    BlockRequiredForDelete,
    /// Block 栈为空时尝试弹出
    BlockStackEmpty,
    /// 隐式关闭失败
    ImplicitOffFailed {
        /// 失败原因
        reason: String,
    },
}

impl EngineError {
    pub fn title(&self) -> String {
        match self {
            EngineError::MissingLocationForNew => {
                "New/Delete 命令之前必须存在 Location 命令".to_string()
            }
            EngineError::BlockRequiredForDelete => {
                "Delete:Block 要求前一个 Location 也使用 Block 指令".to_string()
            }
            EngineError::BlockStackEmpty => "Block 栈为空，无法执行关闭操作".to_string(),
            EngineError::ImplicitOffFailed { .. } => "隐式关闭执行失败".to_string(),
        }
    }

    pub fn detail(&self) -> String {
        match self {
            EngineError::ImplicitOffFailed { reason } => reason.clone(),
            _ => String::new(),
        }
    }

    pub fn hints(&self) -> Vec<&str> {
        match self {
            EngineError::MissingLocationForNew => {
                vec![
                    "在执行 New/Delete 前，请先使用 Location 定位到目标代码块",
                    "或在文件首尾直接插入，使用 New:Start / New:End",
                ]
            }
            EngineError::BlockRequiredForDelete => {
                vec!["将前一个 Location 改为 Location:Block"]
            }
            EngineError::BlockStackEmpty => {
                vec!["检查关闭命令是否与 Location 正确配对"]
            }
            EngineError::ImplicitOffFailed { .. } => {
                vec!["检查文件是否在脚本执行过程中被修改或删除"]
            }
        }
    }
}

// ============================================================
// RegistryError — 命令注册表错误（新增）
// ============================================================

/// 命令注册表错误
#[derive(Debug)]
pub enum RegistryError {
    /// 命令未注册
    CommandNotFound {
        /// 命令名
        cmd_name: String,
        /// 所在行号
        line: LineNumber,
        /// 基于相似度的候选命令名
        suggestion: Option<String>,
    },
    /// 模式未注册
    ModeNotFound {
        /// 命令名
        cmd_name: String,
        /// 模式名
        mode_name: String,
        /// 所在行号
        line: LineNumber,
    },
    /// 所属命令不在 exec_cmds 中
    OwnerNotExecuted {
        /// 当前命令名
        cmd_name: String,
        /// 所需的前置命令名
        owner_name: String,
        /// 所在行号
        line: LineNumber,
    },
    /// Include alias 重名
    AliasConflict {
        /// 冲突的别名
        alias: String,
        /// 已有的命令名
        existing_cmd: String,
        /// 所在行号
        line: LineNumber,
    },
}

impl RegistryError {
    pub fn title(&self) -> String {
        match self {
            RegistryError::CommandNotFound { cmd_name, line, .. } => {
                format!("第 {} 行: 命令 \"{}\" 未注册", line, cmd_name)
            }
            RegistryError::ModeNotFound {
                cmd_name,
                mode_name,
                line,
            } => {
                format!(
                    "第 {} 行: 命令 \"{}\" 没有模式 \"{}\"",
                    line, cmd_name, mode_name
                )
            }
            RegistryError::OwnerNotExecuted {
                cmd_name,
                owner_name,
                line,
            } => {
                format!(
                    "第 {} 行: 命令 \"{}\" 的前置命令 \"{}\" 尚未执行",
                    line, cmd_name, owner_name
                )
            }
            RegistryError::AliasConflict {
                alias,
                existing_cmd,
                line,
            } => {
                format!(
                    "第 {} 行: Include 别名 \"{}\" 与已有命令 \"{}\" 冲突",
                    line, alias, existing_cmd
                )
            }
        }
    }

    pub fn detail(&self) -> String {
        match self {
            RegistryError::CommandNotFound { suggestion, .. } => {
                if let Some(s) = suggestion {
                    format!("未找到命令。您可能想使用: {}", s)
                } else {
                    "该命令未在命令注册表中找到".to_string()
                }
            }
            RegistryError::ModeNotFound { .. } => {
                "该命令不支持指定的模式，请检查模式名称".to_string()
            }
            RegistryError::OwnerNotExecuted { .. } => {
                "该命令必须在指定的前置命令执行之后才能使用".to_string()
            }
            RegistryError::AliasConflict { .. } => {
                "Include 导入的命令别名不能与已有命令重名".to_string()
            }
        }
    }

    pub fn hints(&self) -> Vec<&str> {
        match self {
            RegistryError::CommandNotFound { .. } => {
                vec!["检查命令拼写是否正确", "使用 --help 查看所有可用命令"]
            }
            RegistryError::ModeNotFound { .. } => {
                vec!["检查模式名称拼写", "不指定模式时默认使用 Normal 模式"]
            }
            RegistryError::OwnerNotExecuted { .. } => {
                vec![
                    "确保前置命令在该命令之前已执行",
                    "检查命令顺序是否符合从属关系",
                ]
            }
            RegistryError::AliasConflict { .. } => {
                vec!["为 Include 命令指定不同的 alias"]
            }
        }
    }
}

// ============================================================
// CommandExecError — 命令执行错误（新增）
// ============================================================

/// 命令执行错误
#[derive(Debug)]
pub enum CommandExecError {
    /// Bash/Exec 执行失败
    ExecutionFailed {
        /// 执行的命令
        command: String,
        /// 退出码
        exit_code: Option<i32>,
        /// 标准错误输出
        stderr: String,
    },
    /// 安全审查拒绝
    SecurityDenied {
        /// 被拒绝的命令
        command: String,
        /// 拒绝原因
        reason: String,
    },
    /// 超时
    Timeout {
        /// 执行的命令
        command: String,
        /// 超时秒数
        timeout_secs: u64,
    },
    /// Include 外部命令失败
    IncludeFailed {
        /// 外部命令路径
        path: String,
        /// 失败原因
        reason: String,
    },
}

impl CommandExecError {
    pub fn title(&self) -> String {
        match self {
            CommandExecError::ExecutionFailed { command, .. } => {
                format!("命令执行失败: {}", command)
            }
            CommandExecError::SecurityDenied { command, .. } => {
                format!("命令被安全策略拒绝: {}", command)
            }
            CommandExecError::Timeout {
                command,
                timeout_secs,
            } => {
                format!("命令执行超时 ({}s): {}", timeout_secs, command)
            }
            CommandExecError::IncludeFailed { path, .. } => {
                format!("Include 外部命令失败: {}", path)
            }
        }
    }

    pub fn detail(&self) -> String {
        match self {
            CommandExecError::ExecutionFailed {
                exit_code, stderr, ..
            } => {
                if let Some(code) = exit_code {
                    format!("退出码: {}, stderr: {}", code, stderr)
                } else {
                    format!("stderr: {}", stderr)
                }
            }
            CommandExecError::SecurityDenied { reason, .. } => {
                format!("拒绝原因: {}", reason)
            }
            CommandExecError::Timeout { .. } => "命令执行超过了允许的时间限制".to_string(),
            CommandExecError::IncludeFailed { reason, .. } => {
                format!("原因: {}", reason)
            }
        }
    }

    pub fn hints(&self) -> Vec<&str> {
        match self {
            CommandExecError::ExecutionFailed { .. } => {
                vec![
                    "检查命令语法是否正确",
                    "确认依赖的程序是否已安装",
                    "使用 --verbose 查看详细执行日志",
                ]
            }
            CommandExecError::SecurityDenied { .. } => {
                vec![
                    "如果确认命令安全，请在终端中手动执行",
                    "检查脚本中是否包含危险操作（sudo、rm -rf / 等）",
                ]
            }
            CommandExecError::Timeout { .. } => {
                vec![
                    "检查命令是否进入死循环或无响应状态",
                    "考虑优化命令或将耗时操作拆分为异步执行",
                ]
            }
            CommandExecError::IncludeFailed { .. } => {
                vec!["检查外部命令路径是否正确", "确认外部命令是否具有可执行权限"]
            }
        }
    }
}

// ============================================================
// Display / Error trait 实现
// ============================================================

macro_rules! impl_display_for_error {
    ($error_type:ty) => {
        impl fmt::Display for $error_type {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}\n{}", self.title(), self.detail())
            }
        }
    };
}

impl_display_for_error!(ParseError);
impl_display_for_error!(MatchError);
impl_display_for_error!(FileError);
impl_display_for_error!(EngineError);
impl_display_for_error!(RegistryError);
impl_display_for_error!(CommandExecError);

impl fmt::Display for NcsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\n{}", self.title(), self.detail())
    }
}

impl Error for NcsError {}
impl Error for ParseError {}
impl Error for MatchError {}
impl Error for FileError {}
impl Error for EngineError {}
impl Error for RegistryError {}
impl Error for CommandExecError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ncs_error_parse_wraps_correctly() {
        let err = NcsError::Parse(ParseError::MissingFilePath);
        assert!(err.title().contains("文件路径"));
        assert!(!err.hints().is_empty());
    }

    #[test]
    fn test_ncs_error_match_wraps_correctly() {
        let err = NcsError::Match(MatchError::NoMatch {
            location_content: "fn main()".to_string(),
        });
        assert!(err.title().contains("未找到任何匹配"));
        assert!(!err.hints().is_empty());
    }

    #[test]
    fn test_ncs_error_file_wraps_correctly() {
        let err = NcsError::File(FileError::NotFound {
            path: "test.rs".to_string(),
        });
        assert!(err.title().contains("文件未找到"));
    }

    #[test]
    fn test_ncs_error_engine_wraps_correctly() {
        let err = NcsError::Engine(EngineError::BlockStackEmpty);
        assert!(err.title().contains("栈为空"));
    }

    #[test]
    fn test_ncs_error_registry_wraps_correctly() {
        let err = NcsError::Registry(RegistryError::AliasConflict {
            alias: "open".to_string(),
            existing_cmd: "Open".to_string(),
            line: LineNumber::new(5),
        });
        assert!(err.title().contains("冲突"));
    }

    #[test]
    fn test_ncs_error_command_exec_wraps_correctly() {
        let err = NcsError::CommandExec(CommandExecError::SecurityDenied {
            command: "sudo rm -rf /".to_string(),
            reason: "包含 sudo 提权操作".to_string(),
        });
        assert!(err.title().contains("安全策略拒绝"));
    }

    #[test]
    fn test_registry_error_command_not_found() {
        let err = RegistryError::CommandNotFound {
            cmd_name: "BadCmd".to_string(),
            line: LineNumber::new(5),
            suggestion: Some("Bash".to_string()),
        };
        assert!(err.title().contains("未注册"));
        assert!(err.detail().contains("Bash"));
    }

    #[test]
    fn test_command_exec_error_execution_failed() {
        let err = CommandExecError::ExecutionFailed {
            command: "ls /nonexistent".to_string(),
            exit_code: Some(2),
            stderr: "No such file".to_string(),
        };
        assert!(err.title().contains("执行失败"));
        assert!(!err.hints().is_empty());
    }

    #[test]
    fn test_ncs_error_display_format() {
        let err = NcsError::Parse(ParseError::MissingFilePath);
        let display = format!("{}", err);
        assert!(display.contains("文件路径"));
    }
}
