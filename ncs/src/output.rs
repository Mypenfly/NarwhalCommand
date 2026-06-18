//! 彩色终端输出 (Output)
//!
//! 负责 diff 输出格式化和错误信息颜色渲染。
//!
//! ## 实现逻辑
//!
//! 1. DiffLine 携带行类型（Added / Deleted / Unchanged / Separator）
//! 2. 终端输出时：+ 绿色（新增）、- 红色（删除）、灰色（上下文）
//! 3. format_error_with_color: 按规范格式化错误（Error 红色、标题黄色等）
//! 4. 自动检测 is_terminal，管道/重定向时关闭颜色
//!
//! ## 对应文档
//!
//! 详见 INSTRUCTION.md §5.3 "输出格式", ncs_dev.md §7.3 "错误输出格式"
//!
//! ## 实现状态
//!
//! Phase 2 从 n_edit 迁移（约 95% 可直接复用）。
//!
//! ## 迁移来源
//!
//! 从 n_edit/src/output.rs 迁移。
