//! 命令执行引擎 (Engine)
//!
//! 维护全局状态机，按顺序消费 Parser 输出的 AST 节点。
//!
//! ## 状态流转
//!
//! Open → Location (可嵌套) → New/Delete/Raw → Off
//!
//! ## 错误恢复
//!
//! 执行失败时保持在内存中修改，不写回原文件，
//! 确保原文件不受部分执行的影响。
//!
//! ## 对应文档
//!
//! 详见 INSTRUCTION.md 第 1.3 节 "命令状态机" 及第 3.3-3.4 节

use crate::error::{FileError, NEditError};
use crate::matcher::LocationMatcher;
use crate::model::{ContentBlock, FileContent, Line};
use crate::output::{DiffLine, DiffLineKind};
use crate::parser::{Command, OffTarget};

/// 命令执行引擎
///
/// 维护全局状态机，按顺序消费 Parser 输出的 AST 节点。
pub struct Engine {
    /// 当前打开的文件路径（用于最终写回）
    file_path: Option<String>,
    /// 当前打开的文件内容（Open 命令后设置）
    pub file: Option<FileContent>,
    /// Location 嵌套栈（栈顶为当前操作作用域）
    pub block_stack: Vec<ContentBlock>,
    /// 执行过程中累积的差异输出行（New=Added, Delete=Deleted）
    pub diff_lines: Vec<DiffLine>,
}

