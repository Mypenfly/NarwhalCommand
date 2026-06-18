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
    _engine: &mut Engine,
    _path: &str,
    _args: &std::collections::HashMap<String, String>,
) -> Result<(), NcsError> {
    // Phase 2.3: 待实现 Dir 模式
    Err(NcsError::File(crate::error::FileError::NotFound {
        path: _path.to_string(),
    }))
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
}
