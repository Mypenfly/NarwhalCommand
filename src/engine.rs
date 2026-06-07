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
use crate::model::BLOCK_SNIPPET_MAX_LINES;
use crate::model::{ContentBlock, FileContent, Line, LineNumber, MatchInfo, SearchScope};
use crate::output::{DiffLine, DiffLineKind};
use crate::parser::{Command, OffTarget};
use std::collections::HashMap;

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

// ============================================================
// Delete 匹配辅助函数
// ============================================================

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

        if lines_continuously_match(block, del_lines, start_idx) {
            return Some((start_idx, start_idx + del_lines.len() - 1));
        }
    }

    None
}

/// 检查从 start_idx 开始，block 的行是否与 delete_content 所有行连续匹配
fn lines_continuously_match(
    block: &ContentBlock,
    del_lines: &[crate::model::DeleteLine],
    start_idx: usize,
) -> bool {
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
            return false;
        }
        if block_stripped != del_stripped {
            return false;
        }
    }
    true
}

/// 检查 Delete 匹配位置是否与 Location 最后一行的位置紧邻
///
/// 若之间隔了非空行，说明 Delete 可能删错了位置。
fn check_delete_adjacency(block: &ContentBlock, start_idx: usize) -> Result<(), NEditError> {
    if let MatchInfo::Location { matched_line_count } = &block.match_info {
        if *matched_line_count == 0 {
            return Ok(());
        }
        let location_last_idx = matched_line_count.saturating_sub(1);
        if start_idx <= location_last_idx {
            return Ok(());
        }
        let gap_non_empty: Vec<_> = block.lines[location_last_idx + 1..start_idx]
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
    Ok(())
}

/// 记录被删除的行到 diff_lines
fn record_deleted_lines(block: &ContentBlock, start_idx: usize, end_idx: usize) -> Vec<DiffLine> {
    block.lines[start_idx..=end_idx]
        .iter()
        .map(|line| DiffLine {
            kind: DiffLineKind::Deleted,
            line_number: Some(line.line_num),
            content: line.content.clone(),
        })
        .collect()
}

/// 构建 Delete 未找到匹配时的错误信息
fn delete_not_found_error(
    del_content: &crate::model::DeleteContent,
    block: &ContentBlock,
) -> NEditError {
    let first_del_line = del_content
        .lines
        .first()
        .map(|l| l.content.as_str())
        .unwrap_or("");
    let block_snippet = block
        .lines
        .iter()
        .take(BLOCK_SNIPPET_MAX_LINES)
        .map(|l| l.content.as_str())
        .collect::<Vec<&str>>()
        .join("\n");
    NEditError::Match(crate::error::MatchError::DeleteMatchFailed {
        delete_content: first_del_line.to_string(),
        block_snippet,
    })
}

// ============================================================
// Engine 实现
// ============================================================

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
                Command::Location { block, content } => {
                    self.execute_location(&content, block)?;
                }
                Command::New { position, content } => {
                    self.execute_new(&position, &content)?;
                }
                Command::Delete { block, content } => {
                    self.execute_delete(block, content.as_ref())?;
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
        block: bool,
    ) -> Result<(), NEditError> {
        let search_scope = self.get_search_scope()?;
        let content_block =
            LocationMatcher::find_unique_block(&search_scope, location_content, block)
                .map_err(NEditError::Match)?;
        self.block_stack.push(content_block);
        Ok(())
    }

    /// 执行 Off 命令：根据目标弹出栈或写回文件
    fn execute_off(&mut self, target: &OffTarget) -> Result<(), NEditError> {
        match target {
            OffTarget::Location | OffTarget::New => {
                let popped_block = self.block_stack.pop().ok_or(NEditError::Engine(
                    crate::error::EngineError::BlockStackEmpty,
                ))?;
                self.write_back_to_parent(popped_block)?;
            }
            OffTarget::Open => {
                self.write_back_to_file()?;
            }
        }
        Ok(())
    }

    /// 执行 Delete:Block — 删除整个 ContentBlock
    ///
    /// 移除 block 中所有行，仅保留首行的行号（避免在文件中产生空行）。
    /// 删除的行会被记录到 diff_lines 中。
    fn execute_delete_block(&mut self) -> Result<(), NEditError> {
        let block = self.block_stack.last_mut().ok_or(NEditError::Engine(
            crate::error::EngineError::MissingLocationForNew,
        ))?;

        // 记录所有被删除的行到 diff_lines
        for line in &block.lines {
            self.diff_lines.push(DiffLine {
                kind: DiffLineKind::Deleted,
                line_number: Some(line.line_num),
                content: line.content.clone(),
            });
        }

        // 保留首行的行号，清空所有行
        let first_line_num = block.start_line;
        block.lines.clear();
        block.lines.push(Line {
            line_num: first_line_num,
            taps: 0,
            diff_taps: 0,
            content: String::new(),
            stripped_content: String::new(),
        });
        block.reindex();
        Ok(())
    }

    /// 获取当前 Location 的搜索范围
    ///
    /// 若 block_stack 为空（顶层 Location），搜索范围为完整 FileContent。
    /// 若 block_stack 非空（嵌套 Location），搜索范围为栈顶 ContentBlock。
    fn get_search_scope(&self) -> Result<SearchScope<'_>, NEditError> {
        if let Some(block) = self.block_stack.last() {
            Ok(SearchScope::Block(block))
        } else {
            self.file
                .as_ref()
                .map(SearchScope::File)
                .ok_or(NEditError::File(FileError::NotFound {
                    path: "(no file opened)".to_string(),
                }))
        }
    }

    /// 将弹出的 ContentBlock 写回到父级
    ///
    /// Phase 4 嵌套：若 block_stack 仍有剩余 Block，将 popped 内容写回父级 Block；
    /// 否则写回 FileContent。
    fn write_back_to_parent(&mut self, block: ContentBlock) -> Result<(), NEditError> {
        if let Some(parent) = self.block_stack.last_mut() {
            apply_block_to_parent(&block, parent);
        } else if let Some(ref mut file) = self.file {
            apply_block_to_file(file, &block);
        }
        Ok(())
    }

    /// 将所有修改最终写回磁盘文件
    ///
    /// Phase 4 嵌套：从内到外逐层弹出并写回父级 Block，
    /// 最外层写回 FileContent 后落盘。
    fn write_back_to_file(&mut self) -> Result<(), NEditError> {
        while let Some(block) = self.block_stack.pop() {
            if let Some(parent) = self.block_stack.last_mut() {
                apply_block_to_parent(&block, parent);
            } else if let Some(ref mut file) = self.file {
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
            NewPosition::Start => self.execute_new_start(content),
            NewPosition::End => self.execute_new_end(content),
            NewPosition::Normal => self.execute_new_normal(content),
        }
    }

    /// 在文件/Block 开头插入新内容
    fn execute_new_start(&mut self, content: &crate::model::NewContent) -> Result<(), NEditError> {
        let new_lines = build_new_lines(content);
        let new_line_count = new_lines.len();
        let added_entries = {
            if let Some(ref mut block) = self.block_stack.last_mut() {
                let mut combined = new_lines;
                combined.append(&mut block.lines);
                block.lines = combined;
                block.reindex();
                collect_new_line_info(block, 0, new_line_count)
            } else if let Some(ref mut file) = self.file {
                let mut combined = new_lines;
                combined.append(&mut file.lines);
                file.lines = combined;
                reindex_file(file);
                collect_new_file_line_info(file, 0, new_line_count)
            } else {
                Vec::new()
            }
        };
        self.record_added_lines(added_entries);
        Ok(())
    }

    /// 在文件/Block 末尾插入新内容
    fn execute_new_end(&mut self, content: &crate::model::NewContent) -> Result<(), NEditError> {
        let new_lines = build_new_lines(content);
        let new_line_count = new_lines.len();
        let added_entries = {
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
                collect_new_line_info(block, insert_start, new_line_count)
            } else if let Some(ref mut file) = self.file {
                file.lines.extend(new_lines);
                reindex_file(file);
                collect_new_file_line_info(file, insert_start, new_line_count)
            } else {
                Vec::new()
            }
        };
        self.record_added_lines(added_entries);
        Ok(())
    }

    /// 在 Location 匹配位置之后插入新内容
    fn execute_new_normal(&mut self, content: &crate::model::NewContent) -> Result<(), NEditError> {
        let insert_pos = {
            let block = self.block_stack.last_mut().ok_or(NEditError::Engine(
                crate::error::EngineError::MissingLocationForNew,
            ))?;
            match &block.match_info {
                MatchInfo::Empty => block.lines.len(),
                MatchInfo::Location { matched_line_count } => *matched_line_count,
                MatchInfo::DeleteAt { position } => *position,
            }
        };

        let new_lines = build_new_lines(content);
        let new_line_count = new_lines.len();

        let added_entries = {
            let block = self.block_stack.last_mut().ok_or(NEditError::Engine(
                crate::error::EngineError::MissingLocationForNew,
            ))?;
            if insert_pos >= block.lines.len() {
                block.lines.extend(new_lines);
            } else {
                let tail = block.lines.split_off(insert_pos);
                block.lines.extend(new_lines);
                block.lines.extend(tail);
            }
            block.reindex();
            collect_new_line_info(block, insert_pos, new_line_count)
        };
        self.record_added_lines(added_entries);
        Ok(())
    }

    /// 记录新增行到 diff_lines
    fn record_added_lines(&mut self, entries: Vec<(usize, String)>) {
        for (line_num, content) in entries {
            self.diff_lines.push(DiffLine {
                kind: DiffLineKind::Added,
                line_number: Some(LineNumber::new(line_num)),
                content,
            });
        }
    }

    /// 执行 Delete 命令：在 ContentBlock 中删除匹配内容
    ///
    /// 若 `block` 为 true（Delete:Block），删除整个 ContentBlock.
    /// 否则在 block 内逐行匹配并删除。
    fn execute_delete(
        &mut self,
        block: bool,
        content: Option<&crate::model::DeleteContent>,
    ) -> Result<(), NEditError> {
        if block {
            return self.execute_delete_block();
        }

        let del_content = content.ok_or(NEditError::Engine(
            crate::error::EngineError::MissingLocationForNew,
        ))?;

        let current_block = self.block_stack.last_mut().ok_or(NEditError::Engine(
            crate::error::EngineError::MissingLocationForNew,
        ))?;

        let (start_idx, end_idx) = match find_delete_match(current_block, del_content) {
            Some(range) => range,
            None => return Err(delete_not_found_error(del_content, current_block)),
        };

        // 检查 Delete 匹配是否紧邻 Location 的最后一行
        check_delete_adjacency(current_block, start_idx)?;

        // 记录删除行到 diff_lines
        let deleted = record_deleted_lines(current_block, start_idx, end_idx);
        self.diff_lines.extend(deleted);

        // 执行删除并更新定位信息
        current_block.lines.drain(start_idx..=end_idx);
        current_block.match_info = MatchInfo::DeleteAt {
            position: start_idx,
        };
        current_block.reindex();
        Ok(())
    }
}