/// 在 ContentBlock 中查找 DeleteContent 的连续匹配区间
///
/// 返回 (start_index, end_index) 在 block.lines 中的索引。
/// 要求所有行连续匹配，不可跳行。
fn find_delete_match(
    block: &ContentBlock,
    del_content: &crate::model::DeleteContent,
) -> Option<(usize, usize)> {
    let del_lines = &del_content.lines;
    if del_lines.is_empty() || block.lines.is_empty() {
        return None;
    }

    let first_del_stripped = crate::model::stripped_content(&del_lines[0].content);

    for start_idx in 0..block.lines.len() {
        if block.lines[start_idx].stripped_content() != first_del_stripped {
            continue;
        }

        if start_idx + del_lines.len() > block.lines.len() {
            continue;
        }

        let mut matched = true;
        for (offset, del_line) in del_lines.iter().enumerate() {
            let block_line = &block.lines[start_idx + offset];

            let block_stripped = block_line.stripped_content();
            let del_stripped = crate::model::stripped_content(&del_line.content);

            let block_is_empty = block_line.content.trim().is_empty();
            let del_is_empty = del_line.content.trim().is_empty();

            if block_is_empty && del_is_empty {
                continue;
            }
            if block_is_empty || del_is_empty {
                matched = false;
                break;
            }
            if block_stripped != del_stripped {
                matched = false;
                break;
            }
        }

        if matched {
            return Some((start_idx, start_idx + del_lines.len() - 1));
        }
    }

    None
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    /// 创建新的执行引擎实例
    pub fn new() -> Self {
        Engine {
            file_path: None,
            file: None,
            block_stack: Vec::new(),
            diff_lines: Vec::new(),
        }
    }

    /// 执行完整的 AST 命令序列
    ///
    /// 遍历 commands，逐条调用对应的处理方法。
    /// 执行完毕后自动处理隐式 Off:Open（若脚本末尾未显式关闭）。
    pub fn execute(&mut self, commands: Vec<Command>) -> Result<(), NEditError> {
        for command in commands {
            match command {
                Command::Open { file_path } => {
                    self.execute_open(&file_path)?;
                }
                Command::Location { content, .. } => {
                    self.execute_location(&content)?;
                }
                Command::New { position, content } => {
                    self.execute_new(&position, &content)?;
                }
                Command::Delete { block: _, content } => {
                    self.execute_delete(content.as_ref())?;
                }
                Command::Off { target } => {
                    self.execute_off(&target)?;
                }
            }
        }

        self.handle_implicit_off()
    }

    /// 执行 Open 命令：读取文件并构建 FileContent
    fn execute_open(&mut self, file_path: &str) -> Result<(), NEditError> {
        let file = FileContent::from_path(file_path).map_err(NEditError::File)?;
        self.file_path = Some(file_path.to_string());
        self.file = Some(file);
        Ok(())
    }

    /// 执行 Location 命令：匹配定位内容，将 ContentBlock 推入栈
    fn execute_location(
        &mut self,
        location_content: &crate::model::LocationContent,
    ) -> Result<(), NEditError> {
        let search_scope = self.get_search_scope()?;
        let block = LocationMatcher::find_unique_block(search_scope, location_content)
            .map_err(NEditError::Match)?;
        self.block_stack.push(block);
        Ok(())
    }

    /// 执行 Off 命令：根据目标弹出栈或写回文件
    fn execute_off(&mut self, target: &OffTarget) -> Result<(), NEditError> {
        match target {
            OffTarget::Location => {
                let popped_block = self.block_stack.pop().ok_or(
                    NEditError::Engine(crate::error::EngineError::BlockStackEmpty),
                )?;
                self.write_back_to_parent(popped_block)?;
            }
            OffTarget::New => {
                let popped_block = self.block_stack.pop().ok_or(
                    NEditError::Engine(crate::error::EngineError::BlockStackEmpty),
                )?;
                self.write_back_to_parent(popped_block)?;
            }
            OffTarget::Open => {
                self.write_back_to_file()?;
            }
        }
        Ok(())
    }

    /// 获取当前 Location 的搜索范围
    fn get_search_scope(&self) -> Result<&FileContent, NEditError> {
        self.file
            .as_ref()
            .ok_or(NEditError::File(FileError::NotFound {
                path: "(no file opened)".to_string(),
            }))
    }

    /// 将弹出的 ContentBlock 写回到父级
    fn write_back_to_parent(
        &mut self,
        block: ContentBlock,
    ) -> Result<(), NEditError> {
        if let Some(ref mut file) = self.file {
            apply_block_to_file(file, &block);
        }
        Ok(())
    }

    /// 将所有修改最终写回磁盘文件
    fn write_back_to_file(&mut self) -> Result<(), NEditError> {
        while let Some(block) = self.block_stack.pop() {
            if let Some(ref mut file) = self.file {
                apply_block_to_file(file, &block);
            }
        }

        if let (Some(ref file), Some(ref path)) = (&self.file, &self.file_path) {
            file.write_back(path).map_err(NEditError::File)?;
        }

        Ok(())
    }

    /// 处理隐式 Off:Open — 脚本末尾未显式关闭时自动写回
    fn handle_implicit_off(&mut self) -> Result<(), NEditError> {
        if self.file.is_some() {
            self.write_back_to_file()?;
        }
        Ok(())
    }

    /// 执行 New 命令：在 ContentBlock 中插入新内容
    fn execute_new(
        &mut self,
        position: &crate::parser::NewPosition,
        content: &crate::model::NewContent,
    ) -> Result<(), NEditError> {
        use crate::parser::NewPosition;

        match position {
            NewPosition::Start => {
                let new_lines = build_new_lines(0, content);
                let new_line_count = new_lines.len();
                // Scope the mutable borrow so we can later access self.diff_lines
                let added_entries: Vec<(usize, String)> = {
                    if let Some(ref mut block) = self.block_stack.last_mut() {
                        let mut combined = new_lines;
                        combined.append(&mut block.lines);
                        block.lines = combined;
                        block.reindex();
                        let end = new_line_count.min(block.lines.len());
                        (0..end)
                            .map(|i| (block.lines[i].line_num, block.lines[i].content.clone()))
                            .collect()
                    } else if let Some(ref mut file) = self.file {
                        let mut combined = new_lines;
                        combined.append(&mut file.lines);
                        file.lines = combined;
                        reindex_file(file);
                        let end = new_line_count.min(file.lines.len());
                        (0..end)
                            .map(|i| (file.lines[i].line_num, file.lines[i].content.clone()))
                            .collect()
                    } else {
                        Vec::new()
                    }
                };
                for (line_num, content) in added_entries {
                    self.diff_lines.push(DiffLine {
                        kind: DiffLineKind::Added,
                        line_number: Some(line_num),
                        content,
                    });
                }
            }
            NewPosition::End => {
                let new_lines = build_new_lines(0, content);
                let new_line_count = new_lines.len();
                let added_entries: Vec<(usize, String)> = {
                    let insert_start = if let Some(block) = self.block_stack.last() {
                        block.lines.len()
                    } else if let Some(ref file) = self.file {
                        file.lines.len()
                    } else {
                        0
                    };
                    if let Some(ref mut block) = self.block_stack.last_mut() {
                        block.lines.extend(new_lines);
                        block.reindex();
                        let end = (insert_start + new_line_count).min(block.lines.len());
                        (insert_start..end)
                            .map(|i| (block.lines[i].line_num, block.lines[i].content.clone()))
                            .collect()
                    } else if let Some(ref mut file) = self.file {
                        file.lines.extend(new_lines);
                        reindex_file(file);
                        let end = (insert_start + new_line_count).min(file.lines.len());
                        (insert_start..end)
                            .map(|i| (file.lines[i].line_num, file.lines[i].content.clone()))
                            .collect()
                    } else {
                        Vec::new()
                    }
                };
                for (line_num, content) in added_entries {
                    self.diff_lines.push(DiffLine {
                        kind: DiffLineKind::Added,
                        line_number: Some(line_num),
                        content,
                    });
                }
            }
            NewPosition::Normal => {
                let (base_taps, insert_pos) = {
                    let block = self.block_stack.last_mut().ok_or(
                        NEditError::Engine(crate::error::EngineError::MissingLocationForNew),
                    )?;
                    if block.matched_line_count == 0 {
                        // 空 Location — 在 block 开头插入
                        (0, 0)
                    } else {
                        let insert_after_index = block.matched_line_count.saturating_sub(1);
                        let base_taps = block.lines.get(insert_after_index).map(|l| l.taps).unwrap_or(0);
                        (base_taps, insert_after_index + 1)
                    }
                };

                let new_lines = build_new_lines(base_taps, content);
                let new_line_count = new_lines.len();

                let added_entries: Vec<(usize, String)> = {
                    let block = self.block_stack.last_mut().ok_or(
                        NEditError::Engine(crate::error::EngineError::MissingLocationForNew),
                    )?;
                    if insert_pos >= block.lines.len() {
                        block.lines.extend(new_lines);
                    } else {
                        let tail = block.lines.split_off(insert_pos);
                        block.lines.extend(new_lines);
                        block.lines.extend(tail);
                    }
                    block.reindex();
                    let end = (insert_pos + new_line_count).min(block.lines.len());
                    (insert_pos..end)
                        .map(|i| (block.lines[i].line_num, block.lines[i].content.clone()))
                        .collect()
                };
                for (line_num, content) in added_entries {
                    self.diff_lines.push(DiffLine {
                        kind: DiffLineKind::Added,
                        line_number: Some(line_num),
                        content,
                    });
                }
            }
        }

        Ok(())
    }

    /// 执行 Delete 命令：在 ContentBlock 中删除匹配内容
    fn execute_delete(
        &mut self,
        content: Option<&crate::model::DeleteContent>,
    ) -> Result<(), NEditError> {
        let del_content = content.ok_or(NEditError::Engine(
            crate::error::EngineError::MissingLocationForNew,
        ))?;

        let block = self.block_stack.last_mut().ok_or(
            NEditError::Engine(crate::error::EngineError::MissingLocationForNew),
        )?;

        let find_result = find_delete_match(block, del_content);
        match find_result {
            Some((start_idx, end_idx)) => {
                // 检查 Delete 匹配是否紧邻 Location 的最后一行
                // 空 Location（matched_line_count == 0）不检查邻接
                if block.matched_line_count > 0 {
                    let location_last_idx = block.matched_line_count.saturating_sub(1);
                    if start_idx > location_last_idx {
                    // 检查 Location 最后一行到 Delete 首行之间是否有非空行
                    let gap_non_empty: Vec<_> = block.lines
                        [location_last_idx + 1..start_idx]
                        .iter()
                        .filter(|l| !l.content.trim().is_empty())
                        .collect();
                    if !gap_non_empty.is_empty() {
                        let loc_last = &block.lines[location_last_idx].content;
                        let del_first = &block.lines[start_idx].content;
                        return Err(NEditError::Match(
                            crate::error::MatchError::DeleteNotAdjacent {
                                location_last_line: loc_last.clone(),
                                delete_first_line: del_first.clone(),
                                gap_lines: gap_non_empty.len(),
                            },
                        ));
                    }
                    }
                }
                // Record deleted lines before removal
                for idx in start_idx..=end_idx {
                    let line = &block.lines[idx];
                    self.diff_lines.push(DiffLine {
                        kind: DiffLineKind::Deleted,
                        line_number: Some(line.line_num),
                        content: line.content.clone(),
                    });
                }
                block.lines.drain(start_idx..=end_idx);
                block.reindex();
                Ok(())
            }
            None => {
                let first_del_line = del_content
                    .lines
                    .first()
                    .map(|l| l.content.as_str())
                    .unwrap_or("");
                let block_snippet = block
                    .lines
                    .iter()
                    .take(10)
                    .map(|l| l.content.as_str())
                    .collect::<Vec<&str>>()
                    .join("\n");
                Err(NEditError::Match(
                    crate::error::MatchError::DeleteMatchFailed {
                        delete_content: first_del_line.to_string(),
                        block_snippet,
                    },
                ))
            }
        }
    }
}

