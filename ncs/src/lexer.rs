//! 词法分析器 (Lexer)
//!
//! 负责将输入的 .ncs 脚本内容扫描为 Token 流。
//!
//! ## 实现逻辑
//!
//! 1. 逐行读取脚本内容，识别 `!@` 标识符作为命令起始
//! 2. 根据 CommandRegistry 确定命令的执行类型（行/块/仅展开）
//! 3. 块执行命令按终止规则提取后续内容行
//! 4. `!@Raw` 和 `!@Get` 作为仅展开命令，不触发块终止，内容融入父命令
//! 5. `!@Write Raw` 模式从下一行到 EOF 全部原样提取
//! 6. 输出有序的 Token 序列供 Parser 使用
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §4.1 "词法分析", INSTRUCTION.md §7.2

use crate::error::ParseError;
use crate::model::LineNumber;
use crate::registry::{normalize_command_name, CommandRegistry};
use std::collections::HashMap;

/// 词法分析器产出的 Token
#[derive(Debug, PartialEq)]
pub enum Token {
    /// 命令语句
    Command {
        /// 命令名
        name: String,
        /// 模式名（来自脚本的原始文本，可能为空）
        mode: String,
        /// 键值对参数
        args: HashMap<String, String>,
        /// 非键值对的定位参数
        positional_args: Vec<String>,
        /// 命令所在行号
        line: LineNumber,
        /// 块执行命令的内容行（行执行为空）
        content_lines: Vec<String>,
        /// 是否为块执行
        is_block: bool,
    },
    /// 关闭符号
    Close {
        /// 关闭的命令名
        name: String,
        /// 所在行号
        line: LineNumber,
    },
    /// Capture 指令：捕获命令输出到 pools
    Capture {
        /// 存入 pools 的键名
        pool_name: String,
        /// 所在行号
        line: LineNumber,
    },
}

impl Token {
    /// 检查该 Token 是否为块执行命令
    fn is_block(&self) -> bool {
        match self {
            Token::Command { is_block, .. } => *is_block,
            _ => false,
        }
    }
}

/// 词法分析器
pub struct Lexer;

impl Lexer {
    /// 对脚本内容执行词法分析，返回 Token 流
    ///
    /// # 参数
    ///
    /// * `script` — .ncs 脚本的全部文本
    /// * `registry` — 命令注册表，用于确定命令的执行类型
    ///
    /// # 错误
    ///
    /// 遇到无法识别的命令时返回 `ParseError::UnknownCommand`。
    pub fn tokenize(script: &str, registry: &CommandRegistry) -> Result<Vec<Token>, ParseError> {
        let lines: Vec<&str> = script.lines().collect();
        let total_lines = lines.len();
        let mut tokens: Vec<Token> = Vec::new();
        let mut index = 0;

        while index < total_lines {
            let line = lines[index];
            let line_number = LineNumber::from_index(index);

            // 跳过非命令的普通行（它们已由前一个块命令的提取消费）
            if !line.starts_with("!@") && !line.starts_with("@/") {
                index += 1;
                continue;
            }

            if let Some(header) = line.strip_prefix("!@") {
                let token = Self::parse_command_header(header, line_number, registry)?;

                if token.is_block() {
                    // 提取块内容
                    let cmd_name = match &token {
                        Token::Command { name, .. } => name.clone(),
                        _ => unreachable!(),
                    };
                    let (content_lines, next_index) =
                        Self::extract_block_content(&lines, index, registry, &cmd_name)?;
                    tokens.push(Self::set_content_lines(token, content_lines));
                    index = next_index;
                } else {
                    tokens.push(token);
                    index += 1;
                }
            } else if let Some(rest) = line.strip_prefix("@/") {
                let rest = rest.trim();
                if let Some(capture_token) = Self::parse_capture(rest, line_number) {
                    tokens.push(capture_token);
                } else {
                    tokens.push(Token::Close {
                        name: rest.trim().to_string(),
                        line: line_number,
                    });
                }
                index += 1;
            }
        }

        Ok(tokens)
    }

    /// 为 Command Token 设置 content_lines
    fn set_content_lines(token: Token, content_lines: Vec<String>) -> Token {
        match token {
            Token::Command {
                name,
                mode,
                args,
                positional_args,
                line,
                is_block,
                ..
            } => Token::Command {
                name,
                mode,
                args,
                positional_args,
                line,
                content_lines,
                is_block,
            },
            other => other,
        }
    }