// ============================================================
// Block / File 写回辅助函数
// ============================================================

/// 将 ContentBlock 的修改应用到 FileContent 中对应位置
///
/// 使用 block.start_line 和 block.end_line 确定原始范围，
/// 将其替换为 block 的当前行。
fn apply_block_to_file(file: &mut FileContent, block: &ContentBlock) {
    let start_index = block.start_line.to_index();
    let end_index = block.end_line.to_index();

    let count = end_index.saturating_sub(start_index) + 1;
    let count = count.min(file.lines.len().saturating_sub(start_index));

    let new_lines: Vec<Line> = block
        .lines
        .iter()
        .map(|line| Line {
            line_num: line.line_num,
            taps: line.taps,
            diff_taps: line.diff_taps,
            content: line.content.clone(),
            stripped_content: line.stripped_content.clone(),
        })
        .collect();

    file.lines
        .splice(start_index..start_index + count, new_lines);

    reindex_file(file);
}

/// 将内层 ContentBlock 的修改应用到父级 ContentBlock 中
///
/// 用于嵌套 Location 场景（Phase 4）：内层 Block（inner）弹出后，
/// 通过 start_line 差值计算偏移量，将内层修改合并回父级 Block（outer）。
fn apply_block_to_parent(inner: &ContentBlock, outer: &mut ContentBlock) {
    let start_offset = inner
        .start_line
        .to_index()
        .saturating_sub(outer.start_line.to_index());
    let end_offset = inner
        .end_line
        .to_index()
        .saturating_sub(outer.start_line.to_index());

    let start_offset = start_offset.min(outer.lines.len());
    let end_offset = end_offset.min(outer.lines.len().saturating_sub(1));

    let count = if end_offset >= start_offset {
        end_offset - start_offset + 1
    } else {
        0
    };

    let new_lines: Vec<Line> = inner
        .lines
        .iter()
        .map(|line| Line {
            line_num: line.line_num,
            taps: line.taps,
            diff_taps: line.diff_taps,
            content: line.content.clone(),
            stripped_content: line.stripped_content.clone(),
        })
        .collect();

    outer
        .lines
        .splice(start_offset..start_offset + count, new_lines);
    outer.reindex();
}

