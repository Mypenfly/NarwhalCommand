//! NCS 库入口
//!
//! 导出所有公开模块，供集成测试使用。
//!
//! ## 模块架构
//!
//! 详见 INSTRUCTION.md §6 "项目结构与文件组织"

pub mod block;
pub mod cmd_content;
pub mod engine;
pub mod error;
pub mod file_io;
pub mod lexer;
pub mod matcher;
pub mod model;
pub mod output;
pub mod parser;
pub mod registry;

pub mod commands;