    /// 解析单行 `!@Cmd [mode] [args...]` 命令头部
    ///
    /// 返回不包含 content_lines 的 Token（content_lines 由调用方填入）。
    fn parse_command_header(
        header: &str,
        line_number: LineNumber,
        registry: &CommandRegistry,
    ) -> Result<Token, ParseError> {
        let parts: Vec<&str> = header.split_whitespace().collect();

        if parts.is_empty() {
            return Err(ParseError::UnknownCommand {
                token: header.to_string(),
                line: line_number,
            });
        }

        let cmd_name = parts[0];

        // 查找命令注册表
        let command_entry =
            registry
                .find_command(cmd_name)
                .ok_or_else(|| ParseError::UnknownCommand {
                    token: cmd_name.to_string(),
                    line: line_number,
                })?;

        let is_block = command_entry.cmd_type.is_block_exec();

        // 解析 pre_mode 和 args
        let (mode, positional_args, args) = if parts.len() >= 2 {
            let potential_mode = parts[1];
            // 检查 pre_mode 是否与某个已知模式匹配（不区分大小写）
            let mode_normalized = normalize_command_name(potential_mode);
            let mode_found = command_entry
                .modes
                .keys()
                .any(|k| normalize_command_name(k) == mode_normalized);
            if mode_found {
                // 匹配到模式，剩余为位置参数和键值参数
                let (pos, kv) = parse_remaining_args(&parts[2..]);
                (potential_mode.to_string(), pos, kv)
            } else if potential_mode.contains('=') {
                // 第一个 token 就是 key=value，无模式，无位置参数
                let (pos, kv) = parse_remaining_args(&parts[1..]);
                (String::new(), pos, kv)
            } else {
                // 不匹配任何模式 → 作为位置参数，模式默认空
                let (pos, kv) = parse_remaining_args(&parts[1..]);
                (String::new(), pos, kv)
            }
        } else {
            (String::new(), Vec::new(), HashMap::new())
        };

        Ok(Token::Command {
            name: cmd_name.to_string(),
            mode,
            args,
            positional_args,
            line: line_number,
            content_lines: Vec::new(),
            is_block,
        })
    }

    /// 提取块执行命令的内容行
    ///
    /// 从命令行的下一行开始收集，直到：
    /// - 遇到匹配当前命令名的 `@/Cmd` 关闭行
    /// - 遇到非仅展开命令的 `!@` 行
    ///
    /// 非匹配的 `@/Cmd` 行会作为内容行继续提取。
    ///
    /// 返回 (内容行列表, 下一处理行索引)
    fn extract_block_content(
        lines: &[&str],
        start_index: usize,
        registry: &CommandRegistry,
        cmd_name: &str,
    ) -> Result<(Vec<String>, usize), ParseError> {
        let total = lines.len();
        let mut content_lines: Vec<String> = Vec::new();
        let mut next_index = start_index + 1;
        let cmd_name_upper = cmd_name.to_uppercase();

        while next_index < total {
            let line = lines[next_index];

            if line.starts_with("@/") {
                // 提取 @/ 后的命令名，检查是否匹配当前块命令
                let rest = line.strip_prefix("@/").unwrap().trim();
                let close_name = rest.split_whitespace().next().unwrap_or("");
                // @/Open / @/Off 为根关闭符，@/Location 为块上下文关闭符；
                // 这三者始终终止任何已开启的块内容提取。
                // 其他 @/Cmd 仅当命令名匹配时才终止块。
                let close_upper = close_name.to_uppercase();
                if matches!(close_upper.as_str(), "OPEN" | "OFF" | "LOCATION")
                    || close_upper == cmd_name_upper
                {
                    break;
                }
                // 不匹配的关闭符 — 作为内容行继续提取
                content_lines.push(line.to_string());
                next_index += 1;
                continue;
            }

            if let Some(header) = line.strip_prefix("!@") {
                // 检查是否为仅展开命令
                let cmd_name = header.split_whitespace().next().unwrap_or("");

                if let Some(entry) = registry.find_command(cmd_name) {
                    if entry.cmd_type.is_expand_only() {
                        // 仅展开命令：内容融入父命令，继续提取
                        content_lines.push(line.to_string());
                        next_index += 1;
                        continue;
                    }
                }
                // 非仅展开命令 → 停止提取
                break;
            }

            // 普通内容行
            content_lines.push(line.to_string());
            next_index += 1;
        }

        Ok((content_lines, next_index))
    }