/// 从 NewContent 构建 Line 列表
///
/// 使用 NewContent 中各行的 diff_taps 作为绝对缩进量计算实际 taps，
/// 生成 Line 结构用于插入。line_num 设为占位值，调用方通过 reindex 重算。
fn build_new_lines(content: &crate::model::NewContent) -> Vec<Line> {
    const PLACEHOLDER_LINE_NUM: LineNumber = LineNumber::new(1);

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
                format!("{:indent$}{}", "", new_line.content, indent = actual_taps)
            } else {
                new_line.content.clone()
            };
            let stripped = crate::model::stripped_content(&indented_content);
            Line {
                line_num: PLACEHOLDER_LINE_NUM,
                taps: actual_taps,
                diff_taps: 0,
                content: indented_content,
                stripped_content: stripped,
            }
        })
        .collect()
}

/// 从 ContentBlock 中收集新增行的 (line_num, content) 信息
fn collect_new_line_info(
    block: &ContentBlock,
    insert_pos: usize,
    new_line_count: usize,
) -> Vec<(usize, String)> {
    let end = (insert_pos + new_line_count).min(block.lines.len());
    (insert_pos..end)
        .map(|i| {
            (
                block.lines[i].line_num.to_usize(),
                block.lines[i].content.clone(),
            )
        })
        .collect()
}

/// 从 FileContent 中收集新增行的 (line_num, content) 信息
fn collect_new_file_line_info(
    file: &FileContent,
    insert_pos: usize,
    new_line_count: usize,
) -> Vec<(usize, String)> {
    let end = (insert_pos + new_line_count).min(file.lines.len());
    (insert_pos..end)
        .map(|i| {
            (
                file.lines[i].line_num.to_usize(),
                file.lines[i].content.clone(),
            )
        })
        .collect()
}

