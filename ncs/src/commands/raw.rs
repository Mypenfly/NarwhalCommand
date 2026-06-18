//! Raw 命令
//!
//! `!@Raw` 为仅展开命令，其内容在 Parser 阶段已融入
//! 上一个 New 或 Delete 命令，Engine 无需额外处理。
//!
//! ## 实现逻辑
//!
//! Raw 命令不修改任何引擎状态，直接返回 Ok。
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §5.5 "Raw", INSTRUCTION.md §7.6 "命令模块组织"

use crate::engine::Engine;
use crate::error::NcsError;

/// Raw 命令的执行入口
///
/// Raw 内容已在 Parser 阶段融入 New/Delete 的 ContentLines
/// 并标记 `is_raw`，Engine 无需额外处理。
pub fn execute(_engine: &mut Engine, _content: &str) -> Result<(), NcsError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;

    #[test]
    fn test_raw_noop_does_not_change_state() {
        let mut engine = Engine::new();
        assert!(engine.exec_cmds.is_empty());
        assert!(engine.file.is_none());

        let result = execute(&mut engine, "some raw content");
        assert!(result.is_ok());

        // 引擎状态不变
        assert!(engine.exec_cmds.is_empty());
        assert!(engine.file.is_none());
    }

    #[test]
    fn test_raw_with_empty_content() {
        let mut engine = Engine::new();
        let result = execute(&mut engine, "");
        assert!(result.is_ok());
    }
}
