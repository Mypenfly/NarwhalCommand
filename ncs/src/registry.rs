//! 命令注册表 (CommandRegistry)
//!
//! 管理所有可用命令的元信息，包括内置命令初始化和运行时 Include 扩展。
//!
//! ## 实现逻辑
//!
//! 1. 程序启动时调用 `CommandRegistry::init()` 注册所有 12 个内置命令
//! 2. 每个 `CommandEntry` 包含：名称、路径、类型、模式表、从属/所属关系
//! 3. `CommandType` 由 `PermissionType`（权限）和 `ExecutionType`（执行方式）组成
//! 4. `ModeEntry` 定义每个模式下的参数表（名称、必填性、类型、默认值）
//! 5. `ParamDef` 支持 6 种参数类型：String、Path、Number、Bool、StringList、KeyValue
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §3.1 "命令注册表", §8 "命令注册表初始化", INSTRUCTION.md §2.1

use std::collections::HashMap;
use std::path::PathBuf;

/// 命令注册表 — 全局管理的命令元信息
///
/// 程序启动时初始化内置命令，运行时通过 Include 动态扩展。
#[derive(Debug)]
pub struct CommandRegistry {
    /// 命令名 → 命令入口
    pub entries: HashMap<String, CommandEntry>,
}

/// 命令入口
///
/// 包含一条命令的完整元信息：名称、路径、类型、模式、从属/所属关系。
#[derive(Debug, Clone)]
pub struct CommandEntry {
    /// 命令名（全大写，标准化后的名称）
    pub name: String,
    /// 外部命令的执行路径（内置命令为 None）
    pub exec_path: Option<PathBuf>,
    /// 命令类型（权限 + 执行方式）
    pub cmd_type: CommandType,
    /// 模式注册表：模式名 → 模式信息
    pub modes: HashMap<String, ModeEntry>,
    /// 从属命令/模式关系表：决定哪些命令可以使用本命令的输出
    /// 格式: [(从属命令名, 从属模式列表)]
    /// 空列表表示该从属命令的所有模式都可以
    pub subs: Vec<(String, Vec<String>)>,
    /// 所属命令/模式关系表：决定本命令必须在哪些命令之后执行
    /// 格式: [(所属命令名, 所属模式列表)]
    /// 空列表表示该所属命令的所有模式都可以
    pub owners: Vec<(String, Vec<String>)>,
}

/// 命令类型
///
/// 由权限类型、执行类型和输出类型组合而成。
#[derive(Debug, Clone)]
pub struct CommandType {
    /// 权限类型
    pub permission: PermissionType,
    /// 执行类型（行/块/仅展开）
    pub execution: ExecutionType,
    /// 输出类型（流/值），None 表示无输出
    pub output: Option<OutputType>,
}

impl CommandType {
    pub fn new(permission: PermissionType, execution: ExecutionType) -> Self {
        CommandType {
            permission,
            execution,
            output: None,
        }
    }

    pub fn with_output(
        permission: PermissionType,
        execution: ExecutionType,
        output: OutputType,
    ) -> Self {
        CommandType {
            permission,
            execution,
            output: Some(output),
        }
    }

    /// 是否为流输出
    pub fn is_stream(&self) -> bool {
        matches!(self.output, Some(OutputType::StreamOutput))
    }

    /// 是否为值输出
    pub fn is_value_output(&self) -> bool {
        matches!(self.output, Some(OutputType::ValueOutput))
    }

    /// 是否为行执行（不提取内容）
    pub fn is_line_exec(&self) -> bool {
        matches!(self.execution, ExecutionType::LineExec)
    }

    /// 是否为块执行
    pub fn is_block_exec(&self) -> bool {
        matches!(self.execution, ExecutionType::BlockExec)
    }

    /// 是否为仅展开（不触发块终止）
    pub fn is_expand_only(&self) -> bool {
        matches!(self.execution, ExecutionType::ExpandOnly)
    }
}

