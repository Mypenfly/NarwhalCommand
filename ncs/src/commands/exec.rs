//! Exec 命令
//!
//! 通过 script -c 直连终端执行命令，支持彩色输出和交互。
//!
//! ## 实现逻辑
//!
//! 1. std::process::Command::new("script").arg("-q").arg("-c").arg(command)
//! 2. script -c 保留终端特性（彩色、流式、交互）
//! 3. 值输出：结果仅打印，不保留
//! 4. 使用 script -q 不输出 script 自身的开始/结束消息
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §5.7 "Exec", INSTRUCTION.md §3

use crate::error::{CommandExecError, NcsError};

/// Exec 命令的执行入口
pub fn execute(command: &str) -> Result<(), NcsError> {
    let status = std::process::Command::new("script")
        .arg("-q")
        .arg("-c")
        .arg(command)
        .arg("/dev/null")
        .status()
        .map_err(|e| {
            NcsError::CommandExec(CommandExecError::ExecutionFailed {
                command: command.to_string(),
                exit_code: None,
                stderr: e.to_string(),
            })
        })?;

    if !status.success() {
        return Err(NcsError::CommandExec(CommandExecError::ExecutionFailed {
            command: command.to_string(),
            exit_code: status.code(),
            stderr: String::new(),
        }));
    }

    Ok(())
}
