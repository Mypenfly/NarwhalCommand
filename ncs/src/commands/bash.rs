//! Bash 命令
//!
//! 通过 bash -c 执行系统命令，捕获 stdout/stderr。
//!
//! ## 实现逻辑
//!
//! 1. 安全审查：拦截 sudo/rm -rf / /chmod 777 / 等高危模式
//! 2. std::process::Command::new("bash").arg("-c").arg(command).output()
//! 3. stdout → CmdContent.lines，stderr → 附加到 result
//! 4. 流输出，结果保留供后续命令使用
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §5.6 "Bash", INSTRUCTION.md §3

use crate::cmd_content::CmdContent;
use crate::error::{CommandExecError, NcsError};

/// Bash 命令的执行入口
///
/// 返回 CmdContent 包含 stdout 和 stderr。
pub fn execute(command: &str) -> Result<CmdContent, NcsError> {
    security_check(command)?;

    let output = std::process::Command::new("bash")
        .arg("-c")
        .arg(command)
        .output()
        .map_err(|e| {
            NcsError::CommandExec(CommandExecError::ExecutionFailed {
                command: command.to_string(),
                exit_code: None,
                stderr: e.to_string(),
            })
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(NcsError::CommandExec(CommandExecError::ExecutionFailed {
            command: command.to_string(),
            exit_code: output.status.code(),
            stderr,
        }));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let mut content = CmdContent::from_raw_text(stdout);
    content.source_info = Some(crate::cmd_content::ContentSource::CommandOutput);

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !stderr.is_empty() {
        content.result = stderr
            .lines()
            .enumerate()
            .map(|(i, l)| crate::cmd_content::CmdLine {
                line_num: i + 1,
                content: l.to_string(),
            })
            .collect();
    }

    Ok(content)
}

/// 安全审查：检测并拒绝高危命令模式
pub fn security_check(command: &str) -> Result<(), NcsError> {
    let lower = command.to_lowercase();

    let dangerous_patterns: &[(&str, &str)] = &[
        ("sudo", "禁止使用 sudo 提权执行命令"),
        ("rm -rf /", "禁止递归强制删除根目录"),
        ("chmod 777 /", "禁止对根目录设置 777 权限"),
        ("mkfs.", "禁止格式化文件系统命令"),
        ("dd if=", "禁止使用 dd 命令直接操作磁盘"),
        ("> /dev/sd", "禁止直接写入磁盘设备"),
        ("forkbomb", "禁止 fork 炸弹"),
        (":(){ :|:& };:", "禁止 fork 炸弹"),
    ];

    for (pattern, reason) in dangerous_patterns {
        if lower.contains(pattern) {
            return Err(NcsError::CommandExec(CommandExecError::SecurityDenied {
                command: command.to_string(),
                reason: reason.to_string(),
            }));
        }
    }

    Ok(())
}
