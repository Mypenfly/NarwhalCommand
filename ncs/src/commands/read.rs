//! Read 命令
//!
//! 读取文件或目录内容并显示，带行号和语法高亮。
//!
//! ## 实现逻辑
//!
//! 1. Normal 模式：读取文件，用 syntect 进行语法高亮，逐行带行号输出
//! 2. Dir 模式：列出目录中的文件/子目录名
//! 3. 值输出：结果仅打印，不保留
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §5.8 "Read", INSTRUCTION.md §3

use crate::cmd_content::{CmdContent, CmdLine};
use crate::error::{FileError, NcsError};
use crate::model::FileContent;
use crate::parser::ReadMode;
use colored::Colorize;
use std::path::Path;

/// Read 命令的执行入口
pub fn execute(mode: ReadMode, path: &str) -> Result<CmdContent, NcsError> {
    match mode {
        ReadMode::Normal => execute_normal(path),
        ReadMode::Dir => execute_dir(path),
    }
}

/// Normal 模式：读取文件内容，带行号和语法高亮
fn execute_normal(path: &str) -> Result<CmdContent, NcsError> {
    let file = FileContent::from_path(path)?;

    let abs_path = std::path::Path::new(path);
    let lang = detect_language(abs_path);

    let mut result_lines: Vec<CmdLine> = Vec::new();

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
    }

    let raw_content = file
        .lines
        .iter()
        .map(|l| l.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    let mut content = CmdContent::from_raw_text(raw_content);
    content.is_print = true;
    content.result = result_lines;
    Ok(content)
}

/// Dir 模式：列出目录内容
fn execute_dir(path: &str) -> Result<CmdContent, NcsError> {
    let p = Path::new(path);
    if !p.is_dir() {
        return Err(NcsError::File(FileError::NotFound {
            path: path.to_string(),
        }));
    }

    let mut result_lines: Vec<CmdLine> = Vec::new();
    let header = format!("Directory: {}", path);
    result_lines.push(CmdLine {
        line_num: 0,
        content: header.bold().to_string(),
    });

    let mut entries: Vec<_> = std::fs::read_dir(p)
        .map_err(|e| FileError::CannotOpen {
            path: path.to_string(),
            reason: e.to_string(),
        })?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by(|a, b| {
        let a_is_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let b_is_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
        b_is_dir
            .cmp(&a_is_dir)
            .then_with(|| a.file_name().cmp(&b.file_name()))
    });

    for (i, entry) in entries.iter().enumerate() {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let display = if is_dir {
            format!("  {}/", name.blue().bold())
        } else {
            format!("  {}", name)
        };
        result_lines.push(CmdLine {
            line_num: i + 1,
            content: display,
        });
    }

    let mut content = CmdContent::empty();
    content.is_print = true;
    content.result = result_lines;
    Ok(content)
}

/// 根据文件扩展名检测语言
fn detect_language(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rs".to_string(),
        Some("py") => "py".to_string(),
        Some("js") | Some("jsx") => "js".to_string(),
        Some("ts") | Some("tsx") => "ts".to_string(),
        Some("go") => "go".to_string(),
        Some("c") | Some("h") => "c".to_string(),
        Some("cpp") | Some("hpp") | Some("cc") | Some("cxx") => "cpp".to_string(),
        Some("java") => "java".to_string(),
        Some("rb") => "rb".to_string(),
        Some("sh") | Some("bash") => "sh".to_string(),
        Some("yaml") | Some("yml") => "yaml".to_string(),
        Some("toml") => "toml".to_string(),
        Some("json") => "json".to_string(),
        Some("md") => "md".to_string(),
        Some("ncs") => "ncs".to_string(),
        _ => "txt".to_string(),
    }
}

/// 对单行内容进行语法高亮（使用 syntect）
fn highlight_line(content: &str, lang: &str) -> String {
    use syntect::easy::HighlightLines;
    use syntect::highlighting::ThemeSet;
    use syntect::parsing::SyntaxSet;
    use syntect::util::as_24_bit_terminal_escaped;

    // 同步初始化语法集合和主题（线程局部存储，避免重复加载）
    use std::sync::OnceLock;
    static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
    static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();

    let ss = SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines);
    let ts = THEME_SET.get_or_init(ThemeSet::load_defaults);
    let syntax = ss
        .find_syntax_by_token(lang)
        .unwrap_or_else(|| ss.find_syntax_plain_text());

    let mut h = HighlightLines::new(syntax, &ts.themes["Solarized (dark)"]);
    let ranges = h.highlight_line(content, ss).unwrap_or_default();
    as_24_bit_terminal_escaped(&ranges, false)
}