/// 将 ContentBlock 的修改应用到 FileContent 中对应位置
///
/// 将 file 中从 block.start_line 开始到文件末尾的部分
/// 替换为 block 的当前行。这样 New 插入和 Delete 删除
/// 都能正确地反映到最终文件中。
fn apply_block_to_file(file: &mut FileContent, block: &ContentBlock) {
    let start_index = block.start_line.saturating_sub(1);

    // 截断 file 到 start_index（移除从 start_index 开始的原有行）
    file.lines.truncate(start_index);

    // 追加 block 的当前行
    for block_line in &block.lines {
        file.lines.push(Line {
            line_num: block_line.line_num,
            taps: block_line.taps,
            diff_taps: block_line.diff_taps,
            content: block_line.content.clone(),
            stripped_content: block_line.stripped_content.clone(),
        });
    }

    // 重建行号索引
    reindex_file(file);
}

/// 从 NewContent 构建 Line 列表
///
/// 使用 NewContent 中各行的 diff_taps 作为绝对缩进量计算实际 taps，
/// 生成 Line 结构用于插入。
fn build_new_lines(_base_taps: usize, content: &crate::model::NewContent) -> Vec<Line> {
    content
        .lines
        .iter()
        .map(|new_line| {
            let actual_taps = if new_line.is_raw {
                crate::model::count_leading_spaces(&new_line.content)
            } else {
                new_line.diff_taps
            };
            let indented_content = if new_line.is_raw {
                new_line.content.clone()
            } else if actual_taps > 0 {
                format!(
                    "{:indent$}{}",
                    "",
                    new_line.content,
                    indent = actual_taps
                )
            } else {
                new_line.content.clone()
            };
            let stripped = crate::model::stripped_content(&indented_content);
            Line {
                line_num: 0,
                taps: actual_taps,
                diff_taps: 0,
                content: indented_content,
                stripped_content: stripped,
            }
        })
        .collect()
}

