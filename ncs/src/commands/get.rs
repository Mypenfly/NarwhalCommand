//! Get 命令
//!
//! `!@Get` 从全局 pools 中提取 CmdContent。
//!
//! ## 实现逻辑
//!
//! 1. 从 engine.pools 按 pool_name 查找
//! 2. 返回克隆的 CmdContent（保留原始快照和变更记录）
//! 3. like 伪装模式待 Phase 5 实现
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §5.12 "Get", INSTRUCTION.md §3

use crate::cmd_content::CmdContent;
use crate::engine::Engine;
use crate::error::NcsError;

/// Get 命令的执行入口
///
/// 从 engine.pools 获取 pool_name 对应的 CmdContent 并克隆返回。
pub fn execute(engine: &Engine, pool_name: &str) -> Result<CmdContent, NcsError> {
    engine.pools.get(pool_name).cloned().ok_or_else(|| {
        NcsError::Engine(crate::error::EngineError::NotImplemented {
            feature: format!("Get pool '{}' not found", pool_name),
        })
    })
}
