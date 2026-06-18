//! 命令间数据传递 (CmdContent)
//!
//! 定义所有内置命令间传递的统一数据结构 + 变更追踪。
//!
//! ## 实现逻辑
//!
//! 1. `CmdContent` 是所有内置命令间传递的统一数据载体
//! 2. 命令不直接修改文件行，而是通过 `record_insert()` / `record_delete()` 追加变更记录
//! 3. 变更在 Owner 命令退出时由 `apply_changes()` 统一生效
//! 4. `snapshot_lines` 是 Location 创建时的原始快照，Delete 匹配始终使用它
//! 5. `CommandResult` 包装命令执行结果，含流/值输出标记
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §3.3 "命令间数据传递 — CmdContent + 变更追踪", INSTRUCTION.md §2.3

use std::fmt;

/// 命令间传递的统一数据结构 + 变更追踪
///
/// 核心模型：命令不直接修改文件行，而是追加 `ContentChange` 记录。
/// 变更在 Owner 命令退出时由 `apply_changes()` 统一生效。
#[derive(Debug, Clone)]
pub struct CmdContent {
    /// 原始文本内容（供命令自行解析使用）
    pub raw_content: String,

    /// 按行解析的通用数据（变更应用后的当前状态）
    pub lines: Vec<CmdLine>,

    /// 是否允许打印（由前一个命令的流/值输出类型决定）
    pub is_print: bool,

    /// 格式化后的输出结果（可包含颜色信息）
    pub result: Vec<CmdLine>,

    // === 变更追踪字段 ===
    /// Location 创建时的原始数据快照（不可变引用，Delete 匹配目标）
    pub snapshot_lines: Vec<CmdLine>,

    /// 快照的原始文本
    pub snapshot_raw: String,

    /// 变更记录列表（按命令执行顺序追加）
    pub changes: Vec<ContentChange>,

    /// 数据来源信息（决定变更写回目标：ContentBlock / FileContent / 命令输出）
    pub source_info: Option<ContentSource>,

    // === 三步流水线内部字段（convert → execute_core 数据传递） ===
    /// 待插入的行列表（New convert 阶段从 NewContent 转换，execute_core 阶段消费）
    pub pending_new_lines: Option<Vec<CmdLine>>,
    /// 待删除的行列表（Delete convert 阶段从 DeleteContent 转换，execute_core 阶段消费）
    pub pending_delete_lines: Option<Vec<CmdLine>>,
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

impl CmdLine {
    /// 返回去除所有空白字符后的内容，用于模糊匹配
    pub fn stripped_content(&self) -> String {
        self.content
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect()
    }
}

/// 内容变更记录 — 命令对 CmdContent 的修改追踪
#[derive(Debug, Clone)]
pub enum ContentChange {
    /// 插入变更（来自 New）
    Insert {
        /// 在 snapshot 中的插入位置（行索引，插入到该行之后）
        after_line: usize,
        /// 插入的行内容
        lines: Vec<CmdLine>,
        /// 变更来源命令
        source_cmd: String,
    },
    /// 删除变更（来自 Delete）
    Delete {
        /// 在 snapshot 中的删除起始行索引
        start_line: usize,
        /// 在 snapshot 中的删除结束行索引（含）
        end_line: usize,
        /// 变更来源命令
        source_cmd: String,
    },
}

/// CmdContent 的数据来源（决定 write_back 目标）
#[derive(Debug, Clone)]
pub enum ContentSource {
    /// 来源为 ContentBlock（Location 匹配产生）
    Block { block_index: usize },
    /// 来源为整个文件（Open 产生）
    File { file_path: String },
    /// 来源为命令输出（Bash / Get 产生）
    CommandOutput,
}

impl fmt::Display for ContentSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ContentSource::Block { block_index } => write!(f, "Block({})", block_index),
            ContentSource::File { file_path } => write!(f, "File({})", file_path),
            ContentSource::CommandOutput => write!(f, "CommandOutput"),
        }
    }
}

