//! Location 命令
//!
//! `!@Location` 在搜索范围内精确定位代码位置，
//! 支持 Normal/Block/Path 三种模式。
//!
//! ## 实现逻辑
//!
//! 1. 从 Engine 获取 SearchScope（block_stack 顶 / file）
//! 2. 调用 LocationMatcher 执行匹配
//! 3. 将匹配的 ContentBlock 压入 block_stack
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §5.2 "Location", INSTRUCTION.md §3.1 "Location 匹配算法"
//!
//! ## 迁移来源
//!
//! 从 n_edit/src/engine.rs 的 execute_location() 迁移。

use crate::engine::Engine;
use crate::error::NcsError;
use crate::matcher::LocationMatcher;
use crate::model::LocationContent;
use crate::parser::LocationMode;

/// Location 命令的执行入口
pub fn execute(
    engine: &mut Engine,
    mode: LocationMode,
    content: Option<LocationContent>,
    _args: &std::collections::HashMap<String, String>,
) -> Result<(), NcsError> {
    match mode {
        LocationMode::Normal | LocationMode::Block => {
            execute_content_match(engine, content, matches!(mode, LocationMode::Block))
        }
        LocationMode::Path => Err(NcsError::Engine(
            crate::error::EngineError::NotImplemented {
                feature: "Location Path 模式".to_string(),
            },
        )),
    }
}

fn execute_content_match(
    engine: &mut Engine,
    content: Option<LocationContent>,
    block: bool,
) -> Result<(), NcsError> {
    let search_scope = engine.get_search_scope()?;

    let content_block = if let Some(loc) = content {
        LocationMatcher::find_unique_block(&search_scope, &loc, block)?
    } else {
        // 无定位内容时，返回整个搜索范围
        LocationMatcher::find_unique_block(
            &search_scope,
            &LocationContent { lines: vec![] },
            false,
        )?
    };

    engine.block_stack.push(content_block);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;
    use crate::parser::{LocationMode, OpenMode};
    use std::collections::HashMap;

    /// 创建带临时文件的引擎（已执行 Open）
    fn engine_with_file(content: &str) -> (tempfile::TempDir, Engine) {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.rs");
        std::fs::write(&file_path, content).unwrap();

        let mut engine = Engine::new();
        crate::commands::open::execute(
            &mut engine,
            OpenMode::Normal,
            file_path.to_str().unwrap(),
            &HashMap::new(),
        )
        .unwrap();
        (dir, engine)
    }

    #[test]
    fn test_location_normal_finds_match() {
        let (_dir, mut engine) = engine_with_file("// comment\nfn main() {\n    let x = 1;\n}\n");

        let result = execute(
            &mut engine,
            LocationMode::Normal,
            Some(build_location_content(&["fn main() {"])),
            &HashMap::new(),
        );

        assert!(result.is_ok());
        assert_eq!(engine.block_stack.len(), 1);
        let block = &engine.block_stack[0];
        assert!(block.lines.len() >= 1);
        assert_eq!(block.lines[0].content, "fn main() {");
    }

    #[test]
    fn test_location_block_finds_block_boundary() {
        let (_dir, mut engine) = engine_with_file(
            "// header\nfn main() {\n    let x = 1;\n    println!(\"{}\", x);\n}\n// footer\n",
        );

        let result = execute(
            &mut engine,
            LocationMode::Block,
            Some(build_location_content(&["fn main() {"])),
            &HashMap::new(),
        );

        assert!(result.is_ok());
        assert_eq!(engine.block_stack.len(), 1);
        let block = &engine.block_stack[0];
        assert_eq!(block.lines.len(), 4); // fn main { let x; println; }
        assert_eq!(block.lines[0].content, "fn main() {");
        assert_eq!(block.lines[3].content, "}");
    }

    #[test]
    fn test_location_without_open_errors() {
        let mut engine = Engine::new();
        let result = execute(
            &mut engine,
            LocationMode::Normal,
            Some(build_location_content(&["fn main() {"])),
            &HashMap::new(),
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_location_empty_content_returns_full_scope() {
        let (_dir, mut engine) = engine_with_file("line1\nline2\nline3\n");

        let result = execute(&mut engine, LocationMode::Normal, None, &HashMap::new());

        assert!(result.is_ok());
        assert_eq!(engine.block_stack.len(), 1);
        let block = &engine.block_stack[0];
        assert_eq!(block.lines.len(), 3);
    }

    #[test]
    fn test_nested_location() {
        let (_dir, mut engine) = engine_with_file(
            "fn outer() {\n    fn inner() {\n        let x = 1;\n    }\n    let y = 2;\n}\n",
        );

        // First location: find outer
        execute(
            &mut engine,
            LocationMode::Normal,
            Some(build_location_content(&["fn outer() {"])),
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(engine.block_stack.len(), 1);

        // Second location: find inner within outer
        let result = execute(
            &mut engine,
            LocationMode::Normal,
            Some(build_location_content(&["fn inner() {"])),
            &HashMap::new(),
        );

        assert!(result.is_ok());
        assert_eq!(engine.block_stack.len(), 2);
        let inner = &engine.block_stack[1];
        assert_eq!(inner.lines[0].content, "    fn inner() {");
    }

    /// 辅助函数：根据字符串切片构建 LocationContent
    fn build_location_content(lines: &[&str]) -> LocationContent {
        use crate::model::LocationLine;
        if lines.is_empty() {
            return LocationContent { lines: vec![] };
        }
        let base_taps = crate::model::count_leading_spaces(lines[0]);
        let loc_lines: Vec<LocationLine> = lines
            .iter()
            .enumerate()
            .map(|(i, content)| {
                let line_taps = crate::model::count_leading_spaces(content);
                let diff_taps = Some(line_taps.saturating_sub(base_taps));
                LocationLine {
                    index: i,
                    diff_taps,
                    content: content.to_string(),
                    line_num: None,
                }
            })
            .collect();
        LocationContent { lines: loc_lines }
    }

    #[test]
    fn test_location_path_returns_not_implemented() {
        let mut engine = Engine::new();
        let result = execute(
            &mut engine,
            LocationMode::Path,
            None,
            &std::collections::HashMap::new(),
        );
        assert!(result.is_err());
        match result.unwrap_err() {
            NcsError::Engine(crate::error::EngineError::NotImplemented { .. }) => {}
            other => panic!("Expected NotImplemented error, got {:?}", other),
        }
    }
}
