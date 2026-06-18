//! 命令模块入口
//!
//! 每个内置命令的实现放在独立文件中，
//! 通过此模块统一导出。
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §5 "命令定义", INSTRUCTION.md §7.6 "命令模块组织"

pub mod delete;
pub mod location;
pub mod new;
pub mod open;
pub mod raw;