/// 权限类型 — 决定命令的安全性分类
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionType {
    /// 文件系统：读
    FileRead,
    /// 文件系统：写
    FileWrite,
    /// 文件系统：删
    FileDelete,
    /// 网络连接
    Network,
    /// 程序执行
    ProgramExec,
    /// 无权限要求（如 WorkPath 纯元命令）
    None,
}

/// 执行类型 — 决定命令的执行行为
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionType {
    /// 行执行：只读取命令所在行，不提取后续内容
    LineExec,
    /// 块执行：提取从命令下一行到终止条件的内容
    BlockExec,
    /// 仅展开：遇到时不触发块终止，而是展开为原始字符
    /// （仅用于 Raw 和 Get）
    ExpandOnly,
}

/// 输出类型 — 决定命令输出的保留方式
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputType {
    /// 流输出：输出结果保留在内存中，可供后续命令使用
    StreamOutput,
    /// 值输出：输出结果不保留，仅打印后丢弃
    ValueOutput,
}

/// 模式入口
///
/// 定义命令在某一模式下的参数和行为。
#[derive(Debug, Clone)]
pub struct ModeEntry {
    /// 模式名
    pub name: String,
    /// 参数定义列表
    pub params: Vec<ParamDef>,
    /// 从属命令/模式（模式级别的覆盖）
    pub subs: Vec<(String, Vec<String>)>,
}

/// 参数定义
///
/// 定义命令参数的元信息：名称、类型、必填性、默认值。
#[derive(Debug, Clone)]
pub struct ParamDef {
    /// 参数名
    pub name: String,
    /// 是否必须
    pub required: bool,
    /// 参数类型
    pub param_type: ParamType,
    /// 默认值
    pub default: Option<String>,
}

/// 参数类型
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParamType {
    /// 字符串
    String,
    /// 路径
    Path,
    /// 数字
    Number,
    /// 布尔值
    Bool,
    /// 字符串列表
    StringList,
    /// 键值对
    KeyValue,
}