    /// 解析 `@/Cmd | Capture pool_name` Capture 指令
    ///
    /// 返回 Some(Token::Capture) 如果是 Capture 指令，否则返回 None。
    fn parse_capture(rest: &str, line_number: LineNumber) -> Option<Token> {
        let rest = rest.trim();
        if let Some(pipe_pos) = rest.find('|') {
            let after_pipe = rest[pipe_pos + 1..].trim();
            if after_pipe.to_lowercase().starts_with("capture") {
                let pool_name = after_pipe["capture".len()..].trim().to_string();
                if !pool_name.is_empty() {
                    return Some(Token::Capture {
                        pool_name,
                        line: line_number,
                    });
                }
            }
        }
        None
    }
}

/// 解析剩余参数列表：不含 `=` 的 token 作为位置参数，
/// 含 `=` 的 token 按第一个 `=` 拆分为 key 和 value。
fn parse_remaining_args(tokens: &[&str]) -> (Vec<String>, HashMap<String, String>) {
    let mut positional_args: Vec<String> = Vec::new();
    let mut args: HashMap<String, String> = HashMap::new();

    for token in tokens {
        if let Some(eq_pos) = token.find('=') {
            let key = token[..eq_pos].to_string();
            let value = token[eq_pos + 1..].to_string();
            args.insert(key, value);
        } else {
            positional_args.push(token.to_string());
        }
    }

    (positional_args, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::CommandRegistry;

    fn test_registry() -> CommandRegistry {
        CommandRegistry::init()
    }

    // ============================================================
    // 空脚本 / 无命令脚本
    // ============================================================

    #[test]
    fn test_empty_script_returns_no_tokens() {
        let registry = test_registry();
        let tokens = Lexer::tokenize("", &registry).unwrap();
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_script_without_commands_returns_no_tokens() {
        let registry = test_registry();
        let script = "this is just text\nno commands here\nstill nothing\n";
        let tokens = Lexer::tokenize(script, &registry).unwrap();
        assert!(tokens.is_empty());
    }

    // ============================================================
    // 行执行命令
    // ============================================================

    #[test]
    fn test_line_exec_command_bash() {
        let registry = test_registry();
        let tokens = Lexer::tokenize("!@Bash echo hello", &registry).unwrap();
        assert_eq!(tokens.len(), 1);
        match &tokens[0] {
            Token::Command {
                name,
                mode,
                positional_args,
                is_block,
                content_lines,
                line,
                ..
            } => {
                assert_eq!(name, "Bash");
                assert_eq!(mode, "");
                assert_eq!(
                    positional_args,
                    &vec!["echo".to_string(), "hello".to_string()]
                );
                assert!(!is_block);
                assert!(content_lines.is_empty());
                assert_eq!(*line, 1);
            }
            _ => panic!("Expected Command token"),
        }
    }

    #[test]
    fn test_line_exec_command_work_path() {
        let registry = test_registry();
        let tokens = Lexer::tokenize("!@WorkPath /tmp/test", &registry).unwrap();
        assert_eq!(tokens.len(), 1);
        match &tokens[0] {
            Token::Command {
                name,
                positional_args,
                is_block,
                ..
            } => {
                assert_eq!(name, "WorkPath");
                assert_eq!(positional_args, &vec!["/tmp/test".to_string()]);
                assert!(!is_block);
            }
            _ => panic!("Expected Command token"),
        }
    }

    #[test]
    fn test_line_exec_command_open() {
        let registry = test_registry();
        let tokens = Lexer::tokenize("!@Open ./test.rs", &registry).unwrap();
        assert_eq!(tokens.len(), 1);
        match &tokens[0] {
            Token::Command {
                name,
                positional_args,
                is_block,
                line,
                ..
            } => {
                assert_eq!(name, "Open");
                assert_eq!(positional_args, &vec!["./test.rs".to_string()]);
                assert!(!is_block);
                assert_eq!(*line, 1);
            }
            _ => panic!("Expected Command token"),
        }
    }

    #[test]
    fn test_line_exec_command_open_with_mode() {
        let registry = test_registry();
        let tokens = Lexer::tokenize("!@Open Dir ./mydir depth=5", &registry).unwrap();
        assert_eq!(tokens.len(), 1);
        match &tokens[0] {
            Token::Command {
                name,
                mode,
                positional_args,
                args,
                ..
            } => {
                assert_eq!(name, "Open");
                assert_eq!(mode, "Dir");
                assert_eq!(positional_args, &vec!["./mydir".to_string()]);
                assert_eq!(args.get("depth"), Some(&"5".to_string()));
            }
            _ => panic!("Expected Command token"),
        }
    }

    #[test]
    fn test_line_exec_command_open_with_kv_before_positional() {
        let registry = test_registry();
        let tokens = Lexer::tokenize("!@Open start=10 ./test.rs", &registry).unwrap();
        assert_eq!(tokens.len(), 1);
        match &tokens[0] {
            Token::Command {
                name,
                mode,
                positional_args,
                args,
                ..
            } => {
                assert_eq!(name, "Open");
                assert_eq!(mode, "");
                assert_eq!(positional_args, &vec!["./test.rs".to_string()]);
                assert_eq!(args.get("start"), Some(&"10".to_string()));
            }
            _ => panic!("Expected Command token"),
        }
    }

    // ============================================================
    // 块执行命令
    // ============================================================

    #[test]
    fn test_block_exec_command_new_with_content() {
        let registry = test_registry();
        let script = "!@New\n  let x = 1;\n  let y = 2;\n@/New";
        let tokens = Lexer::tokenize(script, &registry).unwrap();
        assert_eq!(tokens.len(), 2);
        match &tokens[0] {
            Token::Command {
                name,
                is_block,
                content_lines,
                ..
            } => {
                assert_eq!(name, "New");
                assert!(is_block);
                assert_eq!(
                    content_lines,
                    &vec!["  let x = 1;".to_string(), "  let y = 2;".to_string()]
                );
            }
            _ => panic!("Expected Command token"),
        }
        match &tokens[1] {
            Token::Close { name, .. } => {
                assert_eq!(name, "New");
            }
            _ => panic!("Expected Close token"),
        }
    }

    #[test]
    fn test_block_exec_command_terminated_by_next_command() {
        let registry = test_registry();
        let script = "!@New\n  let x = 1;\n!@New\n  let y = 2;\n@/New";
        let tokens = Lexer::tokenize(script, &registry).unwrap();
        assert_eq!(tokens.len(), 3);
        let first_content = match &tokens[0] {
            Token::Command { content_lines, .. } => content_lines.clone(),
            _ => panic!(),
        };
        assert_eq!(first_content, vec!["  let x = 1;".to_string()]);
    }

    #[test]
    fn test_block_exec_command_location() {
        let registry = test_registry();
        let script = "!@Location\nfn main() {\n  println!();\n}\n@/Location";
        let tokens = Lexer::tokenize(script, &registry).unwrap();
        assert_eq!(tokens.len(), 2);
        match &tokens[0] {
            Token::Command {
                name,
                is_block,
                content_lines,
                ..
            } => {
                assert_eq!(name, "Location");
                assert!(is_block);
                assert_eq!(content_lines.len(), 3);
            }
            _ => panic!("Expected Command token"),
        }
    }

    #[test]
    fn test_block_exec_command_delete() {
        let registry = test_registry();
        let script = "!@Delete\nold_code();\n@/Delete";
        let tokens = Lexer::tokenize(script, &registry).unwrap();
        assert_eq!(tokens.len(), 2);
        match &tokens[0] {
            Token::Command {
                name,
                is_block,
                content_lines,
                ..
            } => {
                assert_eq!(name, "Delete");
                assert!(is_block);
                assert_eq!(content_lines, &vec!["old_code();".to_string()]);
            }
            _ => panic!("Expected Command token"),
        }
    }

    // ============================================================
    // Close Token
    // ============================================================

    #[test]
    fn test_close_token() {
        let registry = test_registry();
        let tokens = Lexer::tokenize("@/Open", &registry).unwrap();
        assert_eq!(tokens.len(), 1);
        match &tokens[0] {
            Token::Close { name, line } => {
                assert_eq!(name, "Open");
                assert_eq!(*line, 1);
            }
            _ => panic!("Expected Close token"),
        }
    }

    #[test]
    fn test_close_token_with_extra_whitespace() {
        let registry = test_registry();
        let tokens = Lexer::tokenize("@/  Location  ", &registry).unwrap();
        assert_eq!(tokens.len(), 1);
        match &tokens[0] {
            Token::Close { name, .. } => {
                assert_eq!(name, "Location");
            }
            _ => panic!("Expected Close token"),
        }
    }

    // ============================================================
    // Capture Token
    // ============================================================

    #[test]
    fn test_capture_token() {
        let registry = test_registry();
        let tokens = Lexer::tokenize("@/Open | Capture my_result", &registry).unwrap();
        assert_eq!(tokens.len(), 1);
        match &tokens[0] {
            Token::Capture { pool_name, line } => {
                assert_eq!(pool_name, "my_result");
                assert_eq!(*line, 1);
            }
            _ => panic!("Expected Capture token"),
        }
    }

    #[test]
    fn test_capture_case_insensitive() {
        let registry = test_registry();
        let tokens = Lexer::tokenize("@/Open | capture my_pool", &registry).unwrap();
        assert_eq!(tokens.len(), 1);
        match &tokens[0] {
            Token::Capture { pool_name, .. } => {
                assert_eq!(pool_name, "my_pool");
            }
            _ => panic!("Expected Capture token"),
        }
    }

    // ============================================================
    // 仅展开命令不终止父块
    // ============================================================

    #[test]
    fn test_expand_only_raw_does_not_terminate_block() {
        let registry = test_registry();
        let script = "!@New\n  line one\n!@Raw raw line\n  line two\n@/New";
        let tokens = Lexer::tokenize(script, &registry).unwrap();
        assert_eq!(tokens.len(), 2);
        match &tokens[0] {
            Token::Command { content_lines, .. } => {
                assert_eq!(
                    content_lines,
                    &vec![
                        "  line one".to_string(),
                        "!@Raw raw line".to_string(),
                        "  line two".to_string(),
                    ]
                );
            }
            _ => panic!("Expected Command token"),
        }
    }

    #[test]
    fn test_expand_only_get_does_not_terminate_block() {
        let registry = test_registry();
        let script = "!@New\n  line one\n!@Get my_pool\n  line two\n@/New";
        let tokens = Lexer::tokenize(script, &registry).unwrap();
        assert_eq!(tokens.len(), 2);
        match &tokens[0] {
            Token::Command { content_lines, .. } => {
                assert_eq!(
                    content_lines,
                    &vec![
                        "  line one".to_string(),
                        "!@Get my_pool".to_string(),
                        "  line two".to_string(),
                    ]
                );
            }
            _ => panic!("Expected Command token"),
        }
    }

    // ============================================================
    // 行号信息
    // ============================================================

    #[test]
    fn test_token_line_numbers() {
        let registry = test_registry();
        let script = "!@Bash echo hello\n!@Exec ls\n@/Open";
        let tokens = Lexer::tokenize(script, &registry).unwrap();
        assert_eq!(tokens.len(), 3);
        match &tokens[0] {
            Token::Command { line, .. } => assert_eq!(*line, 1),
            _ => panic!(),
        }
        match &tokens[1] {
            Token::Command { line, .. } => assert_eq!(*line, 2),
            _ => panic!(),
        }
        match &tokens[2] {
            Token::Close { line, .. } => assert_eq!(*line, 3),
            _ => panic!(),
        }
    }

    // ============================================================
    // Unknown command
    // ============================================================

    #[test]
    fn test_unknown_command_returns_error() {
        let registry = test_registry();
        let result = Lexer::tokenize("!@UnknownCmd foo bar", &registry);
        assert!(result.is_err());
        match result.unwrap_err() {
            ParseError::UnknownCommand { token, line } => {
                assert_eq!(token, "UnknownCmd");
                assert_eq!(line, 1);
            }
            _ => panic!("Expected UnknownCommand error"),
        }
    }

    // ============================================================
    // 混合复杂场景
    // ============================================================

    #[test]
    fn test_multiple_commands_and_closes() {
        let registry = test_registry();
        let script = "!@Open ./test.rs\n!@Location\nfn main() {}\n@/Location\n!@New\n  extra();\n@/New\n@/Open";
        let tokens = Lexer::tokenize(script, &registry).unwrap();
        assert_eq!(tokens.len(), 6);
    }

    #[test]
    fn test_mode_from_registry() {
        let registry = test_registry();
        let tokens =
            Lexer::tokenize("!@Location Block\ncode_here();\n@/Location", &registry).unwrap();
        assert_eq!(tokens.len(), 2);
        match &tokens[0] {
            Token::Command {
                name,
                mode,
                is_block,
                ..
            } => {
                assert_eq!(name, "Location");
                assert_eq!(mode, "Block");
                assert!(is_block);
            }
            _ => panic!("Expected Command token"),
        }
    }

    // ============================================================
    // BUG-301: @/ 块终止必须校验命令名匹配
    // ============================================================

    #[test]
    fn test_block_does_not_terminate_on_non_matching_close() {
        let registry = test_registry();
        // @/New 不应终止 !@Location 块（非 Open/Off/Location + 名不匹配）
        let script = "!@Location\nfn main() {}\n@/New\n  extra code\n@/Location";
        let tokens = Lexer::tokenize(script, &registry).unwrap();
        assert_eq!(tokens.len(), 2, "应该只有 2 个 token");
        match &tokens[0] {
            Token::Command {
                name,
                content_lines,
                ..
            } => {
                assert_eq!(name, "Location");
                assert_eq!(content_lines.len(), 3);
                assert_eq!(content_lines[0], "fn main() {}");
                assert_eq!(content_lines[1], "@/New");
                assert_eq!(content_lines[2], "  extra code");
            }
            _ => panic!("Expected Command token"),
        }
        match &tokens[1] {
            Token::Close { name, .. } => {
                assert_eq!(name, "Location");
            }
            _ => panic!("Expected Close token for Location"),
        }
    }

    #[test]
    fn test_block_terminates_on_open_close_even_in_nested_block() {
        let registry = test_registry();
        // @/Open / @/Off / @/Location 始终终止任何块
        let script = "!@Location\nfn main() {}\n@/Open\n!@New\n  code();\n@/New";
        let tokens = Lexer::tokenize(script, &registry).unwrap();
        assert_eq!(tokens.len(), 4, "应有 4 个 token");
        match &tokens[1] {
            Token::Close { name, .. } => assert_eq!(name, "Open"),
            _ => panic!("Expected Close Open token"),
        }
    }

    #[test]
    fn test_location_close_terminates_any_block() {
        let registry = test_registry();
        // @/Location 作为块上下文关闭符，始终终止任何块
        let script = "!@New\n  line 1\n@/Location";
        let tokens = Lexer::tokenize(script, &registry).unwrap();
        assert_eq!(tokens.len(), 2);
        match &tokens[0] {
            Token::Command { content_lines, .. } => {
                assert_eq!(content_lines.len(), 1);
                assert_eq!(content_lines[0], "  line 1");
            }
            _ => panic!("Expected Command token"),
        }
        match &tokens[1] {
            Token::Close { name, .. } => assert_eq!(name, "Location"),
            _ => panic!("Expected Close Location token"),
        }
    }

    #[test]
    fn test_delete_close_does_not_terminate_location_block() {
        let registry = test_registry();
        // @/Delete 不应终止 !@Location 块
        let script = "!@Location\nfn main() {}\n@/Delete\n  still content\n@/Location";
        let tokens = Lexer::tokenize(script, &registry).unwrap();
        assert_eq!(tokens.len(), 2);
        match &tokens[0] {
            Token::Command {
                name,
                content_lines,
                ..
            } => {
                assert_eq!(name, "Location");
                assert_eq!(content_lines.len(), 3);
                assert_eq!(content_lines[1], "@/Delete");
            }
            _ => panic!("Expected Command token"),
        }
    }

    #[test]
    fn test_mode_defaults_to_empty_for_unknown_mode() {
        let registry = test_registry();
        let tokens = Lexer::tokenize("!@Open Something ./path", &registry).unwrap();
        assert_eq!(tokens.len(), 1);
        match &tokens[0] {
            Token::Command {
                mode,
                positional_args,
                ..
            } => {
                assert_eq!(mode, "");
                assert_eq!(
                    positional_args,
                    &vec!["Something".to_string(), "./path".to_string()]
                );
            }
            _ => panic!("Expected Command token"),
        }
    }
}