/// 重新为 FileContent 的所有行分配行号和重算 diff_taps，重建首行索引
fn reindex_file(file: &mut FileContent) {
    let base_taps = file.lines.first().map(|l| l.taps).unwrap_or(0);
    for (index, line) in file.lines.iter_mut().enumerate() {
        line.line_num = LineNumber::from_index(index);
        line.diff_taps = line.taps.saturating_sub(base_taps);
    }
    let mut index: HashMap<String, Vec<usize>> = HashMap::new();
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
    use crate::model::{
        DeleteContent, DeleteLine, LocationContent, LocationLine, NewContent, NewLine,
    };
    use crate::parser::{Command, NewPosition, OffTarget};

    /// 辅助结构：持有临时文件及其路径，确保文件在测试期间存活
    struct TempFile {
        path: String,
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

        engine.execute_open(&tmp.path).unwrap();
        engine
            .execute_location(&make_location_content(&["fn main() {"]), false)
            .unwrap();

        assert_eq!(engine.block_stack.len(), 1);
        let current_block = &engine.block_stack[0];
        assert_eq!(current_block.start_line, 1);
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
            .execute_location(&make_location_content(&["fn main() {"]), false)
            .unwrap();

        let new_content = make_new_content(&["    let x = 1;"]);
        engine
            .execute_new(&NewPosition::Normal, &new_content)
            .unwrap();

        let current_block = engine.block_stack.last().unwrap();
        assert_eq!(current_block.lines.len(), 4);
        assert_eq!(current_block.lines[1].content, "    let x = 1;");
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
        engine.execute_new(&NewPosition::End, &new_content).unwrap();

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
            .execute_location(&make_location_content(&["fn main() {"]), false)
            .unwrap();

        let new_content = make_new_content(&["    let a = 1;", "        let b = 2;"]);
        engine
            .execute_new(&NewPosition::Normal, &new_content)
            .unwrap();

        let current_block = engine.block_stack.last().unwrap();
        assert_eq!(current_block.lines[1].content, "    let a = 1;");
        assert_eq!(current_block.lines[2].content, "        let b = 2;");
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
            .execute_location(&make_location_content(&["fn main() {"]), false)
            .unwrap();

        let del_content = make_delete_content(&["    let x = 1;", "    let y = 2;"]);
        engine.execute_delete(false, Some(&del_content)).unwrap();
    }

    #[test]
    fn test_delete_requires_continuous_match() {
        let tmp = create_temp_file(
            "fn main() {\n    let x = 1;\n    let y = 2;\n    println!(\"{}\", x);\n}\n",
        );
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();
        engine
            .execute_location(&make_location_content(&["fn main() {"]), false)
            .unwrap();

        let del_content = make_delete_content(&["    let x = 1;", "    println!(\"{}\", x);"]);
        let result = engine.execute_delete(false, Some(&del_content));
        assert!(result.is_err());
    }

    #[test]
    fn test_delete_content_not_found() {
        let tmp = create_temp_file("fn main() {\n    println!(\"hi\");\n}\n");
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();
        engine
            .execute_location(&make_location_content(&["fn main() {"]), false)
            .unwrap();

        let del_content = make_delete_content(&["    nonexistent content"]);
        let result = engine.execute_delete(false, Some(&del_content));
        assert!(result.is_err());
    }

    #[test]
    fn test_new_insert_normal_without_location_errors() {
        let tmp = create_temp_file("fn main() {\n    println!(\"hi\");\n}\n");
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();

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
            .execute_location(&make_location_content(&["fn main() {"]), false)
            .unwrap();
        engine
            .execute_new(&NewPosition::Normal, &make_new_content(&["    let x = 1;"]))
            .unwrap();

        assert_eq!(engine.diff_lines.len(), 1);
        assert_eq!(engine.diff_lines[0].kind, DiffLineKind::Added);
        assert_eq!(engine.diff_lines[0].content, "    let x = 1;");
        assert!(engine.diff_lines[0].line_number.is_some());
    }

    #[test]
    fn test_delete_produces_deleted_diff_lines() {
        let tmp = create_temp_file("fn main() {\n    let x = 1;\n    let y = 2;\n}\n");
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();
        engine
            .execute_location(&make_location_content(&["fn main() {"]), false)
            .unwrap();
        engine
            .execute_delete(false, Some(&make_delete_content(&["    let x = 1;"])))
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
        assert_eq!(engine.diff_lines[0].kind, DiffLineKind::Deleted);
        assert_eq!(engine.diff_lines[0].content, "    old_code();");
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
        assert_eq!(
            engine.diff_lines[0].content,
            "// SPDX-License-Identifier: MIT"
        );
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

    // ============================================================
    // Phase 3: Delete → New 定位修复测试
    // ============================================================

    #[test]
    fn test_empty_location_delete_then_new_replaces_deleted() {
        let tmp = create_temp_file(
            "// header\nfn process() {\n    do_work();\n}\n\nfn main() {\n    old_code();\n}\n",
        );
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();
        engine
            .execute_location(&make_location_content(&[]), false)
            .unwrap();
        engine
            .execute_delete(false, Some(&make_delete_content(&["    old_code();"])))
            .unwrap();
        engine
            .execute_new(
                &NewPosition::Normal,
                &make_new_content(&["    new_code();"]),
            )
            .unwrap();

        let current_block = engine.block_stack.last().unwrap();
        let contents: Vec<&str> = current_block
            .lines
            .iter()
            .map(|l| l.content.as_str())
            .collect();
        assert!(contents.contains(&"    new_code();"));
        assert!(!contents.contains(&"    old_code();"));
        assert!(contents.contains(&"fn main() {"));
    }

    #[test]
    fn test_empty_location_new_without_delete_inserts_at_end() {
        let tmp = create_temp_file("line1\nline2\nline3\n");
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();
        engine
            .execute_location(&make_location_content(&[]), false)
            .unwrap();
        engine
            .execute_new(&NewPosition::Normal, &make_new_content(&["line4"]))
            .unwrap();

        let current_block = engine.block_stack.last().unwrap();
        assert_eq!(current_block.lines.len(), 4);
        assert_eq!(current_block.lines[3].content, "line4");
    }

    #[test]
    fn test_delete_at_start_then_new_inserts_at_start() {
        let tmp = create_temp_file("old first\nsecond\nthird\n");
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();
        engine
            .execute_location(&make_location_content(&[]), false)
            .unwrap();
        engine
            .execute_delete(false, Some(&make_delete_content(&["old first"])))
            .unwrap();
        engine
            .execute_new(&NewPosition::Normal, &make_new_content(&["new first"]))
            .unwrap();

        let current_block = engine.block_stack.last().unwrap();
        assert_eq!(current_block.lines[0].content, "new first");
        assert_eq!(current_block.lines[1].content, "second");
        assert_eq!(current_block.lines.len(), 3);
    }

    #[test]
    fn test_delete_then_new_preserves_indentation() {
        let tmp = create_temp_file("impl Foo {\n    fn bar() {\n        old_inner();\n    }\n}\n");
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();
        engine
            .execute_location(&make_location_content(&[]), false)
            .unwrap();
        engine
            .execute_delete(false, Some(&make_delete_content(&["        old_inner();"])))
            .unwrap();
        engine
            .execute_new(
                &NewPosition::Normal,
                &make_new_content(&["        new_inner();"]),
            )
            .unwrap();

        let current_block = engine.block_stack.last().unwrap();
        assert!(current_block
            .lines
            .iter()
            .any(|l| l.content == "        new_inner();"));
        assert!(!current_block
            .lines
            .iter()
            .any(|l| l.content == "        old_inner();"));
        assert!(current_block
            .lines
            .iter()
            .any(|l| l.content == "    fn bar() {"));
        assert!(current_block.lines.iter().any(|l| l.content == "}"));
    }

    // ============================================================
    // Phase 4: 嵌套 Location 测试
    // ============================================================

    #[test]
    fn test_nested_location_basic() {
        let tmp = create_temp_file(
            "fn outer() {\n    let x = 1;\n    fn inner() {\n        let y = 2;\n    }\n    let z = 3;\n}\n",
        );
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();

        engine
            .execute_location(&make_location_content(&["fn outer() {"]), false)
            .unwrap();

        engine
            .execute_location(&make_location_content(&["    fn inner() {"]), false)
            .unwrap();

        assert_eq!(engine.block_stack.len(), 2);

        let inner_block = &engine.block_stack[1];
        assert_eq!(inner_block.start_line, 3);
        assert!(inner_block.lines.len() >= 3);

        engine.execute_off(&OffTarget::Location).unwrap();
        assert_eq!(engine.block_stack.len(), 1);

        engine.execute_off(&OffTarget::Location).unwrap();
        assert_eq!(engine.block_stack.len(), 0);
    }

    #[test]
    fn test_nested_location_new() {
        let tmp =
            create_temp_file("fn outer() {\n    fn inner() {\n        let a = 1;\n    }\n}\n");
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();

        engine
            .execute_location(&make_location_content(&["fn outer() {"]), false)
            .unwrap();

        engine
            .execute_location(&make_location_content(&["    fn inner() {"]), false)
            .unwrap();

        engine
            .execute_new(
                &NewPosition::Normal,
                &make_new_content(&["        let b = 2;"]),
            )
            .unwrap();

        let inner_block = engine.block_stack.last().unwrap();
        assert!(inner_block
            .lines
            .iter()
            .any(|l| l.content.contains("let b = 2;")));
        assert!(inner_block.lines.len() >= 4);

        let outer_block = &engine.block_stack[0];
        assert!(outer_block.lines.len() >= 4);
    }

    #[test]
    fn test_nested_location_delete() {
        let tmp = create_temp_file(
            "fn outer() {\n    fn inner() {\n        let old = 1;\n        let keep = 2;\n    }\n}\n",
        );
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();

        engine
            .execute_location(&make_location_content(&["fn outer() {"]), false)
            .unwrap();
        engine
            .execute_location(&make_location_content(&["    fn inner() {"]), false)
            .unwrap();

        engine
            .execute_delete(false, Some(&make_delete_content(&["        let old = 1;"])))
            .unwrap();

        let inner_block = engine.block_stack.last().unwrap();
        assert!(!inner_block
            .lines
            .iter()
            .any(|l| l.content.contains("let old")));
        assert!(inner_block
            .lines
            .iter()
            .any(|l| l.content.contains("let keep")));
        assert!(inner_block.lines.len() >= 3);
    }

    #[test]
    fn test_nested_location_off_chain() {
        let tmp = create_temp_file(
            "fn outer() {\n    fn inner() {\n        let old = 1;\n    }\n    let z = 3;\n}\n",
        );
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();

        engine
            .execute_location(&make_location_content(&["fn outer() {"]), false)
            .unwrap();
        engine
            .execute_location(&make_location_content(&["    fn inner() {"]), false)
            .unwrap();

        engine
            .execute_delete(false, Some(&make_delete_content(&["        let old = 1;"])))
            .unwrap();
        engine
            .execute_new(
                &NewPosition::Normal,
                &make_new_content(&["        let new = 2;"]),
            )
            .unwrap();

        engine.execute_off(&OffTarget::Location).unwrap();
        assert_eq!(engine.block_stack.len(), 1);

        let outer_block = engine.block_stack.last().unwrap();
        assert!(outer_block
            .lines
            .iter()
            .any(|l| l.content.contains("let new = 2;")));
        assert!(!outer_block
            .lines
            .iter()
            .any(|l| l.content.contains("let old")));

        engine.execute_off(&OffTarget::Location).unwrap();

        let file = engine.file.as_ref().unwrap();
        assert!(!file.lines.iter().any(|l| l.content.contains("let old")));
        assert!(file.lines.iter().any(|l| l.content.contains("let new")));
        assert!(file.lines.iter().any(|l| l.content.contains("let z")));
    }

    #[test]
    fn test_nested_location_with_empty_inner() {
        let tmp = create_temp_file(
            "fn outer() {\n    fn inner() {\n        let x = 1;\n    }\n    let y = 2;\n}\n",
        );
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();

        engine
            .execute_location(&make_location_content(&["fn outer() {"]), false)
            .unwrap();

        engine
            .execute_location(&make_location_content(&[]), false)
            .unwrap();

        let inner_block = engine.block_stack.last().unwrap();
        assert_eq!(inner_block.start_line, 1);
        assert_eq!(inner_block.lines.len(), 6);
    }

    #[test]
    fn test_nested_location_new_start_end() {
        let tmp =
            create_temp_file("fn outer() {\n    fn inner() {\n        let x = 1;\n    }\n}\n");
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();

        engine
            .execute_location(&make_location_content(&["fn outer() {"]), false)
            .unwrap();
        engine
            .execute_location(&make_location_content(&["    fn inner() {"]), false)
            .unwrap();

        engine
            .execute_new(
                &NewPosition::Start,
                &make_new_content(&["        // start of inner"]),
            )
            .unwrap();

        let inner_block = engine.block_stack.last().unwrap();
        assert_eq!(inner_block.lines[0].content, "        // start of inner");

        engine
            .execute_new(
                &NewPosition::End,
                &make_new_content(&["        // end of inner"]),
            )
            .unwrap();

        let inner_block = engine.block_stack.last().unwrap();
        assert_eq!(
            inner_block.lines.last().unwrap().content,
            "        // end of inner"
        );
    }

    #[test]
    fn test_nested_location_via_commands() {
        let tmp =
            create_temp_file("fn outer() {\n    fn inner() {\n        let a = 1;\n    }\n}\n");
        let commands = vec![
            Command::Open {
                file_path: tmp.path.clone(),
            },
            Command::Location {
                block: false,
                content: make_location_content(&["fn outer() {"]),
            },
            Command::Location {
                block: false,
                content: make_location_content(&["    fn inner() {"]),
            },
            Command::New {
                position: NewPosition::Normal,
                content: make_new_content(&["        let b = 2;"]),
            },
            Command::Off {
                target: OffTarget::Location,
            },
            Command::Off {
                target: OffTarget::Location,
            },
        ];

        let mut engine = Engine::new();
        let result = engine.execute(commands);
        assert!(result.is_ok(), "Unexpected error: {:?}", result.err());

        let file = engine.file.as_ref().unwrap();
        assert!(file.lines.iter().any(|l| l.content.contains("let b = 2;")));
        assert!(file.lines.iter().any(|l| l.content.contains("fn outer")));
        assert!(file.lines.iter().any(|l| l.content.contains("fn inner")));
    }

    // ============================================================
    // Phase 4: 复杂工程场景 — 嵌套 Location 集成测试
    // ============================================================

    #[test]
    fn test_nested_three_level_method_match_arm() {
        let content = [
            "impl Service {",
            "    fn process(&self, status: Status) {",
            "        match status {",
            "            Status::Active => {",
            "                self.do_work();",
            "            }",
            "            Status::Inactive => {",
            "                self.skip();",
            "            }",
            "        }",
            "    }",
            "}",
        ]
        .join("\n");
        let tmp = create_temp_file(&content);
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();

        engine
            .execute_location(&make_location_content(&["impl Service {"]), false)
            .unwrap();

        engine
            .execute_location(&make_location_content(&["        match status {"]), false)
            .unwrap();

        engine
            .execute_location(
                &make_location_content(&["            Status::Active => {"]),
                false,
            )
            .unwrap();

        engine
            .execute_new(
                &NewPosition::Normal,
                &make_new_content(&["                log::info!(\"processing active status\");"]),
            )
            .unwrap();

        let inner = engine.block_stack.last().unwrap();
        assert!(inner.lines.iter().any(|l| l.content.contains("log::info")));

        engine.execute_off(&OffTarget::Location).unwrap();
        engine.execute_off(&OffTarget::Location).unwrap();
        engine.execute_off(&OffTarget::Location).unwrap();

        let file = engine.file.as_ref().unwrap();
        assert!(file.lines.iter().any(|l| l.content.contains("log::info")));
        assert!(file
            .lines
            .iter()
            .any(|l| l.content.contains("Status::Inactive")));
    }

    #[test]
    fn test_nested_three_level_async_error_refactor() {
        let content = [
            "async fn handle_request(req: Request) -> Result<Response> {",
            "    let data = fetch_data().await?;",
            "    if let Some(payload) = data.payload {",
            "        match payload.kind {",
            "            Kind::Success => process(payload).await?,",
            "            Kind::Retry => {",
            "                self.retry_count += 1;",
            "                return Err(Error::retry_exhausted());",
            "            }",
            "        }",
            "    }",
            "    Ok(Response::ok())",
            "}",
        ]
        .join("\n");
        let tmp = create_temp_file(&content);
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();

        engine
            .execute_location(
                &make_location_content(&[
                    "async fn handle_request(req: Request) -> Result<Response> {",
                ]),
                false,
            )
            .unwrap();

        engine
            .execute_location(
                &make_location_content(&["    if let Some(payload) = data.payload {"]),
                false,
            )
            .unwrap();

        engine
            .execute_location(&make_location_content(&["        Kind::Retry => {"]), false)
            .unwrap();

        engine
            .execute_delete(
                false,
                Some(&make_delete_content(&[
                    "            self.retry_count += 1;",
                    "            return Err(Error::retry_exhausted());",
                ])),
            )
            .unwrap();

        engine
            .execute_new(
                &NewPosition::Normal,
                &make_new_content(&[
                    "            self.metrics.record_retry();",
                    "            self.retry_count += 1;",
                    "            if self.retry_count > 3 {",
                    "                return Err(Error::retry_exhausted());",
                    "            }",
                    "            continue;",
                ]),
            )
            .unwrap();

        engine.execute_off(&OffTarget::Location).unwrap();
        engine.execute_off(&OffTarget::Location).unwrap();
        engine.execute_off(&OffTarget::Location).unwrap();

        let file = engine.file.as_ref().unwrap();
        let contents: Vec<&str> = file.lines.iter().map(|l| l.content.as_str()).collect();
        let joined = contents.join("\n");

        assert!(!joined.contains("self.retry_count += 1;\n            return Err"));
        assert!(joined.contains("self.metrics.record_retry()"));
        assert!(joined.contains("if self.retry_count > 3"));
        assert!(joined.contains("Kind::Success"));
    }

    #[test]
    fn test_nested_cross_level_new_delete_with_module_end() {
        let tmp = create_temp_file(
            "pub mod utils {\n\
             pub fn validate(input: &str) -> bool {\n\
             let trimmed = input.trim();\n\
             if trimmed.is_empty() {\n\
             self.log_warning();\n\
             return false;\n\
             }\n\
             true\n\
             }\n\
             }\n",
        );
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();

        engine
            .execute_location(&make_location_content(&["pub mod utils {"]), false)
            .unwrap();

        engine
            .execute_new(
                &NewPosition::End,
                &make_new_content(&[
                    "",
                    "    pub fn sanitize(input: &str) -> String {",
                    "        input.trim().to_lowercase()",
                    "    }",
                ]),
            )
            .unwrap();

        engine
            .execute_location(
                &make_location_content(&["    pub fn validate(input: &str) -> bool {"]),
                false,
            )
            .unwrap();

        engine
            .execute_location(
                &make_location_content(&["        if trimmed.is_empty() {"]),
                false,
            )
            .unwrap();

        engine
            .execute_delete(
                false,
                Some(&make_delete_content(&["            self.log_warning();"])),
            )
            .unwrap();

        engine
            .execute_new(
                &NewPosition::Normal,
                &make_new_content(&["            crate::log::warn(\"empty input\");"]),
            )
            .unwrap();

        engine.execute_off(&OffTarget::Location).unwrap();
        engine.execute_off(&OffTarget::Location).unwrap();
        engine.execute_off(&OffTarget::Location).unwrap();

        let file = engine.file.as_ref().unwrap();
        let contents: Vec<&str> = file.lines.iter().map(|l| l.content.as_str()).collect();
        let joined = contents.join("\n");

        assert!(joined.contains("pub fn sanitize"));
        assert!(joined.contains("input.trim().to_lowercase()"));
        assert!(!joined.contains("self.log_warning()"));
        assert!(joined.contains("crate::log::warn"));
        assert!(joined.contains("pub fn validate"));
    }

    #[test]
    fn test_nested_deep_indentation_preserved() {
        let tmp = create_temp_file(
            "struct Processor {\n\
             items: Vec<Item>,\n\
             }\n\
             impl Processor {\n\
             fn run(&mut self) {\n\
             for item in &self.items {\n\
             if item.is_valid() {\n\
             item.process();\n\
             }\n\
             }\n\
             }\n\
             }\n",
        );
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();

        engine
            .execute_location(&make_location_content(&["impl Processor {"]), false)
            .unwrap();

        engine
            .execute_location(&make_location_content(&["    fn run(&mut self) {"]), false)
            .unwrap();

        engine
            .execute_location(
                &make_location_content(&["        for item in &self.items {"]),
                false,
            )
            .unwrap();

        engine
            .execute_location(
                &make_location_content(&["            if item.is_valid() {"]),
                false,
            )
            .unwrap();

        engine
            .execute_new(
                &NewPosition::Normal,
                &make_new_content(&[
                    "                log::debug!(\"processing item {}\", item.id);",
                    "                metrics::increment_counter(\"items_processed\");",
                ]),
            )
            .unwrap();

        let innermost = engine.block_stack.last().unwrap();
        let logged = innermost
            .lines
            .iter()
            .find(|l| l.content.contains("log::debug"));
        assert!(logged.is_some(), "log::debug line should exist");
        assert_eq!(
            logged.unwrap().taps,
            16,
            "log::debug should have 16 spaces indent"
        );

        engine.execute_off(&OffTarget::Location).unwrap();
        engine.execute_off(&OffTarget::Location).unwrap();
        engine.execute_off(&OffTarget::Location).unwrap();
        engine.execute_off(&OffTarget::Location).unwrap();

        let file = engine.file.as_ref().unwrap();
        assert!(file
            .lines
            .iter()
            .any(|l| l.content.contains("metrics::increment_counter")));
        assert!(file
            .lines
            .iter()
            .any(|l| l.content.contains("item.process()")));
    }

    #[test]
    fn test_nested_multi_operation_inner_block() {
        let content = [
            "fn handler() {",
            "    let config = load_config();",
            "    let mut buffer = Vec::new();",
            "    process_data(&mut buffer);",
            "    let result = finalize(buffer);",
            "    log_result(&result);",
            "}",
        ]
        .join("\n");
        let tmp = create_temp_file(&content);
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();

        engine
            .execute_location(&make_location_content(&["fn handler() {"]), false)
            .unwrap();

        engine
            .execute_location(
                &make_location_content(&["    let mut buffer = Vec::new();"]),
                false,
            )
            .unwrap();

        engine
            .execute_new(
                &NewPosition::Normal,
                &make_new_content(&["    buffer.reserve(1024);"]),
            )
            .unwrap();

        engine.execute_off(&OffTarget::Location).unwrap();

        engine
            .execute_location(&make_location_content(&[]), false)
            .unwrap();
        engine
            .execute_location(
                &make_location_content(&["    process_data(&mut buffer);"]),
                false,
            )
            .unwrap();

        engine
            .execute_delete(
                false,
                Some(&make_delete_content(&["    process_data(&mut buffer);"])),
            )
            .unwrap();

        engine
            .execute_new(
                &NewPosition::Normal,
                &make_new_content(&[
                    "    validate_buffer(&buffer);",
                    "    transform_data(&mut buffer);",
                ]),
            )
            .unwrap();

        engine.execute_off(&OffTarget::Location).unwrap();
        engine.execute_off(&OffTarget::Location).unwrap();
        engine.execute_off(&OffTarget::Location).unwrap();

        let file = engine.file.as_ref().unwrap();
        let contents: Vec<&str> = file.lines.iter().map(|l| l.content.as_str()).collect();
        let joined = contents.join("\n");

        assert!(joined.contains("buffer.reserve(1024)"));
        assert!(joined.contains("validate_buffer"));
        assert!(joined.contains("transform_data"));
        assert!(!joined.contains("process_data(&mut buffer)"));
        assert!(joined.contains("fn handler() {"));
        assert!(joined.contains("log_result(&result)"));
    }

    #[test]
    fn test_nested_location_block_delete_and_new() {
        let content = [
            "impl Calculator {",
            "    fn add(&self, a: i32, b: i32) -> i32 {",
            "        a + b",
            "    }",
            "    fn old_method(&self) {",
            "        self.deprecated_work();",
            "        self.cleanup();",
            "    }",
            "    fn multiply(&self, a: i32, b: i32) -> i32 {",
            "        a * b",
            "    }",
            "}",
        ]
        .join("\n");
        let tmp = create_temp_file(&content);
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();

        engine
            .execute_location(&make_location_content(&["impl Calculator {"]), false)
            .unwrap();

        engine
            .execute_location(
                &make_location_content(&[
                    "    fn old_method(&self) {",
                    "        self.deprecated_work();",
                ]),
                false,
            )
            .unwrap();

        engine
            .execute_delete(
                false,
                Some(&make_delete_content(&[
                    "    fn old_method(&self) {",
                    "        self.deprecated_work();",
                    "        self.cleanup();",
                    "    }",
                ])),
            )
            .unwrap();

        engine
            .execute_new(
                &NewPosition::Normal,
                &make_new_content(&[
                    "    fn subtract(&self, a: i32, b: i32) -> i32 {",
                    "        a - b",
                    "    }",
                ]),
            )
            .unwrap();

        engine.execute_off(&OffTarget::Location).unwrap();
        engine.execute_off(&OffTarget::Location).unwrap();

        let file = engine.file.as_ref().unwrap();
        let contents: Vec<&str> = file.lines.iter().map(|l| l.content.as_str()).collect();
        let joined = contents.join("\n");

        assert!(!joined.contains("fn old_method"));
        assert!(!joined.contains("deprecated_work"));
        assert!(joined.contains("fn subtract"));
        assert!(joined.contains("fn add"));
        assert!(joined.contains("fn multiply"));
    }

    #[test]
    fn test_nested_diff_lines_tracks_all_changes() {
        let tmp = create_temp_file("fn run() {\n    let x = old_calc();\n    let y = x + 1;\n}\n");
        let mut engine = Engine::new();
        engine.execute_open(&tmp.path).unwrap();

        engine
            .execute_location(&make_location_content(&["fn run() {"]), false)
            .unwrap();

        engine
            .execute_location(&make_location_content(&["    let x = old_calc();"]), false)
            .unwrap();

        engine
            .execute_delete(
                false,
                Some(&make_delete_content(&["    let x = old_calc();"])),
            )
            .unwrap();

        engine
            .execute_new(
                &NewPosition::Normal,
                &make_new_content(&["    let x = new_calc();", "    debug_assert!(x >= 0);"]),
            )
            .unwrap();

        engine
            .execute_location(
                &make_location_content(&["    debug_assert!(x >= 0);"]),
                false,
            )
            .unwrap();

        engine
            .execute_new(
                &NewPosition::Normal,
                &make_new_content(&["    log::info!(\"x = {}\", x);"]),
            )
            .unwrap();

        engine.execute_off(&OffTarget::Location).unwrap();
        engine.execute_off(&OffTarget::Location).unwrap();
        engine.execute_off(&OffTarget::Location).unwrap();

        let added: Vec<_> = engine
            .diff_lines
            .iter()
            .filter(|d| d.kind == DiffLineKind::Added)
            .collect();
        let deleted: Vec<_> = engine
            .diff_lines
            .iter()
            .filter(|d| d.kind == DiffLineKind::Deleted)
            .collect();

        assert_eq!(deleted.len(), 1, "Should have 1 deleted line");
        assert!(deleted[0].content.contains("old_calc"));
        assert_eq!(added.len(), 3, "Should have 3 added lines");
        assert!(added.iter().any(|d| d.content.contains("new_calc")));
        assert!(added.iter().any(|d| d.content.contains("debug_assert")));
        assert!(added.iter().any(|d| d.content.contains("log::info")));
    }
}