impl CmdContent {
    /// 创建一个空的 CmdContent
    pub fn empty() -> Self {
        CmdContent {
            raw_content: String::new(),
            lines: Vec::new(),
            is_print: false,
            result: Vec::new(),
            snapshot_lines: Vec::new(),
            snapshot_raw: String::new(),
            changes: Vec::new(),
            source_info: None,
            pending_new_lines: None,
            pending_delete_lines: None,
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
            snapshot_lines: Vec::new(),
            snapshot_raw: String::new(),
            changes: Vec::new(),
            source_info: None,
            pending_new_lines: None,
            pending_delete_lines: None,
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

    // === 变更追踪方法 ===

    /// 记录一个 Insert 变更（来自 New）
    pub fn record_insert(&mut self, after_line: usize, lines: Vec<CmdLine>, source_cmd: &str) {
        self.changes.push(ContentChange::Insert {
            after_line,
            lines,
            source_cmd: source_cmd.to_string(),
        });
    }

    /// 记录一个 Delete 变更（来自 Delete）
    pub fn record_delete(&mut self, start_line: usize, end_line: usize, source_cmd: &str) {
        self.changes.push(ContentChange::Delete {
            start_line,
            end_line,
            source_cmd: source_cmd.to_string(),
        });
    }

    /// 将所有变更应用到 snapshot，生成最终 lines
    ///
    /// 在 Owner 命令退出时调用。所有变更基于 snapshot 的位置索引。
    /// Deletes 和 Inserts 都相对于原始快照解析，应用顺序不影响最终结果。
    pub fn apply_changes(&mut self) {
        let mut inserts: Vec<(usize, Vec<CmdLine>)> = Vec::new();
        let mut deletes: Vec<(usize, usize)> = Vec::new();

        for change in &self.changes {
            match change {
                ContentChange::Insert {
                    after_line, lines, ..
                } => {
                    inserts.push((*after_line, lines.clone()));
                }
                ContentChange::Delete {
                    start_line,
                    end_line,
                    ..
                } => {
                    deletes.push((*start_line, *end_line));
                }
            }
        }

        let mut result: Vec<CmdLine> = Vec::new();

        for (i, line) in self.snapshot_lines.iter().enumerate() {
            let is_deleted = deletes.iter().any(|(start, end)| i >= *start && i <= *end);
            if !is_deleted {
                result.push(line.clone());
            }

            for (after, insert_lines) in &inserts {
                if *after == i {
                    result.extend(insert_lines.iter().cloned());
                }
            }
        }

        self.lines = result;
        self.raw_content = self
            .lines
            .iter()
            .map(|l| l.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
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
#[derive(Debug, Clone)]
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

    // === Phase 3: Change Tracking Tests ===

    fn make_snapshot(lines: &[&str]) -> Vec<CmdLine> {
        lines
            .iter()
            .enumerate()
            .map(|(i, s)| CmdLine {
                line_num: i + 1,
                content: s.to_string(),
            })
            .collect()
    }

    #[test]
    fn test_record_insert_adds_content_change() {
        let snapshot = make_snapshot(&["A", "B", "C"]);
        let mut content = CmdContent {
            snapshot_lines: snapshot.clone(),
            snapshot_raw: "A\nB\nC".to_string(),
            changes: vec![],
            ..CmdContent::empty()
        };

        let new_lines = make_snapshot(&["X", "Y"]);
        content.record_insert(1, new_lines.clone(), "NEW");

        assert_eq!(content.changes.len(), 1);
        match &content.changes[0] {
            ContentChange::Insert {
                after_line,
                lines,
                source_cmd,
            } => {
                assert_eq!(*after_line, 1);
                assert_eq!(lines.len(), 2);
                assert_eq!(lines[0].content, "X");
                assert_eq!(source_cmd, "NEW");
            }
            _ => panic!("Expected Insert change"),
        }
    }

    #[test]
    fn test_record_delete_adds_content_change() {
        let snapshot = make_snapshot(&["A", "B", "C", "D"]);
        let mut content = CmdContent {
            snapshot_lines: snapshot.clone(),
            snapshot_raw: "A\nB\nC\nD".to_string(),
            changes: vec![],
            ..CmdContent::empty()
        };

        content.record_delete(1, 2, "DELETE");

        assert_eq!(content.changes.len(), 1);
        match &content.changes[0] {
            ContentChange::Delete {
                start_line,
                end_line,
                source_cmd,
            } => {
                assert_eq!(*start_line, 1);
                assert_eq!(*end_line, 2);
                assert_eq!(source_cmd, "DELETE");
            }
            _ => panic!("Expected Delete change"),
        }
    }

    #[test]
    fn test_apply_changes_insert_only() {
        let snapshot = make_snapshot(&["A", "B", "C"]);
        let mut content = CmdContent {
            snapshot_lines: snapshot.clone(),
            snapshot_raw: "A\nB\nC".to_string(),
            changes: vec![],
            ..CmdContent::empty()
        };

        let new_lines = make_snapshot(&["X"]);
        content.record_insert(0, new_lines, "NEW");
        content.apply_changes();

        // After insert "X" after position 0: [A, X, B, C]
        assert_eq!(content.lines.len(), 4);
        assert_eq!(content.lines[0].content, "A");
        assert_eq!(content.lines[1].content, "X");
        assert_eq!(content.lines[2].content, "B");
        assert_eq!(content.lines[3].content, "C");
    }

    #[test]
    fn test_apply_changes_delete_only() {
        let snapshot = make_snapshot(&["A", "B", "C", "D"]);
        let mut content = CmdContent {
            snapshot_lines: snapshot.clone(),
            snapshot_raw: "A\nB\nC\nD".to_string(),
            changes: vec![],
            ..CmdContent::empty()
        };

        content.record_delete(1, 2, "DELETE");
        content.apply_changes();

        // After delete positions 1..2: [A, D]
        assert_eq!(content.lines.len(), 2);
        assert_eq!(content.lines[0].content, "A");
        assert_eq!(content.lines[1].content, "D");
    }

    #[test]
    fn test_apply_changes_insert_then_delete() {
        let snapshot = make_snapshot(&["A", "B", "C"]);
        let mut content = CmdContent {
            snapshot_lines: snapshot.clone(),
            snapshot_raw: "A\nB\nC".to_string(),
            changes: vec![],
            ..CmdContent::empty()
        };

        let new_lines = make_snapshot(&["X"]);
        content.record_insert(0, new_lines, "NEW");
        content.record_delete(1, 2, "DELETE");
        content.apply_changes();

        // Insert "X" after position 0: [A, X, B, C]
        // Delete positions 1..2 (B, C from original): [A, X]
        assert_eq!(content.lines.len(), 2);
        assert_eq!(content.lines[0].content, "A");
        assert_eq!(content.lines[1].content, "X");
    }

    #[test]
    fn test_snapshot_lines_not_modified_by_record_insert() {
        let snapshot = make_snapshot(&["A", "B", "C"]);
        let mut content = CmdContent {
            snapshot_lines: snapshot.clone(),
            snapshot_raw: "A\nB\nC".to_string(),
            changes: vec![],
            ..CmdContent::empty()
        };

        let new_lines = make_snapshot(&["X"]);
        content.record_insert(0, new_lines, "NEW");

        // snapshot_lines must remain unchanged
        assert_eq!(content.snapshot_lines.len(), 3);
        assert_eq!(content.snapshot_lines[0].content, "A");
        assert_eq!(content.snapshot_lines[1].content, "B");
        assert_eq!(content.snapshot_lines[2].content, "C");
    }

    #[test]
    fn test_multiple_inserts_and_deletes_accumulate_in_order() {
        let snapshot = make_snapshot(&["1", "2", "3", "4", "5"]);
        let mut content = CmdContent {
            snapshot_lines: snapshot.clone(),
            snapshot_raw: "1\n2\n3\n4\n5".to_string(),
            changes: vec![],
            ..CmdContent::empty()
        };

        content.record_insert(0, make_snapshot(&["a"]), "NEW");
        content.record_delete(2, 2, "DELETE");
        content.record_insert(3, make_snapshot(&["b"]), "NEW");

        assert_eq!(content.changes.len(), 3);
        match &content.changes[0] {
            ContentChange::Insert { after_line, .. } => assert_eq!(*after_line, 0),
            _ => panic!(),
        }
        match &content.changes[1] {
            ContentChange::Delete { start_line, .. } => assert_eq!(*start_line, 2),
            _ => panic!(),
        }
        match &content.changes[2] {
            ContentChange::Insert { after_line, .. } => assert_eq!(*after_line, 3),
            _ => panic!(),
        }
    }
}
