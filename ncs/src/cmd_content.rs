//! 命令间数据传递 (CmdContent)
//!
//! 定义所有内置命令间传递的统一数据结构。
//!
//! ## 实现逻辑
//!
//! 1. `CmdContent` 是所有内置命令间传递的统一数据载体
//! 2. 每个内置命令必须实现 `convert()` → 内部结构和 `out()` → CmdContent
//! 3. `raw_content` 保留原始文本，`lines` 为按行解析的通用数据
//! 4. `is_print` 控制是否输出到终端，`result` 为格式化后的结果
//! 5. `CommandResult` 包装命令执行结果，含流/值输出标记
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §3.3 "命令间数据传递 — CmdContent", INSTRUCTION.md §2.3

/// 命令间传递的统一数据结构
///
/// 所有内置命令之间通过此结构传递数据，
/// 其核心职责是保证数据格式在各级命令间的完整性与一致性。
#[derive(Debug, Clone)]
pub struct CmdContent {
    /// 原始文本内容（供命令自行解析使用）
    pub raw_content: String,

    /// 按行解析的通用数据
    pub lines: Vec<CmdLine>,

    /// 是否允许打印（由前一个命令的流/值输出类型决定）
    pub is_print: bool,

    /// 格式化后的输出结果（可包含颜色信息）
    pub result: Vec<CmdLine>,
}

/// 通用的行数据结构
///
/// 携带行号和内容，支持从各种来源（文件、命令输出等）构建。
#[derive(Debug, Clone)]
pub struct CmdLine {
    /// 行号（对应于最初内容的行号，如文件中的行号）
    pub line_num: usize,
    /// 行内容
    pub content: String,
}

impl CmdContent {
    /// 创建一个空的 CmdContent
    pub fn empty() -> Self {
        CmdContent {
            raw_content: String::new(),
            lines: Vec::new(),
            is_print: false,
            result: Vec::new(),
        }
    }

    /// 从原始文本构建 CmdContent，自动按行解析
    ///
    /// 逐行填入 `lines`，并将全文存入 `raw_content`。
    pub fn from_raw_text(raw: String) -> Self {
        let lines: Vec<CmdLine> = raw
            .lines()
            .enumerate()
            .map(|(index, content)| CmdLine {
                line_num: index + 1,
                content: content.to_string(),
            })
            .collect();
        CmdContent {
            raw_content: raw,
            lines,
            is_print: false,
            result: Vec::new(),
        }
    }

    /// 序列化为最原始的字符串
    ///
    /// 用于作为外部命令调用的最后一个参数传递。
    pub fn send(&self) -> String {
        self.raw_content.clone()
    }

    /// 若 is_print 为 true，将 result 输出到终端
    ///
    /// 根据 `is_print` 控制打印行为。
    pub fn print(&self) {
        if !self.is_print {
            return;
        }
        for line in &self.result {
            println!("{}", line.content);
        }
    }
}

impl Default for CmdContent {
    fn default() -> Self {
        Self::empty()
    }
}

/// 命令执行结果
///
/// 每个命令执行完毕后返回此结构，
/// 包含输出的 CmdContent 和流/值输出标记。
#[derive(Debug)]
pub struct CommandResult {
    /// 输出的 CmdContent（供从属命令使用）
    pub content: CmdContent,

    /// 是否为流输出（true 则保留在内存，false 则仅打印后丢弃）
    pub is_stream: bool,
}

impl CommandResult {
    /// 创建流输出结果
    pub fn stream(content: CmdContent) -> Self {
        CommandResult {
            content,
            is_stream: true,
        }
    }

    /// 创建值输出结果（不保留）
    pub fn value(content: CmdContent) -> Self {
        CommandResult {
            content,
            is_stream: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cmd_content_empty() {
        let content = CmdContent::empty();
        assert!(content.raw_content.is_empty());
        assert!(content.lines.is_empty());
        assert!(!content.is_print);
    }

    #[test]
    fn test_cmd_content_from_raw_text() {
        let raw = "line1\nline2\nline3".to_string();
        let content = CmdContent::from_raw_text(raw);
        assert_eq!(content.lines.len(), 3);
        assert_eq!(content.lines[0].line_num, 1);
        assert_eq!(content.lines[0].content, "line1");
        assert_eq!(content.lines[2].line_num, 3);
        assert_eq!(content.lines[2].content, "line3");
    }

    #[test]
    fn test_cmd_content_send_returns_raw() {
        let raw = "hello world".to_string();
        let content = CmdContent::from_raw_text(raw.clone());
        assert_eq!(content.send(), raw);
    }

    #[test]
    fn test_cmd_content_print_when_is_print_true() {
        let mut content = CmdContent::empty();
        content.is_print = true;
        content.result = vec![
            CmdLine {
                line_num: 1,
                content: "result1".to_string(),
            },
            CmdLine {
                line_num: 2,
                content: "result2".to_string(),
            },
        ];
        // print() 输出到 stdout，此处仅验证不 panic
        content.print();
    }

    #[test]
    fn test_cmd_content_print_when_is_print_false() {
        let mut content = CmdContent::empty();
        content.is_print = false;
        content.result = vec![CmdLine {
            line_num: 1,
            content: "should_not_print".to_string(),
        }];
        // 不应该输出任何内容
        content.print();
    }

    #[test]
    fn test_command_result_stream() {
        let content = CmdContent::from_raw_text("data".to_string());
        let result = CommandResult::stream(content);
        assert!(result.is_stream);
        assert_eq!(result.content.raw_content, "data");
    }

    #[test]
    fn test_command_result_value() {
        let content = CmdContent::from_raw_text("data".to_string());
        let result = CommandResult::value(content);
        assert!(!result.is_stream);
        assert_eq!(result.content.raw_content, "data");
    }

    #[test]
    fn test_cmd_line_creation() {
        let line = CmdLine {
            line_num: 42,
            content: "hello".to_string(),
        };
        assert_eq!(line.line_num, 42);
        assert_eq!(line.content, "hello");
    }

    #[test]
    fn test_cmd_content_empty_lines_index() {
        let raw = "".to_string();
        let content = CmdContent::from_raw_text(raw);
        assert_eq!(content.lines.len(), 0);
    }

    #[test]
    fn test_cmd_content_single_line() {
        let raw = "single".to_string();
        let content = CmdContent::from_raw_text(raw);
        assert_eq!(content.lines.len(), 1);
        assert_eq!(content.lines[0].line_num, 1);
        assert_eq!(content.lines[0].content, "single");
    }
}