impl CommandRegistry {
    /// 初始化命令注册表，注册所有 12 个内置命令
    ///
    /// 详见 ncs_dev.md §8。
    pub fn init() -> Self {
        let mut entries = HashMap::new();

        // --- Open ---
        let mut open_modes = HashMap::new();
        open_modes.insert(
            "Normal".to_string(),
            ModeEntry {
                name: "Normal".to_string(),
                params: vec![
                    ParamDef {
                        name: "start".to_string(),
                        required: false,
                        param_type: ParamType::Number,
                        default: Some("1".to_string()),
                    },
                    ParamDef {
                        name: "end".to_string(),
                        required: false,
                        param_type: ParamType::Number,
                        default: None,
                    },
                ],
                subs: vec![],
            },
        );
        open_modes.insert(
            "Dir".to_string(),
            ModeEntry {
                name: "Dir".to_string(),
                params: vec![
                    ParamDef {
                        name: "depth".to_string(),
                        required: false,
                        param_type: ParamType::Number,
                        default: Some("3".to_string()),
                    },
                    ParamDef {
                        name: "ignore".to_string(),
                        required: false,
                        param_type: ParamType::String,
                        default: Some("*.bin".to_string()),
                    },
                    ParamDef {
                        name: "filter".to_string(),
                        required: false,
                        param_type: ParamType::String,
                        default: None,
                    },
                ],
                subs: vec![],
            },
        );
        entries.insert(
            "OPEN".to_string(),
            CommandEntry {
                name: "Open".to_string(),
                exec_path: None,
                cmd_type: CommandType::with_output(
                    PermissionType::FileRead,
                    ExecutionType::LineExec,
                    OutputType::StreamOutput,
                ),
                modes: open_modes,
                subs: vec![
                    ("Location".to_string(), vec!["Normal".to_string()]),
                    ("Location".to_string(), vec!["Block".to_string()]),
                    ("Location".to_string(), vec!["Path".to_string()]),
                    ("New".to_string(), vec!["Start".to_string()]),
                    ("New".to_string(), vec!["End".to_string()]),
                ],
                owners: vec![],
            },
        );

        // --- Location ---
        let mut location_modes = HashMap::new();
        location_modes.insert(
            "Normal".to_string(),
            ModeEntry {
                name: "Normal".to_string(),
                params: vec![],
                subs: vec![],
            },
        );
        location_modes.insert(
            "Block".to_string(),
            ModeEntry {
                name: "Block".to_string(),
                params: vec![],
                subs: vec![],
            },
        );
        location_modes.insert(
            "Path".to_string(),
            ModeEntry {
                name: "Path".to_string(),
                params: vec![],
                subs: vec![],
            },
        );
        entries.insert(
            "LOCATION".to_string(),
            CommandEntry {
                name: "Location".to_string(),
                exec_path: None,
                cmd_type: CommandType::with_output(
                    PermissionType::FileRead,
                    ExecutionType::BlockExec,
                    OutputType::StreamOutput,
                ),
                modes: location_modes,
                subs: vec![
                    ("New".to_string(), vec!["Normal".to_string()]),
                    ("Delete".to_string(), vec!["Normal".to_string()]),
                    ("Delete".to_string(), vec!["Block".to_string()]),
                ],
                owners: vec![("Open".to_string(), vec![])],
            },
        );

        // --- New ---
        let mut new_modes = HashMap::new();
        new_modes.insert(
            "Normal".to_string(),
            ModeEntry {
                name: "Normal".to_string(),
                params: vec![],
                subs: vec![],
            },
        );
        new_modes.insert(
            "Start".to_string(),
            ModeEntry {
                name: "Start".to_string(),
                params: vec![],
                subs: vec![],
            },
        );
        new_modes.insert(
            "End".to_string(),
            ModeEntry {
                name: "End".to_string(),
                params: vec![],
                subs: vec![],
            },
        );
        entries.insert(
            "NEW".to_string(),
            CommandEntry {
                name: "New".to_string(),
                exec_path: None,
                cmd_type: CommandType::new(PermissionType::FileWrite, ExecutionType::BlockExec),
                modes: new_modes,
                subs: vec![],
                owners: vec![
                    ("Location".to_string(), vec!["Normal".to_string()]),
                    ("Location".to_string(), vec!["Block".to_string()]),
                    ("Location".to_string(), vec!["Path".to_string()]),
                    ("Open".to_string(), vec!["Start".to_string()]),
                    ("Open".to_string(), vec!["End".to_string()]),
                ],
            },
        );

        // --- Delete ---
        let mut delete_modes = HashMap::new();
        delete_modes.insert(
            "Normal".to_string(),
            ModeEntry {
                name: "Normal".to_string(),
                params: vec![],
                subs: vec![],
            },
        );
        delete_modes.insert(
            "Block".to_string(),
            ModeEntry {
                name: "Block".to_string(),
                params: vec![],
                subs: vec![],
            },
        );
        entries.insert(
            "DELETE".to_string(),
            CommandEntry {
                name: "Delete".to_string(),
                exec_path: None,
                cmd_type: CommandType::new(PermissionType::FileWrite, ExecutionType::BlockExec),
                modes: delete_modes,
                subs: vec![],
                owners: vec![
                    ("Location".to_string(), vec!["Normal".to_string()]),
                    ("Location".to_string(), vec!["Block".to_string()]),
                    ("Location".to_string(), vec!["Path".to_string()]),
                ],
            },
        );

        // --- Raw ---
        let mut raw_modes = HashMap::new();
        raw_modes.insert(
            "Normal".to_string(),
            ModeEntry {
                name: "Normal".to_string(),
                params: vec![],
                subs: vec![],
            },
        );
        entries.insert(
            "RAW".to_string(),
            CommandEntry {
                name: "Raw".to_string(),
                exec_path: None,
                cmd_type: CommandType::new(PermissionType::None, ExecutionType::ExpandOnly),
                modes: raw_modes,
                subs: vec![],
                owners: vec![("New".to_string(), vec![]), ("Delete".to_string(), vec![])],
            },
        );

        // --- Bash ---
        let mut bash_modes = HashMap::new();
        bash_modes.insert(
            "Normal".to_string(),
            ModeEntry {
                name: "Normal".to_string(),
                params: vec![],
                subs: vec![],
            },
        );
        entries.insert(
            "BASH".to_string(),
            CommandEntry {
                name: "Bash".to_string(),
                exec_path: None,
                cmd_type: CommandType::with_output(
                    PermissionType::ProgramExec,
                    ExecutionType::LineExec,
                    OutputType::StreamOutput,
                ),
                modes: bash_modes,
                subs: vec![],
                owners: vec![],
            },
        );

        // --- Exec ---
        let mut exec_modes = HashMap::new();
        exec_modes.insert(
            "Normal".to_string(),
            ModeEntry {
                name: "Normal".to_string(),
                params: vec![],
                subs: vec![],
            },
        );
        entries.insert(
            "EXEC".to_string(),
            CommandEntry {
                name: "Exec".to_string(),
                exec_path: None,
                cmd_type: CommandType::with_output(
                    PermissionType::ProgramExec,
                    ExecutionType::LineExec,
                    OutputType::ValueOutput,
                ),
                modes: exec_modes,
                subs: vec![],
                owners: vec![],
            },
        );

        // --- Read ---
        let mut read_modes = HashMap::new();
        read_modes.insert(
            "Normal".to_string(),
            ModeEntry {
                name: "Normal".to_string(),
                params: vec![],
                subs: vec![],
            },
        );
        read_modes.insert(
            "Dir".to_string(),
            ModeEntry {
                name: "Dir".to_string(),
                params: vec![],
                subs: vec![],
            },
        );
        entries.insert(
            "READ".to_string(),
            CommandEntry {
                name: "Read".to_string(),
                exec_path: None,
                cmd_type: CommandType::with_output(
                    PermissionType::FileRead,
                    ExecutionType::LineExec,
                    OutputType::ValueOutput,
                ),
                modes: read_modes,
                subs: vec![],
                owners: vec![],
            },
        );

        // --- Write ---
        let mut write_modes = HashMap::new();
        write_modes.insert(
            "Normal".to_string(),
            ModeEntry {
                name: "Normal".to_string(),
                params: vec![],
                subs: vec![],
            },
        );
        write_modes.insert(
            "Raw".to_string(),
            ModeEntry {
                name: "Raw".to_string(),
                params: vec![],
                subs: vec![],
            },
        );
        entries.insert(
            "WRITE".to_string(),
            CommandEntry {
                name: "Write".to_string(),
                exec_path: None,
                cmd_type: CommandType::with_output(
                    PermissionType::FileWrite,
                    ExecutionType::BlockExec,
                    OutputType::ValueOutput,
                ),
                modes: write_modes,
                subs: vec![],
                owners: vec![],
            },
        );

        // --- Include ---
        let mut include_modes = HashMap::new();
        include_modes.insert(
            "Normal".to_string(),
            ModeEntry {
                name: "Normal".to_string(),
                params: vec![
                    ParamDef {
                        name: "alias".to_string(),
                        required: true,
                        param_type: ParamType::String,
                        default: None,
                    },
                    ParamDef {
                        name: "block".to_string(),
                        required: false,
                        param_type: ParamType::Bool,
                        default: Some("false".to_string()),
                    },
                    ParamDef {
                        name: "type".to_string(),
                        required: false,
                        param_type: ParamType::StringList,
                        default: Some("OnlyPrint".to_string()),
                    },
                    ParamDef {
                        name: "exec".to_string(),
                        required: false,
                        param_type: ParamType::String,
                        default: Some("default".to_string()),
                    },
                    ParamDef {
                        name: "owners".to_string(),
                        required: false,
                        param_type: ParamType::StringList,
                        default: None,
                    },
                    ParamDef {
                        name: "subs".to_string(),
                        required: false,
                        param_type: ParamType::StringList,
                        default: None,
                    },
                ],
                subs: vec![],
            },
        );
        entries.insert(
            "INCLUDE".to_string(),
            CommandEntry {
                name: "Include".to_string(),
                exec_path: None,
                cmd_type: CommandType::new(PermissionType::ProgramExec, ExecutionType::LineExec),
                modes: include_modes,
                subs: vec![],
                owners: vec![],
            },
        );

        // --- WorkPath ---
        let mut work_path_modes = HashMap::new();
        work_path_modes.insert(
            "Normal".to_string(),
            ModeEntry {
                name: "Normal".to_string(),
                params: vec![],
                subs: vec![],
            },
        );
        entries.insert(
            "WORKPATH".to_string(),
            CommandEntry {
                name: "WorkPath".to_string(),
                exec_path: None,
                cmd_type: CommandType::new(PermissionType::None, ExecutionType::LineExec),
                modes: work_path_modes,
                subs: vec![],
                owners: vec![],
            },
        );

        // --- Get ---
        let mut get_modes = HashMap::new();
        get_modes.insert(
            "Normal".to_string(),
            ModeEntry {
                name: "Normal".to_string(),
                params: vec![ParamDef {
                    name: "like".to_string(),
                    required: false,
                    param_type: ParamType::String,
                    default: None,
                }],
                subs: vec![],
            },
        );
        entries.insert(
            "GET".to_string(),
            CommandEntry {
                name: "Get".to_string(),
                exec_path: None,
                cmd_type: CommandType::with_output(
                    PermissionType::None,
                    ExecutionType::ExpandOnly,
                    OutputType::StreamOutput,
                ),
                modes: get_modes,
                subs: vec![],
                owners: vec![],
            },
        );

        CommandRegistry { entries }
    }

