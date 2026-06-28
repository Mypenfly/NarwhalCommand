//! Like 命令
//!
//! `!@Like pool_name like=CommandName [Mode]` 从 pools 中提取 CmdContent，
//! 并将伪装的目标命令写入 exec_cmds，供后续命令的 owner 检查通过。
//!
//! ## 实现逻辑
//!
//! 1. 从 engine.pools 按 pool_name 查找 CmdContent
//! 2. 将 (like_cmd, like_mode) 写入 exec_cmds 作为伪装条目
//! 3. 将 CmdContent 存入 engine.last_result
//! 4. like= 后面的命令不实际执行，仅用于伪装
//!
//! ## 对应文档
//!
//! 详见 phases.md §Phase 5.2 "Like 伪装命令"

use crate::cmd_content::CmdContent;
use crate::engine::{Engine, ExecutedCommand};
use crate::error::NcsError;

/// Like 命令的执行入口
///
/// 从 pools 取出数据，并将伪装条目加入 exec_cmds。
pub fn execute(
    engine: &mut Engine,
    pool_name: &str,
    like_cmd: &str,
    like_mode: &str,
) -> Result<CmdContent, NcsError> {
    let content = engine.pools.get(pool_name).cloned().ok_or_else(|| {
        NcsError::Engine(crate::error::EngineError::NotImplemented {
            feature: format!("Like pool '{}' not found", pool_name),
        })
    })?;

    engine.exec_cmds.push(ExecutedCommand {
        cmd_name: like_cmd.to_uppercase(),
        mode_name: like_mode.to_string(),
        is_independent: false,
    });

    Ok(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd_content::CmdContent;

    #[test]
    fn test_like_adds_to_exec_cmds() {
        let mut engine = Engine::new();
        engine.pools.insert(
            "my_pool".to_string(),
            CmdContent::from_raw_text("hello world".to_string()),
        );

        let result = execute(&mut engine, "my_pool", "Open", "Normal");
        assert!(result.is_ok());
        assert_eq!(engine.exec_cmds.len(), 1);
        assert_eq!(engine.exec_cmds[0].cmd_name, "OPEN");
        assert_eq!(engine.exec_cmds[0].mode_name, "Normal");
    }

    #[test]
    fn test_like_missing_pool_is_error() {
        let mut engine = Engine::new();
        let result = execute(&mut engine, "missing", "Open", "Normal");
        assert!(result.is_err());
    }
}
