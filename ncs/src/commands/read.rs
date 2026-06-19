//! Read 命令
//!
//! 读取文件或目录内容并显示，带行号和语法高亮。
//!
//! ## 实现逻辑
//!
//! 1. Normal 模式：读取文件，支持 start/end 参数限定范围，syntect 语法高亮，行号灰色右对齐
//! 2. Dir 模式：目录树形结构，目录名蓝色加粗，文件普通显示
//! 3. 值输出：结果仅打印，不保留
//! 4. 路径基于 engine.work_path 展开
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §5.8 "Read", INSTRUCTION.md §3

use crate::cmd_content::{CmdContent, CmdLine};
use crate::engine::Engine;
use crate::error::NcsError;
use crate::model::FileContent;
use crate::parser::ReadMode;
use colored::Colorize;
use std::collections::HashMap;
use std::path::Path;

/// 语法高亮主题名
const HIGHLIGHT_THEME: &str = "base16-ocean.dark";

/// Read 命令的执行入口
pub fn execute(
    engine: &Engine,
    mode: ReadMode,
    path: &str,
    args: &HashMap<String, String>,
) -> Result<CmdContent, NcsError> {
    let resolved = engine.work_path.join(path);
    match mode {
        ReadMode::Normal => execute_normal(&resolved, args),
        ReadMode::Dir => execute_dir(&resolved, args),
    }
}

/// Normal 模式默认最大显示行数（end = start + DEFAULT_WINDOW）
const DEFAULT_WINDOW: usize = 999;

/// Normal 模式：读取文件内容，带行号和语法高亮
fn execute_normal(path: &Path, args: &HashMap<String, String>) -> Result<CmdContent, NcsError> {
    let path_str = path.to_string_lossy();
    let mut file = FileContent::from_path(&path_str)?;
    let total = file.lines.len();

    let start: usize = args.get("start").and_then(|s| s.parse().ok()).unwrap_or(1);
    let end: usize = args
        .get("end")
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            let wanted = start.saturating_add(DEFAULT_WINDOW);
            if wanted < total {
                eprintln!(
                    "{} 文件共 {} 行，默认最多显示 1000 行（{}..{}）。使用 end 参数指定更大范围。",
                    "Warning:".yellow(),
                    total,
                    start,
                    wanted,
                );
            }
            wanted
        })
        .min(total);

    let start_idx = (start.saturating_sub(1)).min(total);
    let end_idx = end.saturating_sub(1).min(total.saturating_sub(1));

    if start_idx <= end_idx && start_idx < total {
        file.lines = file.lines[start_idx..=end_idx].to_vec();
    }

    let lang = detect_language(path);
    let mut result_lines: Vec<CmdLine> = Vec::new();
    let mut raw_parts: Vec<&str> = Vec::new();

    for line in &file.lines {
        let line_num = line.line_num.to_usize();
        let highlighted = highlight_line(&line.content, &lang);
        let display = format!(
            "{:>6}  {}",
            line_num.to_string().bright_black(),
            highlighted
        );
        result_lines.push(CmdLine {
            line_num,
            content: display,
        });
        raw_parts.push(&line.content);
    }

    let raw_content = raw_parts.join("\n");
    let mut content = CmdContent::from_raw_text(raw_content);
    content.is_print = true;
    content.result = result_lines;
    Ok(content)
}

/// Dir 模式：目录树形结构输出
fn execute_dir(path: &Path, args: &HashMap<String, String>) -> Result<CmdContent, NcsError> {
    if !path.is_dir() {
        return Err(NcsError::File(crate::error::FileError::NotFound {
            path: path.to_string_lossy().to_string(),
        }));
    }

    let depth: usize = args.get("depth").and_then(|s| s.parse().ok()).unwrap_or(3);
    let ignore_patterns: Vec<&str> = args
        .get("ignore")
        .map(|s| s.split(',').map(|p| p.trim()).collect())
        .unwrap_or_else(|| vec!["*.bin"]);
    let filter_patterns: Vec<&str> = args
        .get("filter")
        .map(|s| s.split(',').map(|p| p.trim()).collect())
        .unwrap_or_default();

    let root_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());

    let mut result_lines: Vec<CmdLine> = Vec::new();
    result_lines.push(CmdLine {
        line_num: 0,
        content: format!("{}:", root_name.blue().bold()),
    });

    build_dir_tree(
        path,
        depth,
        &ignore_patterns,
        &filter_patterns,
        1,
        &mut result_lines,
    );

    let mut content = CmdContent::empty();
    content.is_print = true;
    content.result = result_lines;
    Ok(content)
}