    /// 通过命令名查找对应的 CommandEntry（不区分大小写）
    ///
    /// 内部会将命令名标准化为全大写后再查找。
    pub fn find_command(&self, name: &str) -> Option<&CommandEntry> {
        let normalized = normalize_command_name(name);
        self.entries.get(&normalized)
    }

    /// 注册一个新命令（由 Include 调用）
    ///
    /// 若命令名已存在，返回旧条目。
    pub fn register(&mut self, entry: CommandEntry) -> Option<CommandEntry> {
        let key = normalize_command_name(&entry.name);
        self.entries.insert(key, entry)
    }
}

/// 标准化命令名为全大写，消除空格、下划线、连字符
///
/// 例如 "example_command" / "Example-Command" 均标准化为 "EXAMPLECOMMAND"
pub fn normalize_command_name(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_whitespace() && *c != '_' && *c != '-')
        .flat_map(|c| c.to_uppercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_init_contains_all_commands() {
        let registry = CommandRegistry::init();
        let expected = [
            "OPEN", "LOCATION", "NEW", "DELETE", "RAW", "BASH", "EXEC", "READ", "WRITE", "INCLUDE",
            "WORKPATH", "GET",
        ];
        for name in &expected {
            assert!(
                registry.entries.contains_key(*name),
                "Registry should contain {}",
                name
            );
        }
        assert_eq!(registry.entries.len(), 12);
    }

    #[test]
    fn test_find_command_case_insensitive() {
        let registry = CommandRegistry::init();
        assert!(registry.find_command("open").is_some());
        assert!(registry.find_command("OPEN").is_some());
        assert!(registry.find_command("Open").is_some());
        assert!(registry.find_command("location").is_some());
        assert!(registry.find_command("LOCATION").is_some());
    }

    #[test]
    fn test_find_command_with_underscores() {
        let registry = CommandRegistry::init();
        assert!(registry.find_command("WorkPath").is_some());
        assert!(registry.find_command("work_path").is_some());
        assert!(registry.find_command("WORK-PATH").is_some());
    }

    #[test]
    fn test_find_command_nonexistent() {
        let registry = CommandRegistry::init();
        assert!(registry.find_command("UnknownCmd").is_none());
    }

    #[test]
    fn test_open_entry_has_correct_subs() {
        let registry = CommandRegistry::init();
        let open = registry.find_command("open").unwrap();
        assert!(open.subs.iter().any(|(name, _)| name == "Location"));
        assert!(open.subs.iter().any(|(name, _)| name == "New"));
        assert!(open.owners.is_empty());
    }

    #[test]
    fn test_location_entry_has_correct_owners() {
        let registry = CommandRegistry::init();
        let location = registry.find_command("location").unwrap();
        assert!(location.owners.iter().any(|(name, _)| name == "Open"));
        assert!(location.subs.iter().any(|(name, _)| name == "New"));
        assert!(location.subs.iter().any(|(name, _)| name == "Delete"));
    }

    #[test]
    fn test_new_entry_has_correct_owners() {
        let registry = CommandRegistry::init();
        let new_cmd = registry.find_command("new").unwrap();
        assert!(new_cmd.owners.iter().any(|(name, _)| name == "Location"));
        assert!(new_cmd.subs.is_empty());
    }

    #[test]
    fn test_open_has_dir_mode() {
        let registry = CommandRegistry::init();
        let open = registry.find_command("open").unwrap();
        assert!(open.modes.contains_key("Dir"));
        let dir_mode = &open.modes["Dir"];
        assert_eq!(dir_mode.params.len(), 3);
        assert_eq!(dir_mode.params[0].name, "depth");
        assert_eq!(dir_mode.params[0].default, Some("3".to_string()));
    }

    #[test]
    fn test_command_type_is_stream() {
        let t = CommandType::with_output(
            PermissionType::FileRead,
            ExecutionType::LineExec,
            OutputType::StreamOutput,
        );
        assert!(t.is_stream());
        assert!(t.is_line_exec());
        assert!(!t.is_block_exec());
        assert!(!t.is_expand_only());
    }

    #[test]
    fn test_command_type_is_block_exec() {
        let t = CommandType::new(PermissionType::FileWrite, ExecutionType::BlockExec);
        assert!(t.is_block_exec());
        assert!(!t.is_stream());
    }

    #[test]
    fn test_command_type_is_expand_only() {
        let t = CommandType::new(PermissionType::None, ExecutionType::ExpandOnly);
        assert!(t.is_expand_only());
        assert!(!t.is_stream());
        assert!(!t.is_block_exec());
    }

    #[test]
    fn test_normalize_command_name() {
        assert_eq!(normalize_command_name("open"), "OPEN");
        assert_eq!(normalize_command_name("Work_Path"), "WORKPATH");
        assert_eq!(normalize_command_name("Example-Command"), "EXAMPLECOMMAND");
        assert_eq!(normalize_command_name("e x_a m-p l e"), "EXAMPLE");
    }

    #[test]
    fn test_register_new_command() {
        let mut registry = CommandRegistry::init();
        let entry = CommandEntry {
            name: "TestCmd".to_string(),
            exec_path: Some(PathBuf::from("/usr/bin/test")),
            cmd_type: CommandType::new(PermissionType::ProgramExec, ExecutionType::LineExec),
            modes: HashMap::new(),
            subs: vec![],
            owners: vec![],
        };
        let result = registry.register(entry);
        assert!(result.is_none());
        assert!(registry.find_command("testcmd").is_some());
    }

    #[test]
    fn test_register_overwrites_existing() {
        let mut registry = CommandRegistry::init();
        let entry = CommandEntry {
            name: "Open".to_string(),
            exec_path: Some(PathBuf::from("/custom/open")),
            cmd_type: CommandType::new(PermissionType::FileRead, ExecutionType::LineExec),
            modes: HashMap::new(),
            subs: vec![],
            owners: vec![],
        };
        let result = registry.register(entry);
        assert!(result.is_some());
    }

    #[test]
    fn test_include_has_required_params() {
        let registry = CommandRegistry::init();
        let include = registry.find_command("include").unwrap();
        let normal_mode = &include.modes["Normal"];
        let alias_param = normal_mode
            .params
            .iter()
            .find(|p| p.name == "alias")
            .unwrap();
        assert!(alias_param.required, "alias should be required");
        assert_eq!(alias_param.param_type, ParamType::String);
    }

    #[test]
    fn test_get_has_like_param() {
        let registry = CommandRegistry::init();
        let get_cmd = registry.find_command("get").unwrap();
        let normal_mode = &get_cmd.modes["Normal"];
        let like_param = normal_mode
            .params
            .iter()
            .find(|p| p.name == "like")
            .unwrap();
        assert!(!like_param.required, "like should be optional");
        assert_eq!(like_param.param_type, ParamType::String);
    }
}
