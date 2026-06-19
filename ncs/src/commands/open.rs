//! Open 命令
//!
//! `!@Open` 打开目标文件或目录，加载内容供后续命令操作。
//!
//! ## 实现逻辑
//!
//! 1. Normal 模式：打开文本文件，支持 start/end 限定读取范围
//! 2. Dir 模式：递归扫描目录，支持 depth/ignore/filter 参数
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §5.1 "Open", INSTRUCTION.md §7.6 "命令模块组织"
//!
//! ## 迁移来源
//!
//! 从 n_edit/src/engine.rs 的 execute_open() 迁移并增强。

use crate::engine::Engine;
use crate::error::NcsError;
use crate::model::FileContent;
use crate::parser::OpenMode;

/// Open 命令的执行入口
pub fn execute(
    engine: &mut Engine,
    mode: OpenMode,
    path: &str,
    args: &std::collections::HashMap<String, String>,
) -> Result<(), NcsError> {
    match mode {
        OpenMode::Normal => execute_normal(engine, path, args),
        OpenMode::Dir => execute_dir(engine, path, args),
    }
}

fn execute_normal(
    engine: &mut Engine,
    path: &str,
    args: &std::collections::HashMap<String, String>,
) -> Result<(), NcsError> {
    engine.file_path = Some(path.to_string());
    let mut file_content = FileContent::from_path(path)?;

    let start: usize = args.get("start").and_then(|s| s.parse().ok()).unwrap_or(1);
    let end_opt: Option<usize> = args.get("end").and_then(|s| s.parse().ok());

    if start > 1 || end_opt.is_some() {
        let end = end_opt.unwrap_or(file_content.lines.len());
        let start_idx = (start.saturating_sub(1)).min(file_content.lines.len());
        let end_idx = (end.saturating_sub(1)).min(file_content.lines.len().saturating_sub(1));

        if start_idx < file_content.lines.len() && end_idx >= start_idx {
            file_content.lines = file_content.lines[start_idx..=end_idx].to_vec();
            // 重建 first_line_index
            use std::collections::HashMap;
            let mut index: HashMap<String, Vec<usize>> = HashMap::new();
            for (i, line) in file_content.lines.iter().enumerate() {
                index
                    .entry(line.stripped_content.clone())
                    .or_default()
                    .push(i);
            }
            file_content.first_line_index = index;
        }
    }

    // reindex 确保行号正确
    let base_taps = file_content.lines.first().map(|l| l.taps).unwrap_or(0);
    for (i, line) in file_content.lines.iter_mut().enumerate() {
        line.line_num = crate::model::LineNumber::from_index(i);
        line.diff_taps = line.taps.saturating_sub(base_taps);
    }

    engine.file = Some(file_content);
    Ok(())
}

