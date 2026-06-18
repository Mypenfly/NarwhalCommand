//! 核心匹配算法 (Matcher)
//!
//! 负责 Location 命令中的搜索与匹配逻辑。
//!
//! ## 实现逻辑
//!
//! 1. 首行去空白 → 使用 first_line_index HashMap O(1) 查找候选集
//! 2. 对每个候选逐行比对：去空白 content 一致 + diff_taps 一致
//! 3. 跳过空行进行匹配
//! 4. 唯一性校验：恰好 1 个匹配
//!
//! ## 对应文档
//!
//! 详见 INSTRUCTION.md §3.1 "Location 匹配算法", n_edit_dev.md Location 章节
//!
//! ## 实现状态
//!
//! Phase 2 从 n_edit 迁移（约 95% 可直接复用）。
//!
//! ## 迁移来源
//!
//! 从 n_edit/src/matcher.rs 迁移。

use crate::error::MatchError;
use crate::model::{ContentBlock, LocationContent, SearchScope};

/// Location 匹配器
pub struct LocationMatcher;

impl LocationMatcher {
    /// 在搜索范围内查找 LocationContent 的唯一匹配块
    ///
    /// Phase 2: 待从 n_edit 迁移实现。
    pub fn find_unique_block(
        _scope: &SearchScope<'_>,
        _content: &LocationContent,
        _use_block: bool,
    ) -> Result<ContentBlock, MatchError> {
        // Phase 2: 待实现
        Err(MatchError::NoMatch {
            location_content: String::new(),
        })
    }
}