/// 递归构建目录树形结构（用于显示）
fn build_dir_tree(
    dir: &Path,
    max_depth: usize,
    ignore_patterns: &[&str],
    filter_patterns: &[&str],
    indent_level: usize,
    lines: &mut Vec<CmdLine>,
) {
    if max_depth == 0 {
        return;
    }

    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
        Err(_) => return,
    };
    entries.sort_by(|a, b| {
        let a_is_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let b_is_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
        b_is_dir
            .cmp(&a_is_dir)
            .then_with(|| a.file_name().cmp(&b.file_name()))
    });

    let indent = "  ".repeat(indent_level);
    let line_num_base = lines.len();

    for (i, entry) in entries.iter().enumerate() {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);

        if matches_any_pattern(&name, ignore_patterns) {
            continue;
        }
        if !filter_patterns.is_empty() && !is_dir && !matches_any_pattern(&name, filter_patterns) {
            continue;
        }

        let display = if is_dir {
            format!("{}{}:", indent, name.blue().bold())
        } else {
            format!("{}{}", indent, name)
        };

        lines.push(CmdLine {
            line_num: line_num_base + i + 1,
            content: display,
        });

        if is_dir {
            build_dir_tree(
                &entry.path(),
                max_depth - 1,
                ignore_patterns,
                filter_patterns,
                indent_level + 1,
                lines,
            );
        }
    }
}

/// 简单通配符匹配（仅支持 *）
fn matches_any_pattern(name: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|p| simple_glob_match(p, name))
}

fn simple_glob_match(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        return name.ends_with(suffix);
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }
    pattern == name
}

/// 根据文件扩展名检测语言
fn detect_language(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rs".to_string(),
        Some("py") => "py".to_string(),
        Some("js") | Some("jsx") | Some("mjs") => "js".to_string(),
        Some("ts") | Some("tsx") | Some("mts") => "ts".to_string(),
        Some("go") => "go".to_string(),
        Some("c") | Some("h") => "c".to_string(),
        Some("cpp") | Some("hpp") | Some("cc") | Some("cxx") => "cpp".to_string(),
        Some("java") => "java".to_string(),
        Some("rb") => "rb".to_string(),
        Some("sh") | Some("bash") | Some("zsh") => "sh".to_string(),
        Some("yaml") | Some("yml") => "yaml".to_string(),
        Some("toml") => "toml".to_string(),
        Some("json") => "json".to_string(),
        Some("md") | Some("mdx") => "md".to_string(),
        Some("css") => "css".to_string(),
        Some("html") | Some("htm") => "html".to_string(),
        Some("xml") | Some("svg") => "xml".to_string(),
        Some("sql") => "sql".to_string(),
        Some("nix") => "nix".to_string(),
        _ => "txt".to_string(),
    }
}