fn execute_dir(
    engine: &mut Engine,
    path: &str,
    args: &std::collections::HashMap<String, String>,
) -> Result<(), NcsError> {
    let resolved = engine.work_path.join(path);
    if !resolved.is_dir() {
        return Err(NcsError::File(crate::error::FileError::NotFound {
            path: path.to_string(),
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

    let tree = serialize_dir(&resolved, depth, &ignore_patterns, &filter_patterns);
    let file_content = crate::model::FileContent::from_text(&tree);
    engine.file_path = Some(path.to_string());
    engine.file = Some(file_content);
    engine.is_dir_mode = true;
    engine.dir_snapshot = Some(tree);
    Ok(())
}

/// 将目录结构序列化为树形文本
///
/// 格式:
/// dirname:
///   file1.rs
///   subdir:
///     file3.py
fn serialize_dir(
    dir_path: &std::path::Path,
    depth: usize,
    ignore_patterns: &[&str],
    filter_patterns: &[&str],
) -> String {
    let mut output = String::new();
    let root_name = dir_name(dir_path);
    output.push_str(&format!("{}:\n", root_name));
    serialize_dir_entries(
        dir_path,
        depth - 1,
        ignore_patterns,
        filter_patterns,
        1,
        &mut output,
    );
    output
}

fn serialize_dir_entries(
    dir_path: &std::path::Path,
    remaining_depth: usize,
    ignore_patterns: &[&str],
    filter_patterns: &[&str],
    indent_level: usize,
    output: &mut String,
) {
    let mut entries: Vec<std::path::PathBuf> = match std::fs::read_dir(dir_path) {
        Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
        Err(_) => return,
    };
    entries.sort();

    let indent = "  ".repeat(indent_level);

    for entry in entries {
        let name = dir_name(&entry);
        let is_dir = entry.is_dir();

        if matches_any_pattern(&name, ignore_patterns) {
            continue;
        }
        if !filter_patterns.is_empty() && !is_dir && !matches_any_pattern(&name, filter_patterns) {
            continue;
        }

        if is_dir {
            output.push_str(&format!("{}{}:\n", indent, name));
            if remaining_depth > 0 {
                serialize_dir_entries(
                    &entry,
                    remaining_depth - 1,
                    ignore_patterns,
                    filter_patterns,
                    indent_level + 1,
                    output,
                );
            }
        } else {
            output.push_str(&format!("{}{}\n", indent, name));
        }
    }
}

/// 简单通配符匹配（仅支持 * 和 ?）
fn matches_any_pattern(name: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|p| simple_glob_match(p, name))
}

fn simple_glob_match(pattern: &str, name: &str) -> bool {
    let chars: Vec<char> = pattern.chars().collect();
    let name_chars: Vec<char> = name.chars().collect();
    glob_match_impl(&chars, &name_chars, 0, 0)
}

fn glob_match_impl(pat: &[char], name: &[char], pi: usize, ni: usize) -> bool {
    if pi == pat.len() {
        return ni == name.len();
    }
    match pat[pi] {
        '*' => {
            // * matches zero or more characters
            for skip in 0..=name.len().saturating_sub(ni) {
                if glob_match_impl(pat, name, pi + 1, ni + skip) {
                    return true;
                }
            }
            false
        }
        '?' => ni < name.len() && glob_match_impl(pat, name, pi + 1, ni + 1),
        c => ni < name.len() && name[ni] == c && glob_match_impl(pat, name, pi + 1, ni + 1),
    }
}

/// 树形条目（反序列化结果）
#[derive(Debug, Clone, PartialEq)]
pub struct TreeEntry {
    pub relative_path: String,
    pub is_dir: bool,
}

/// 将树形文本反序列化为条目列表
///
/// 每个条目包含相对于 root 的路径和是否目录。
/// 根条目（缩进=0 的 `dirname:`）代表 root 本身，其相对路径为空串。
pub fn deserialize_tree(tree: &str, _root: &str) -> Vec<TreeEntry> {
    let mut entries = Vec::new();
    let mut path_stack: Vec<String> = Vec::new();
    let mut is_first = true;

    for line in tree.lines() {
        if line.is_empty() {
            continue;
        }
        let indent = count_indent(line);
        let name = line.trim().trim_end_matches(':').to_string();
        let is_dir = line.trim().ends_with(':');

        // 跳过根条目（它代表 root 本身，不加入路径栈）
        if is_first {
            is_first = false;
            continue;
        }

        while path_stack.len() > indent {
            path_stack.pop();
        }

        let relative = if path_stack.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", path_stack.join("/"), name)
        };

        entries.push(TreeEntry {
            relative_path: relative.clone(),
            is_dir,
        });

        if is_dir {
            path_stack.push(name);
        }
    }

    entries
}

/// 计算行的缩进级别（每 2 空格 = 1 级）
fn count_indent(line: &str) -> usize {
    let spaces = line.chars().take_while(|c| *c == ' ').count();
    spaces / 2
}

/// 获取路径的最后一个组件名
fn dir_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;
    use crate::parser::OpenMode;
    use std::collections::HashMap;

    /// 创建临时测试文件
    fn create_temp_file(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test_file.txt");
        std::fs::write(&file_path, content).unwrap();
        (dir, file_path)
    }

    #[test]
    fn test_open_normal_loads_file() {
        let (_dir, file_path) = create_temp_file("line1\nline2\nline3\n");
        let mut engine = Engine::new();

        let result = execute(
            &mut engine,
            OpenMode::Normal,
            file_path.to_str().unwrap(),
            &HashMap::new(),
        );

        assert!(result.is_ok());
        assert!(engine.file_path.is_some());
        assert!(engine.file.is_some());

        let file = engine.file.as_ref().unwrap();
        assert_eq!(file.lines.len(), 3);
        assert_eq!(file.lines[0].content, "line1");
        assert_eq!(file.lines[1].content, "line2");
        assert_eq!(file.lines[2].content, "line3");
    }

    #[test]
    fn test_open_normal_with_start() {
        let (_dir, file_path) = create_temp_file("line1\nline2\nline3\nline4\nline5\n");
        let mut engine = Engine::new();
        let mut args = HashMap::new();
        args.insert("start".to_string(), "2".to_string());

        let result = execute(
            &mut engine,
            OpenMode::Normal,
            file_path.to_str().unwrap(),
            &args,
        );

        assert!(result.is_ok());
        let file = engine.file.as_ref().unwrap();
        assert_eq!(file.lines.len(), 4);
        assert_eq!(file.lines[0].content, "line2");
    }

    #[test]
    fn test_open_normal_with_start_and_end() {
        let (_dir, file_path) = create_temp_file("a\nb\nc\nd\ne\n");
        let mut engine = Engine::new();
        let mut args = HashMap::new();
        args.insert("start".to_string(), "2".to_string());
        args.insert("end".to_string(), "4".to_string());

        let result = execute(
            &mut engine,
            OpenMode::Normal,
            file_path.to_str().unwrap(),
            &args,
        );

        assert!(result.is_ok());
        let file = engine.file.as_ref().unwrap();
        assert_eq!(file.lines.len(), 3);
        assert_eq!(file.lines[0].content, "b");
        assert_eq!(file.lines[2].content, "d");
    }

    #[test]
    fn test_open_file_not_found() {
        let mut engine = Engine::new();
        let result = execute(
            &mut engine,
            OpenMode::Normal,
            "/nonexistent/path/file.txt",
            &HashMap::new(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_open_overwrites_previous() {
        let (_dir1, file_path1) = create_temp_file("aaa\n");
        let (_dir2, file_path2) = create_temp_file("bbb\nccc\n");
        let mut engine = Engine::new();

        execute(
            &mut engine,
            OpenMode::Normal,
            file_path1.to_str().unwrap(),
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(engine.file.as_ref().unwrap().lines.len(), 1);

        execute(
            &mut engine,
            OpenMode::Normal,
            file_path2.to_str().unwrap(),
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(engine.file.as_ref().unwrap().lines.len(), 2);
    }

    #[test]
    fn test_serialize_dir_flat() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "").unwrap();
        std::fs::write(dir.path().join("b.rs"), "").unwrap();
        let tree = serialize_dir(dir.path(), 1, &["*.bin"], &[]);
        let expected = format!("{}:\n  a.txt\n  b.rs\n", dir_name(dir.path()));
        assert_eq!(tree, expected);
    }

    #[test]
    fn test_serialize_dir_nested() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("c.py"), "").unwrap();
        std::fs::write(dir.path().join("main.rs"), "").unwrap();
        let tree = serialize_dir(dir.path(), 3, &["*.bin"], &[]);
        let expected = format!("{}:\n  main.rs\n  sub:\n    c.py\n", dir_name(dir.path()));
        assert_eq!(tree, expected);
    }

    #[test]
    fn test_serialize_dir_ignore_pattern() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("keep.rs"), "").unwrap();
        std::fs::write(dir.path().join("skip.bin"), "").unwrap();
        let tree = serialize_dir(dir.path(), 1, &["*.bin"], &[]);
        let expected = format!("{}:\n  keep.rs\n", dir_name(dir.path()));
        assert_eq!(tree, expected);
    }

    #[test]
    fn test_serialize_dir_filter_pattern() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.rs"), "").unwrap();
        std::fs::write(dir.path().join("b.txt"), "").unwrap();
        let tree = serialize_dir(dir.path(), 1, &[], &["*.rs"]);
        let expected = format!("{}:\n  a.rs\n", dir_name(dir.path()));
        assert_eq!(tree, expected);
    }

    #[test]
    fn test_serialize_dir_depth_limit() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("deep.rs"), "").unwrap();
        std::fs::write(dir.path().join("top.txt"), "").unwrap();
        let tree = serialize_dir(dir.path(), 1, &["*.bin"], &[]);
        let expected = format!("{}:\n  sub:\n  top.txt\n", dir_name(dir.path()));
        assert_eq!(tree, expected);
    }

    // ============================================================
    // deserialize_tree 测试
    // ============================================================

    #[test]
    fn test_deserialize_tree_flat() {
        let root = "/tmp/proj";
        let tree = "proj:\n  a.txt\n  b.rs\n";
        let entries = deserialize_tree(tree, root);
        assert_eq!(entries.len(), 2);
        assert!(entries
            .iter()
            .any(|e| e.relative_path == "a.txt" && !e.is_dir));
        assert!(entries
            .iter()
            .any(|e| e.relative_path == "b.rs" && !e.is_dir));
    }

    #[test]
    fn test_deserialize_tree_nested() {
        let root = "/tmp/proj";
        let tree = "proj:\n  main.rs\n  sub:\n    c.py\n";
        let entries = deserialize_tree(tree, root);
        assert_eq!(entries.len(), 3);
        assert!(entries
            .iter()
            .any(|e| e.relative_path == "main.rs" && !e.is_dir));
        assert!(entries.iter().any(|e| e.relative_path == "sub" && e.is_dir));
        assert!(entries
            .iter()
            .any(|e| e.relative_path == "sub/c.py" && !e.is_dir));
    }

    #[test]
    fn test_deserialize_tree_empty_dir() {
        let root = "/tmp/proj";
        let tree = "proj:\n  sub:\n";
        let entries = deserialize_tree(tree, root);
        assert_eq!(entries.len(), 1);
        assert!(entries.iter().any(|e| e.relative_path == "sub" && e.is_dir));
    }
}
