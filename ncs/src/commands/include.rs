//! Include 命令
//!
//! 动态注册外部命令到 CommandRegistry。
//!
//! ## 实现逻辑
//!
//! 1. alias 前的所有位置参数拼接为外部命令的执行指令
//! 2. 校验 alias 不与内置命令重名
//! 3. 根据 exec 参数选择执行策略（Default/Bash/Script）
//! 4. 根据 work_path 展开相对路径
//! 5. 调用 registry.register() 注册
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §5.10 "Include", INSTRUCTION.md §3

use crate::error::{NcsError, RegistryError};
use crate::registry::{
    CommandEntry, CommandRegistry, CommandType, ExecMethod, ExecutionType, OutputType,
    PermissionType,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Include 命令的执行入口
pub fn execute(
    command: &str,
    args: &HashMap<String, String>,
    registry: &mut CommandRegistry,
    base_path: &Path,
) -> Result<(), NcsError> {
    let alias = args.get("alias").ok_or_else(|| {
        NcsError::Registry(RegistryError::AliasConflict {
            alias: "missing".to_string(),
            existing_cmd: String::new(),
            line: crate::model::LineNumber::new(0),
        })
    })?;

    let normalized_alias = crate::registry::normalize_command_name(alias);

    if registry.find_command(&normalized_alias).is_some() {
        return Err(NcsError::Registry(RegistryError::AliasConflict {
            alias: alias.clone(),
            existing_cmd: normalized_alias,
            line: crate::model::LineNumber::new(0),
        }));
    }

    // 展开相对路径：所有以 ./ 或 ../ 开头的词都展开
    let mut parts: Vec<String> = command.split_whitespace().map(|s| s.to_string()).collect();
    for part in &mut parts {
        *part = resolve_path(part, base_path);
    }
    let resolved_command = parts.join(" ");

    let is_block = args.get("block").map(|v| v == "true").unwrap_or(false);

    let exec_str = args.get("exec").map(|v| v.as_str()).unwrap_or("default");
    let exec_method = match exec_str.to_lowercase().as_str() {
        "bash" => ExecMethod::Bash,
        "script" => ExecMethod::Script,
        _ => ExecMethod::Default,
    };

    let types = args.get("type").map(|v| v.as_str()).unwrap_or("OnlyPrint");

    let execution = if is_block {
        ExecutionType::BlockExec
    } else {
        ExecutionType::LineExec
    };

    let output = if types.contains("StreamOutput") {
        Some(OutputType::StreamOutput)
    } else {
        Some(OutputType::ValueOutput)
    };

    let entry = CommandEntry {
        name: alias.clone(),
        exec_path: Some(PathBuf::from(&resolved_command)),
        exec_method,
        cmd_type: CommandType {
            permission: PermissionType::ProgramExec,
            execution,
            output,
        },
        modes: HashMap::new(),
        subs: vec![],
        owners: vec![],
    };

    registry.register(entry);
    Ok(())
}

/// 根据 base_path 展开相对路径为绝对路径
///
/// 只有以 `./` 或 `../` 开头的显式相对路径才被展开。
/// 系统命令（如 `python3`）、绝对路径、`~` 路径均保持原样。
fn resolve_path(path: &str, base_path: &Path) -> String {
    if path.starts_with('/') {
        return path.to_string();
    }
    if path.starts_with("./") {
        let rel = path.strip_prefix("./").unwrap();
        return base_path.join(rel).to_string_lossy().to_string();
    }
    if path.starts_with("../") {
        return base_path.join(path).to_string_lossy().to_string();
    }
    // 系统命令或 ~ 路径保持原样
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_path_preserves_system_command() {
        let base = Path::new("/home/user/project");
        let result = resolve_path("python3", base);
        assert_eq!(result, "python3", "系统命令不应被展开");
    }

    #[test]
    fn test_resolve_path_expands_dot_slash() {
        let base = Path::new("/home/user/project");
        let result = resolve_path("./web_search.py", base);
        assert_eq!(result, "/home/user/project/web_search.py");
    }

    #[test]
    fn test_resolve_path_expands_dot_dot_slash() {
        let base = Path::new("/home/user/project/subdir");
        let result = resolve_path("../tools/tool.sh", base);
        // join 保留 .. 组件，路径语义正确即可
        assert!(result.contains("/project"));
        assert!(result.contains("tools/tool.sh"));
    }

    #[test]
    fn test_resolve_path_preserves_absolute_path() {
        let base = Path::new("/home/user/project");
        let result = resolve_path("/usr/bin/python3", base);
        assert_eq!(result, "/usr/bin/python3");
    }

    #[test]
    fn test_resolve_path_expands_tilde() {
        let base = Path::new("/home/user/project");
        let result = resolve_path("~/bin/mytool", base);
        assert_eq!(
            result, "~/bin/mytool",
            "~ 开头的命令不展开（由 shell 处理）"
        );
    }
}