/// 重新为 FileContent 的所有行分配行号和重算 diff_taps
fn reindex_file(file: &mut FileContent) {
    let base_taps = file.lines.first().map(|l| l.taps).unwrap_or(0);
    for (index, line) in file.lines.iter_mut().enumerate() {
        line.line_num = index + 1;
        line.diff_taps = line.taps.saturating_sub(base_taps);
    }
    // Rebuild first_line_index
    let mut index: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (i, line) in file.lines.iter().enumerate() {
        index
            .entry(line.stripped_content.clone())
            .or_default()
            .push(i);
    }
    file.first_line_index = index;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LocationContent, LocationLine, NewContent, NewLine, DeleteContent, DeleteLine};
    use crate::parser::{Command, NewPosition, OffTarget};

    /// 辅助结构：持有临时文件及其路径，确保文件在测试期间存活
    struct TempFile {
        path: String,
        // _file 在结构体存活期间保持文件存在（Drop 时不会删除
        // 因为 tempfile 在 drop 时会删除，但我们使用 persist 转为永久文件）
        _temp_dir: tempfile::TempDir,
    }

    /// 辅助函数：创建测试用的临时文件并返回包装结构
    fn create_temp_file(content: &str) -> TempFile {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test_file.txt");
        let path_str = file_path.to_str().unwrap().to_string();
        std::fs::write(&file_path, content).unwrap();
        TempFile {
            path: path_str,
            _temp_dir: dir,
        }
    }

    /// 辅助函数：构建简单的 LocationContent
    fn make_location_content(lines: &[&str]) -> LocationContent {
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

    /// 辅助函数：构建简单的 NewContent（diff_taps 为绝对缩进量）
    fn make_new_content(lines: &[&str]) -> NewContent {
        let new_lines: Vec<NewLine> = lines
            .iter()
            .map(|content| {
                let line_taps = crate::model::count_leading_spaces(content);
                let stripped_content = content[line_taps..].to_string();
                NewLine {
                    diff_taps: line_taps,
                    content: stripped_content,
                    is_raw: false,
                }
            })
            .collect();
        NewContent { lines: new_lines }
    }

    /// 辅助函数：构建简单的 DeleteContent
    fn make_delete_content(lines: &[&str]) -> DeleteContent {
        let del_lines: Vec<DeleteLine> = lines
            .iter()
            .map(|content| DeleteLine {
                content: content.to_string(),
                is_raw: false,
            })
            .collect();
        DeleteContent { lines: del_lines }
    }

    // ============================================================
    // Engine 基本生命周期测试
    // ============================================================

    #[test]
    fn test_engine_open_reads_file() {
        let tmp = create_temp_file("line one\nline two\nline three\n");
        let commands = vec![Command::Open {
            file_path: tmp.path.clone(),
        }];

        let mut engine = Engine::new();
        let result = engine.execute(commands);
        assert!(result.is_ok(), "Unexpected error: {:?}", result.err());

        let file = engine.file.as_ref().unwrap();
        assert_eq!(file.lines.len(), 3);
        assert_eq!(file.lines[0].content, "line one");
    }

    #[test]
    fn test_engine_open_nonexistent_file_errors() {
        let commands = vec![Command::Open {
            file_path: "/nonexistent/path.xyz".to_string(),
        }];

        let mut engine = Engine::new();
        let result = engine.execute(commands);
        assert!(result.is_err());
    }

    #[test]
    fn test_engine_location_pushes_to_block_stack() {
        let tmp = create_temp_file("fn main() {\n    let x = 1;\n}\n");
        let mut engine = Engine::new();

        // 直接调用方法测试中间状态，避免隐式 Off 干扰
        engine.execute_open(&tmp.path).unwrap();
        engine
            .execute_location(&make_location_content(&["fn main() {"]))
            .unwrap();

        // 在隐式 Off 之前，block_stack 应有 1 个元素
        assert_eq!(engine.block_stack.len(), 1);
        let block = &engine.block_stack[0];
        assert_eq!(block.start_line, 1);
    }

    #[test]
    fn test_engine_location_no_match_errors() {
        let tmp = create_temp_file("fn foo() {}\nfn bar() {}\n");
        let commands = vec![
            Command::Open {
                file_path: tmp.path.clone(),
            },
            Command::Location {
                block: false,
                content: make_location_content(&["fn nonexistent() {}"]),
            },
        ];

        let mut engine = Engine::new();
        let result = engine.execute(commands);
        assert!(result.is_err());
    }

    #[test]
    fn test_engine_off_location_pops_stack() {
        let tmp = create_temp_file("fn main() {\n    let x = 1;\n}\n");
        let commands = vec![
            Command::Open {
                file_path: tmp.path.clone(),
            },
            Command::Location {
                block: false,
                content: make_location_content(&["fn main() {"]),
            },
            Command::Off {
                target: OffTarget::Location,
            },
        ];

        let mut engine = Engine::new();
        let result = engine.execute(commands);
        assert!(result.is_ok(), "Unexpected error: {:?}", result.err());

        assert_eq!(engine.block_stack.len(), 0);
    }

    #[test]
    fn test_engine_off_open_writes_back_to_file() {
        let tmp = create_temp_file("original content\n");
        let commands = vec![
            Command::Open {
                file_path: tmp.path.clone(),
            },
            Command::Off {
                target: OffTarget::Open,
            },
        ];

        let mut engine = Engine::new();
        let result = engine.execute(commands);
        assert!(result.is_ok(), "Unexpected error: {:?}", result.err());

        let content = std::fs::read_to_string(&tmp.path).unwrap();
        assert_eq!(content, "original content\n");
    }

    // ============================================================
    // 隐式 Off:Open 测试
    // ============================================================

    #[test]
    fn test_engine_implicit_off_open_writes_back() {
        let tmp = create_temp_file("content\n");
        let commands = vec![Command::Open {
            file_path: tmp.path.clone(),
        }];

        let mut engine = Engine::new();
        let result = engine.execute(commands);
        assert!(result.is_ok(), "Unexpected error: {:?}", result.err());

        let content = std::fs::read_to_string(&tmp.path).unwrap();
        assert_eq!(content, "content\n");
    }

    // ============================================================
    // Open-Location-Off 完整流程测试
    // ============================================================

    #[test]
    fn test_engine_full_open_location_off_flow() {
        let tmp = create_temp_file(
            "// header\nfn process() {\n    do_work();\n}\n\nfn main() {\n    process();\n}\n",
        );
        let commands = vec![
            Command::Open {
                file_path: tmp.path.clone(),
            },
            Command::Location {
                block: false,
                content: make_location_content(&["fn main() {"]),
            },
            Command::Off {
                target: OffTarget::Open,
            },
        ];

        let mut engine = Engine::new();
        let result = engine.execute(commands);
        assert!(result.is_ok(), "Unexpected error: {:?}", result.err());

        let content = std::fs::read_to_string(&tmp.path).unwrap();
        assert!(content.contains("fn main()"));
        assert!(content.contains("fn process()"));
    }

    // ============================================================
    // execute_new — 插入测试
    // ============================================================

    #[test]
    fn test_new_insert_normal() {
        let tmp = create_temp_file("fn main() {\n    println!(\"hi\");\n}\n");
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();
        engine
            .execute_location(&make_location_content(&["fn main() {"]))
            .unwrap();

        let new_content = make_new_content(&["    let x = 1;"]);
        engine
            .execute_new(&NewPosition::Normal, &new_content)
            .unwrap();

        let block = engine.block_stack.last().unwrap();
        // Block should have: fn main(), let x = 1, println!("hi"), }
        assert_eq!(block.lines.len(), 4);
        assert_eq!(block.lines[1].content, "    let x = 1;");
    }

    #[test]
    fn test_new_insert_start() {
        let tmp = create_temp_file("fn main() {\n    println!(\"hi\");\n}\n");
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();

        let new_content = make_new_content(&["// SPDX-License-Identifier: MIT"]);
        engine
            .execute_new(&NewPosition::Start, &new_content)
            .unwrap();

        let file = engine.file.as_ref().unwrap();
        assert_eq!(file.lines.len(), 4);
        assert_eq!(file.lines[0].content, "// SPDX-License-Identifier: MIT");
    }

    #[test]
    fn test_new_insert_end() {
        let tmp = create_temp_file("fn main() {\n    println!(\"hi\");\n}\n");
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();

        let new_content = make_new_content(&["// EOF"]);
        engine
            .execute_new(&NewPosition::End, &new_content)
            .unwrap();

        let file = engine.file.as_ref().unwrap();
        assert_eq!(file.lines.len(), 4);
        assert_eq!(file.lines[3].content, "// EOF");
    }

    #[test]
    fn test_new_insert_preserves_indentation() {
        let tmp = create_temp_file("fn main() {\n    println!(\"hi\");\n}\n");
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();
        engine
            .execute_location(&make_location_content(&["fn main() {"]))
            .unwrap();

        let new_content = make_new_content(&["    let a = 1;", "        let b = 2;"]);
        engine
            .execute_new(&NewPosition::Normal, &new_content)
            .unwrap();

        let block = engine.block_stack.last().unwrap();
        assert_eq!(block.lines[1].content, "    let a = 1;");
        assert_eq!(block.lines[2].content, "        let b = 2;");
    }

    // ============================================================
    // execute_delete — 删除测试
    // ============================================================

    #[test]
    fn test_delete_removes_matching_lines() {
        let tmp = create_temp_file(
            "fn main() {\n    let x = 1;\n    let y = 2;\n    println!(\"{}\", x);\n}\n",
        );
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();
        engine
            .execute_location(&make_location_content(&["fn main() {"]))
            .unwrap();

        let del_content = make_delete_content(&["    let x = 1;", "    let y = 2;"]);
        engine.execute_delete(Some(&del_content)).unwrap();

        let block = engine.block_stack.last().unwrap();
        assert_eq!(block.lines.len(), 3); // fn main(), println!, }
        assert_eq!(block.lines[0].content, "fn main() {");
        assert_eq!(block.lines[1].content, "    println!(\"{}\", x);");
        assert_eq!(block.lines[2].content, "}");
    }

    #[test]
    fn test_delete_requires_continuous_match() {
        let tmp = create_temp_file(
            "fn main() {\n    let x = 1;\n    let y = 2;\n    println!(\"{}\", x);\n}\n",
        );
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();
        engine
            .execute_location(&make_location_content(&["fn main() {"]))
            .unwrap();

        // Delete content that matches non-contiguous lines should fail
        let del_content = make_delete_content(&["    let x = 1;", "    println!(\"{}\", x);"]);
        let result = engine.execute_delete(Some(&del_content));
        assert!(result.is_err());
    }

    #[test]
    fn test_delete_content_not_found() {
        let tmp = create_temp_file("fn main() {\n    println!(\"hi\");\n}\n");
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();
        engine
            .execute_location(&make_location_content(&["fn main() {"]))
            .unwrap();

        let del_content = make_delete_content(&["    nonexistent content"]);
        let result = engine.execute_delete(Some(&del_content));
        assert!(result.is_err());
    }

    #[test]
    fn test_new_insert_normal_without_location_errors() {
        let tmp = create_temp_file("fn main() {\n    println!(\"hi\");\n}\n");
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();

        // No Location executed, so block_stack is empty
        let new_content = make_new_content(&["    let x = 1;"]);
        let result = engine.execute_new(&NewPosition::Normal, &new_content);
        assert!(result.is_err());
    }

    #[test]
    fn test_full_new_delete_pipeline() {
        let tmp = create_temp_file("fn main() {\n    old_code();\n}\n");
        let commands = vec![
            Command::Open {
                file_path: tmp.path.clone(),
            },
            Command::Location {
                block: false,
                content: make_location_content(&["fn main() {"]),
            },
            Command::Delete {
                block: false,
                content: Some(make_delete_content(&["    old_code();"])),
            },
            Command::New {
                position: NewPosition::Normal,
                content: make_new_content(&["    let x = 1;"]),
            },
            Command::Off {
                target: OffTarget::Open,
            },
        ];

        let mut engine = Engine::new();
        let result = engine.execute(commands);
        assert!(result.is_ok(), "Unexpected error: {:?}", result.err());

        let content = std::fs::read_to_string(&tmp.path).unwrap();
        assert!(content.contains("    let x = 1;"));
        assert!(!content.contains("old_code"));
    }

    // ============================================================
    // diff_lines 输出测试
    // ============================================================

    #[test]
    fn test_new_produces_added_diff_lines() {
        let tmp = create_temp_file("fn main() {\n    println!(\"hi\");\n}\n");
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();
        engine
            .execute_location(&make_location_content(&["fn main() {"]))
            .unwrap();
        engine
            .execute_new(
                &NewPosition::Normal,
                &make_new_content(&["    let x = 1;"]),
            )
            .unwrap();

        assert_eq!(engine.diff_lines.len(), 1);
        assert_eq!(engine.diff_lines[0].kind, DiffLineKind::Added);
        assert_eq!(engine.diff_lines[0].content, "    let x = 1;");
        assert!(engine.diff_lines[0].line_number.is_some());
    }

    #[test]
    fn test_delete_produces_deleted_diff_lines() {
        let tmp = create_temp_file(
            "fn main() {\n    let x = 1;\n    let y = 2;\n}\n",
        );
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();
        engine
            .execute_location(&make_location_content(&["fn main() {"]))
            .unwrap();
        engine
            .execute_delete(Some(&make_delete_content(&["    let x = 1;"])))
            .unwrap();

        assert_eq!(engine.diff_lines.len(), 1);
        assert_eq!(engine.diff_lines[0].kind, DiffLineKind::Deleted);
        assert_eq!(engine.diff_lines[0].content, "    let x = 1;");
        assert!(engine.diff_lines[0].line_number.is_some());
    }

    #[test]
    fn test_new_delete_produces_mixed_diff_lines() {
        let tmp = create_temp_file("fn main() {\n    old_code();\n}\n");
        let commands = vec![
            Command::Open {
                file_path: tmp.path.clone(),
            },
            Command::Location {
                block: false,
                content: make_location_content(&["fn main() {"]),
            },
            Command::Delete {
                block: false,
                content: Some(make_delete_content(&["    old_code();"])),
            },
            Command::New {
                position: NewPosition::Normal,
                content: make_new_content(&["    let x = 1;"]),
            },
            Command::Off {
                target: OffTarget::Open,
            },
        ];

        let mut engine = Engine::new();
        let result = engine.execute(commands);
        assert!(result.is_ok(), "Unexpected error: {:?}", result.err());

        assert_eq!(engine.diff_lines.len(), 2);
        // First comes the Delete (Deleted)
        assert_eq!(engine.diff_lines[0].kind, DiffLineKind::Deleted);
        assert_eq!(engine.diff_lines[0].content, "    old_code();");
        // Then comes the New (Added)
        assert_eq!(engine.diff_lines[1].kind, DiffLineKind::Added);
        assert_eq!(engine.diff_lines[1].content, "    let x = 1;");
    }

    #[test]
    fn test_new_start_produces_diff_lines() {
        let tmp = create_temp_file("fn main() {\n}\n");
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();
        engine
            .execute_new(
                &NewPosition::Start,
                &make_new_content(&["// SPDX-License-Identifier: MIT"]),
            )
            .unwrap();

        assert_eq!(engine.diff_lines.len(), 1);
        assert_eq!(engine.diff_lines[0].kind, DiffLineKind::Added);
        assert_eq!(engine.diff_lines[0].content, "// SPDX-License-Identifier: MIT");
    }

    #[test]
    fn test_new_end_produces_diff_lines() {
        let tmp = create_temp_file("fn main() {\n}\n");
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();
        engine
            .execute_new(&NewPosition::End, &make_new_content(&["// EOF"]))
            .unwrap();

        assert_eq!(engine.diff_lines.len(), 1);
        assert_eq!(engine.diff_lines[0].kind, DiffLineKind::Added);
        assert_eq!(engine.diff_lines[0].content, "// EOF");
    }
}