/// 对单行内容进行语法高亮（使用 syntect）
fn highlight_line(content: &str, lang: &str) -> String {
    use std::sync::OnceLock;
    use syntect::easy::HighlightLines;
    use syntect::highlighting::ThemeSet;
    use syntect::parsing::SyntaxSet;
    use syntect::util::as_24_bit_terminal_escaped;

    static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
    static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();

    let ss = SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines);
    let ts = THEME_SET.get_or_init(ThemeSet::load_defaults);
    let syntax = ss
        .find_syntax_by_token(lang)
        .unwrap_or_else(|| ss.find_syntax_plain_text());

    let theme = &ts.themes[HIGHLIGHT_THEME];
    let mut h = HighlightLines::new(syntax, theme);
    let ranges = h.highlight_line(content, ss).unwrap_or_default();
    as_24_bit_terminal_escaped(&ranges, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ReadMode;
    use std::collections::HashMap;

    fn make_engine() -> Engine {
        Engine::new()
    }

    #[test]
    fn test_read_normal_loads_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "line1\nline2\nline3\n").unwrap();

        let mut engine = make_engine();
        engine.work_path = dir.path().to_path_buf();

        let result = execute(&engine, ReadMode::Normal, "test.txt", &HashMap::new());
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(content.is_print);
        assert_eq!(content.result.len(), 3);
    }

    #[test]
    fn test_read_normal_with_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "a\nb\nc\nd\ne\n").unwrap();

        let mut engine = make_engine();
        engine.work_path = dir.path().to_path_buf();
        let mut args = HashMap::new();
        args.insert("start".to_string(), "2".to_string());

        let result = execute(&engine, ReadMode::Normal, "test.txt", &args);
        assert!(result.is_ok());
        let content = result.unwrap();
        assert_eq!(content.result.len(), 4); // lines 2-5
        assert!(content.result[0].content.contains('b'));
    }

    #[test]
    fn test_read_normal_with_start_and_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "a\nb\nc\nd\ne\n").unwrap();

        let mut engine = make_engine();
        engine.work_path = dir.path().to_path_buf();
        let mut args = HashMap::new();
        args.insert("start".to_string(), "2".to_string());
        args.insert("end".to_string(), "4".to_string());

        let result = execute(&engine, ReadMode::Normal, "test.txt", &args);
        assert!(result.is_ok());
        let content = result.unwrap();
        assert_eq!(content.result.len(), 3);
        assert!(content.result[0].content.contains('b'));
        assert!(content.result[2].content.contains('d'));
    }

    #[test]
    fn test_read_normal_file_not_found() {
        let engine = make_engine();
        let result = execute(
            &engine,
            ReadMode::Normal,
            "/nonexistent/file.txt",
            &HashMap::new(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_read_dir_tree_structure() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "").unwrap();
        std::fs::write(dir.path().join("b.rs"), "").unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("c.py"), "").unwrap();

        let mut engine = make_engine();
        engine.work_path = dir.path().parent().unwrap().to_path_buf();
        let dir_name = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let result = execute(&engine, ReadMode::Dir, &dir_name, &HashMap::new());
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(content.is_print);
        // 应有 dirname: 头 + 条目
        assert!(content.result.len() >= 3);
        // 第一个是 dirname:
        assert!(content.result[0].content.contains(':'));
    }

    #[test]
    fn test_read_dir_not_found() {
        let engine = make_engine();
        let result = execute(&engine, ReadMode::Dir, "/nonexistent_dir", &HashMap::new());
        assert!(result.is_err());
    }

    #[test]
    fn test_read_dir_not_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        std::fs::write(&path, "content").unwrap();

        let mut engine = make_engine();
        engine.work_path = dir.path().to_path_buf();

        let result = execute(&engine, ReadMode::Dir, "file.txt", &HashMap::new());
        assert!(result.is_err());
    }

    #[test]
    fn test_read_normal_work_path_resolution() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("subdir");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("nested.txt"), "hello\n").unwrap();

        let mut engine = make_engine();
        engine.work_path = dir.path().to_path_buf();

        // 使用相对于 work_path 的路径
        let result = execute(
            &engine,
            ReadMode::Normal,
            "subdir/nested.txt",
            &HashMap::new(),
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_read_syntax_highlight_no_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rs");
        std::fs::write(&path, "fn main() {\n    println!(\"hello\");\n}\n").unwrap();

        let mut engine = make_engine();
        engine.work_path = dir.path().to_path_buf();

        let result = execute(&engine, ReadMode::Normal, "test.rs", &HashMap::new());
        assert!(result.is_ok());
        let content = result.unwrap();
        assert!(!content.result.is_empty());
        // 高亮后的内容包含 ANSI 转义序列，剥离后验证原文存在
        let first = &content.result[0].content;
        let stripped = strip_ansi(&first);
        assert!(stripped.contains("fn"), "Should contain fn: {}", stripped);
        assert!(
            stripped.contains("main"),
            "Should contain main: {}",
            stripped
        );
    }

    #[test]
    fn test_read_normal_end_clamped_to_total() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        let content: String = (0..10).map(|i| format!("line{}\n", i)).collect();
        std::fs::write(&path, &content).unwrap();

        let mut engine = make_engine();
        engine.work_path = dir.path().to_path_buf();
        let mut args = HashMap::new();
        args.insert("end".to_string(), "999".to_string());

        // end=999 超出文件总行数 10，clamp 到 10
        let result = execute(&engine, ReadMode::Normal, "test.txt", &args);
        assert!(result.is_ok());
        let c = result.unwrap();
        assert_eq!(c.result.len(), 10);
    }

    #[test]
    fn test_read_normal_end_beyond_clamped_with_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        let content: String = (0..20).map(|i| format!("line{}\n", i)).collect();
        std::fs::write(&path, &content).unwrap();

        let mut engine = make_engine();
        engine.work_path = dir.path().to_path_buf();
        let mut args = HashMap::new();
        args.insert("start".to_string(), "18".to_string());

        // end 默认 = start+999 = 1017, clamp 到 20, 结果 lines 18-20 = 3 行
        let result = execute(&engine, ReadMode::Normal, "test.txt", &args);
        assert!(result.is_ok());
        let c = result.unwrap();
        assert_eq!(c.result.len(), 3);
    }

    #[test]
    fn test_read_dir_with_depth() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("deep.rs"), "").unwrap();
        std::fs::write(dir.path().join("top.txt"), "").unwrap();

        let mut engine = make_engine();
        engine.work_path = dir.path().parent().unwrap().to_path_buf();
        let dir_name = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let mut args = HashMap::new();
        args.insert("depth".to_string(), "1".to_string());

        let result = execute(&engine, ReadMode::Dir, &dir_name, &args);
        assert!(result.is_ok());
        let c = result.unwrap();
        // depth=1 应有 root: + sub: + top.txt，但不含 deep.rs
        let text: String = c
            .result
            .iter()
            .map(|l| &l.content)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let stripped = strip_ansi(&text);
        assert!(stripped.contains("top.txt"));
        assert!(stripped.contains("sub:"));
        assert!(
            !stripped.contains("deep.rs"),
            "depth=1 should not show deep.rs"
        );
    }

    #[test]
    fn test_read_dir_with_ignore() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("keep.rs"), "").unwrap();
        std::fs::write(dir.path().join("skip.bin"), "").unwrap();

        let mut engine = make_engine();
        engine.work_path = dir.path().parent().unwrap().to_path_buf();
        let dir_name = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let mut args = HashMap::new();
        args.insert("ignore".to_string(), "*.bin".to_string());

        let result = execute(&engine, ReadMode::Dir, &dir_name, &args);
        assert!(result.is_ok());
        let c = result.unwrap();
        let text: String = c
            .result
            .iter()
            .map(|l| &l.content)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let stripped = strip_ansi(&text);
        assert!(stripped.contains("keep.rs"));
        assert!(!stripped.contains("skip.bin"));
    }

    #[test]
    fn test_read_dir_with_filter() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "").unwrap();
        std::fs::write(dir.path().join("b.txt"), "").unwrap();

        let mut engine = make_engine();
        engine.work_path = dir.path().parent().unwrap().to_path_buf();
        let dir_name = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let mut args = HashMap::new();
        args.insert("filter".to_string(), "*.rs".to_string());

        let result = execute(&engine, ReadMode::Dir, &dir_name, &args);
        assert!(result.is_ok());
        let c = result.unwrap();
        let text: String = c
            .result
            .iter()
            .map(|l| &l.content)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        let stripped = strip_ansi(&text);
        assert!(stripped.contains("a.rs"));
        assert!(!stripped.contains("b.txt"));
    }

    /// 剥离 ANSI 转义序列
    fn strip_ansi(text: &str) -> String {
        let mut result = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' && chars.peek() == Some(&'[') {
                chars.next(); // skip [
                while let Some(&d) = chars.peek() {
                    chars.next();
                    if d == 'm' {
                        break;
                    }
                }
            } else {
                result.push(c);
            }
        }
        result
    }
}
