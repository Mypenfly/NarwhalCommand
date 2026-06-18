//! Block 解析器 (Block)
//!
//! 负责解析代码块边界（花括号块 / 缩进块），
//! 用于 Location:Block 指令的精确定位。
//!
//! ## 实现逻辑
//!
//! 1. detect_language: 判断代码语言类型（花括号 / 缩进 / Unknown）
//! 2. parse_brace_block: 逐字符扫描，处理 depth/in_string/in_comment
//! 3. parse_indent_block: 基于 taps 层级判断边界
//!
//! ## 对应文档
//!
//! 详见 INSTRUCTION.md §3.2 "Block 解析算法", n_edit_dev.md Location:Block 章节
//!
//! ## 实现状态
//!
//! Phase 2 从 n_edit 迁移（100% 可直接复用）。
//!
//! ## 迁移来源
//!
//! 从 n_edit/src/block.rs 迁移。
