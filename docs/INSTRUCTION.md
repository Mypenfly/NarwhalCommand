# Narwhal Command Script 实现指令

## 1. 总体设计路径

### 1.1 架构总览

```
                        ┌──────────────┐
       script.ncs ────▶ │   CLI 入口    │
                        └──────┬───────┘
                               │
                        ┌──────▼───────┐
                        │  词法分析器    │  —— 扫描 `!@Cmd` 标识符，切分为 Token 流
                        └──────┬───────┘
                               │
                        ┌──────▼───────┐
                        │  语法分析器    │  —— 将 Token 流组装为 AST（命令序列）
                        └──────┬───────┘
                               │
                        ┌──────▼───────┐
                        │  执行引擎     │  —— 状态机 + exec_cmds 管理 + 命令分发
                        └──────┬───────┘
                               │
               ┌───────────────┼───────────────┐
               │               │               │
        ┌──────▼──────┐ ┌─────▼─────┐ ┌───────▼───────┐
        │ 命令注册表    │ │  匹配器   │ │  Block解析器   │
        └─────────────┘ └───────────┘ └───────────────┘
               │
        ┌──────▼──────┐
        │ 各命令实现    │  —— Open / Location / New / Delete / Raw
        │ (commands/) │      Bash / Exec / Read / Write / Include
        └─────────────┘      WorkPath / Get
```

### 1.2 核心概念

| 概念 | 说明 | 详见 |
|------|------|------|
| `!@Cmd` | 命令前缀，`!` 示意脚本执行，`@` 标识定向命令 | ncs_dev.md §1 |
| `@/Cmd` | 关闭符号，发起层级的关闭，块执行的终止符之一 | ncs_dev.md §1 |
| **块执行** | 提取命令下一行到终止条件的内容作为操作对象 | ncs_dev.md §2.3 |
| **仅展开命令** | `!@Raw`、`!@Get`，遇到时不触发块终止，展开为原始字符 | ncs_dev.md §3.1 |
| **流输出** | 执行结果保留供后续命令使用，`@/Cmd` 时触发打印 | ncs_dev.md §6.2 |
| **值输出** | 执行结果仅打印后丢弃 | ncs_dev.md §6.2 |
| **exec_cmds** | 已执行且仍在生效的命令列表，管理命令从属关系 | ncs_dev.md §3.2 |
| **CmdContent** | 命令间传递的统一数据结构 | ncs_dev.md §3.3 |
| **CommandRegistry** | 全局命令注册表，运行时管理所有可用命令的元信息 | ncs_dev.md §3.1 |
| **diff_taps** | 相对于所在 ContentBlock 首行的缩进差异 | n_edit_dev.md |
| **stripped_content** | 行内容去除所有空白字符后的版本，用于模糊匹配 | n_edit_dev.md |

### 1.3 命令状态机

```
                        ┌──────────────────────────────────────────────┐
                        │                 全局状态                     │
                        │  file: Option<FileContent>                   │
                        │  block_stack: Vec<ContentBlock>              │
                        │  exec_cmds: Vec<ExecutedCommand>             │
                        │  pools: HashMap<String, CmdContent>          │
                        │  last_result: Option<CommandResult>          │
                        │       (当前 CmdContent 管道，含变更记录)      │
                        └──────────────────────────────────────────────┘

   Open ──► Location ──► [Location]* ──► New ──► Delete ──► @/Open
              │              │
              │              └──► [Raw]  (仅展开，融入父命令内容)
              │
              └──► Bash / Exec / Read / Include / Get  (独立命令)
                           │
                           └──► WorkPath  (无类型元命令)
```

**状态约束**：
- `New(Normal)` / `Delete` 之前必须存在已执行的 `Location`（在 `exec_cmds` 中）
- `New(Start)` / `New(End)` 之前必须存在已执行的 `Open`
- `Delete(Block)` 要求前一个 `Location` 使用了 Block 模式
- `@/Cmd` 按从后向前的顺序清除 `exec_cmds` 中的非独立命令
- 脚本末尾未显式关闭的命令自动执行隐式关闭
- **CmdContent 变更追踪**：命令不直接修改文件行，向 CmdContent 追加 ContentChange 记录，变更在 Owner 关闭时统一生效

---

## 2. 详细数据结构

### 2.1 命令注册表

```rust
/// 命令注册表 — 全局管理的命令元信息
/// 程序启动时初始化内置命令，运行时通过 Include 扩展
struct CommandRegistry {
    entries: HashMap<String, CommandEntry>,
}

struct CommandEntry {
    name: String,
    exec_path: Option<PathBuf>,
    cmd_type: CommandType,
    modes: HashMap<String, ModeEntry>,
    subs: Vec<(String, Vec<String>)>,
    owners: Vec<(String, Vec<String>)>,
}

struct CommandType {
    permission: PermissionType,
    execution: ExecutionType,
}

enum PermissionType { FileRead, FileWrite, FileDelete, Network, ProgramExec, None }
enum ExecutionType { LineExec, BlockExec, ValueOutput, StreamOutput, ExpandOnly }

struct ModeEntry {
    name: String,
    params: Vec<ParamDef>,
    subs: Vec<(String, Vec<String>)>,
}

struct ParamDef {
    name: String,
    required: bool,
    param_type: ParamType,
    default: Option<String>,
}

enum ParamType { String, Path, Number, Bool, StringList, KeyValue }
```

### 2.2 exec_cmds

```rust
/// 已执行且仍在生效的命令记录
struct ExecutedCommand {
    cmd_name: String,
    mode_name: String,
    is_independent: bool,
}
```

### 2.3 CmdContent — 命令间数据传递 + 变更追踪

```rust
/// 命令间传递的统一数据结构 + 变更追踪
///
/// 核心模型：命令不直接修改文件行，而是追加 ContentChange 记录。
/// 变更在 Owner 命令退出时由 apply_changes() 统一生效。
struct CmdContent {
    raw_content: String,
    lines: Vec<CmdLine>,
    is_print: bool,
    result: Vec<CmdLine>,

    // === 变更追踪字段 ===
    /// Location 创建时的原始快照（Delete 匹配目标，不随 Insert 改变）
    snapshot_lines: Vec<CmdLine>,
    snapshot_raw: String,
    /// 变更记录列表（按命令执行顺序追加）
    changes: Vec<ContentChange>,
    /// 数据来源（Block / File / CommandOutput），决定变更写回目标
    source_info: Option<ContentSource>,
}

struct CmdLine { line_num: usize, content: String }

/// 内容变更记录
enum ContentChange {
    Insert {
        after_line: usize,      // snapshot 中的插入位置
        lines: Vec<CmdLine>,
        source_cmd: String,     // "NEW"
    },
    Delete {
        start_line: usize,      // snapshot 中的删除起始
        end_line: usize,        // snapshot 中的删除结束
        source_cmd: String,     // "DELETE"
    },
}

enum ContentSource {
    Block { block_index: usize },
    File { file_path: String },
    CommandOutput,
}
```

**关键设计要点**：
- `snapshot_lines` 是 Location 创建时的原始数据，**从不被修改**。Delete 始终在 snapshot 上匹配
- `changes` 是追加列表，每个命令通过 `record_insert()` / `record_delete()` 追加
- `lines` 是惰性字段，在 `apply_changes()` 时从 snapshot + changes 计算
- 同一作用域内的命令共享同一个 CmdContent，串行查看/追加变更

### 2.4 命令 AST（Parser 输出）

```rust
enum Command {
    Open   { mode: OpenMode,   path: String,   args: HashMap<String, String> },
    Location { mode: LocationMode, content: Option<LocationContent>, args: HashMap<String, String> },
    New    { mode: NewMode,    content: NewContent },
    Delete { mode: DeleteMode, content: Option<DeleteContent> },
    Raw    { content: String },
    Bash   { command: String },
    Exec   { command: String },
    Read   { path: String,     args: HashMap<String, String> },
    Write  { mode: WriteMode,  path: String,   content: Option<String> },
    Include { path: String,    args: HashMap<String, String> },
    WorkPath { path: String },
    Get    { pool_name: String, like: Option<String> },
    Close  { name: String },
}

enum OpenMode    { Normal, Dir }
enum LocationMode { Normal, Block, Path }
enum NewMode     { Normal, Start, End }
enum DeleteMode  { Normal, Block }
enum WriteMode   { Normal, Raw }
```

### 2.5 文件 / Block 数据结构（保留自 n_edit）

```rust
struct FileContent {
    lines: Vec<Line>,
    first_line_index: HashMap<String, Vec<usize>>,
}

struct ContentBlock {
    start_line: LineNumber,
    end_line: LineNumber,
    lines: Vec<Line>,
    first_line_index: HashMap<String, Vec<usize>>,
    match_info: MatchInfo,
}

struct Line {
    line_num: LineNumber,
    taps: usize,
    diff_taps: usize,
    content: String,
    stripped_content: String,     // 预计算的去空白版本，加速匹配
}

enum MatchInfo {
    Empty,
    Location { matched_line_count: usize },
    DeleteAt { position: usize },
}
```

### 2.6 Location / New / Delete 内容结构（保留自 n_edit）

```rust
struct LocationContent {
    lines: Vec<LocationLine>,
}
struct LocationLine {
    index: usize,
    diff_taps: Option<usize>,
    content: String,
    line_num: Option<LineNumber>,
}

struct NewContent {
    lines: Vec<NewLine>,
}
struct NewLine {
    diff_taps: usize,
    content: String,
    is_raw: bool,
}

struct DeleteContent {
    lines: Vec<DeleteLine>,
}
struct DeleteLine {
    content: String,
    is_raw: bool,
}
```

---

## 3. 关键算法流程

### 3.1 Location 匹配算法

> 详见 n_edit_dev.md 的 Location 章节，核心逻辑完全保留。

```
输入: SearchScope, LocationContent
输出: ContentBlock 或 MatchError

1. 首行去空白 → 使用 first_line_index HashMap O(1) 查找候选集
2. 对每个候选：逐行比对
   - 去空白 content 必须一致
   - diff_taps 必须一致
   - 跳过空行
3. 唯一性校验：恰好 1 个 → 返回 ContentBlock
   否则 → 报 MatchError（附带候选列表，最多展示 3 个）
4. 若 block = true → 调用 BlockParser 获取精确块边界
   否则 → Block 边界为从首行到搜索范围末尾
```

### 3.2 Block 解析算法

> 详见 n_edit_dev.md 的 Location:Block 章节，核心逻辑完全保留。

```
1. detect_language(scope, start_index):
   - 花括号语言 (Rust/C/JS/Java): 检查 {, }
   - 缩进语言 (Python/YAML): 检查 diff_taps 不全为 0
   - Unknown → BlockNotParseable 错误

2. 花括号: parse_brace_block → 逐字符扫描
   维护 depth/in_string/in_comment，处理 //、/* */、\"、\\

3. 缩进: parse_indent_block → 基于 taps 层级判断
   跳过空行和注释行，taps <= base_taps 时结束
```

### 3.3 New 插入算法（变更追踪）

> 详见 ncs_dev.md §5.3。

```
1. 检查 exec_cmds 中有所属命令（Location），否则报错
2. 接收上一个命令的 CmdContent（含 snapshot_lines 和已有变更）
3. 将 NewContent 行转换为 Vec<CmdLine>（含 diff_taps 缩进计算）
4. 确定插入位置：
   - Normal: 在 Location 匹配位置之后（match_info.matched_line_count）
   - Start:  block 开头（索引 0）
   - End:    block 末尾（snapshot_lines.len()）
5. is_raw 行保留原始内容，不计算缩进
6. 调用 content.record_insert(position, new_lines, "NEW") 追加 ContentChange::Insert
7. 不立即修改 ContentBlock — 变更在 Location 关闭时由 apply_changes() 统一生效
```

### 3.4 Delete 操作算法（变更追踪 + 快照匹配）

> 详见 ncs_dev.md §5.4。

```
1. 检查 exec_cmds 中有 Location，否则报错
2. 接收上一个命令的 CmdContent（含 snapshot_lines）
3. 在 snapshot_lines（Location 原始快照）中逐行去空白匹配 DeleteContent
   - 注意：使用 snapshot_lines 而非 lines，确保不受已有 Insert 变更影响
4. 要求连续匹配，不可跳行；首行必须紧邻 Location 最后一行（邻接检查）
5. 匹配成功后调用 content.record_delete(start_idx, end_idx, "DELETE") 追加 ContentChange::Delete
6. Delete:Block → 追加 Delete 变更覆盖整个 snapshot_lines 范围
7. 不立即修改 ContentBlock — 变更在 Location 关闭时统一生效
```

### 3.5 块执行内容提取

> 详见 ncs_dev.md §2.3。

```
行为: 从命令所在行的下一行开始收集内容
终止条件:
  a. 遇到下一个非仅展开命令的 !@Cmd 行
  b. 遇到对应的 @/Cmd
内容中的 !@Raw / !@Get 展开为原始字符，不触发终止
例外: !@Write Raw — 收集到 EOF，所有内容原样，不解析任何命令
```

### 3.6 exec_cmds 生命周期

> 详见 ncs_dev.md §3.2、§6.3。

```
加入: 命令执行完毕 → exec_cmds.push(...)
执行前检查: 新命令的 owner 是否在 exec_cmds 中？不在则报 OwnerNotExecuted 错误
退出 (@/Cmd):
  1. 从 exec_cmds 末尾向前找第一个匹配的 Cmd
  2. 将该 Cmd 到尾部的所有非独立命令全部移除
  3. 若有 Capture 指令，将 CmdContent 存入 pools
脚本结束: 隐式关闭 exec_cmds 中剩余命令，从后向前
```

---

## 4. 重难点与性能考量

### 4.1 从 n_edit 继承的重难点

| 难点 | 说明 | 实现位置 |
|------|------|----------|
| 多语言 Block 解析 | 逐字符扫描、花括号/缩进/字符串/注释处理 | `block.rs` |
| 去空白匹配性能 | 预计算 stripped_content + 首行哈希索引 O(1) | `model.rs`, `matcher.rs` |
| 匹配歧义 | 错误信息中展示候选上下文 | `matcher.rs`, `error.rs` |
| 嵌套 Location 状态管理 | block_stack push/pop + 行号映射 | `engine.rs` |
| New/Delete 后的格式一致性 | reindex() 统一重算 | `model.rs` |
| Unicode 和编码 | 统一 UTF-8，缩进只计 ASCII 空格 | `model.rs` |

### 4.2 NCS 新增重难点

| 难点 | 说明 | 参考 |
|------|------|------|
| 命令注册表设计 | 运行时动态扩展（Include），重名检测、从属关系校验 | ncs_dev.md §3.1 |
| exec_cmds 管理 | 加入/退出/隐式关闭的正确性，与 Capture/Get 的交互 | ncs_dev.md §3.2, §6.3 |
| CmdContent 数据流 | convert/out 的格式一致性，多命令数据传递不丢失信息 | ncs_dev.md §3.3 |
| 块提取终止规则 | 区分行执行/块执行/仅展开命令/Write Raw 例外 | ncs_dev.md §2.3 |
| Bash 安全审查 | sudo/rm/chmod 等危险命令拦截 | ncs_dev.md §5.6 |
| Capture/Get 的数据一致性 | pools 键名冲突、Get 的 like 写入 exec_cmds 的伪装 | ncs_dev.md §5.12 |

### 4.3 性能优化点

| 优化项 | 方法 | 优先级 | 从何继承 |
|--------|------|--------|----------|
| 预计算 stripped_content | FileContent 构建时对每行预存去空白版本 | 高 | n_edit |
| 首行哈希索引 | HashMap<String, Vec<usize>> O(1) 查找 | 高 | n_edit |
| 懒解析 block | 只在 Block 指令时才解析 block 边界 | 中 | n_edit |
| 增量 reindex | 修改后只重算受影响行的元信息 | 低 | — |

---

## 5. 错误信息规范

### 5.1 错误体系

```
NcsError
├── ParseError          # 词法/语法解析错误
├── MatchError          # 匹配相关错误（继承自 n_edit）
├── FileError           # 文件 I/O 错误（继承自 n_edit）
├── EngineError         # 引擎执行错误（继承自 n_edit）
├── RegistryError       # 命令注册表错误（新增）
└── CommandExecError    # 命令执行错误（新增：Bash/Exec/Include）
```

> 完整的错误枚举定义见 ncs_dev.md §7。

### 5.2 错误信息格式

所有错误应包含：
1. **错误类型** — 简短概括
2. **具体原因** — 一句话说明
3. **上下文** — 相关代码块/内容的格式化展示
4. **建议** — 如何修复的提示

示例：
```
Error: 第 5 行: New 命令前缺少 Location 定位
  `@/more` 导致了插入位置不明确。请在此命令之前使用 Location 明确指定操作位置。
  Hint: 在 New 之前添加 !@Location ... 来指定操作范围
  Hint: 或者使用 New:Start / New:End 直接在文件首尾插入
```

### 5.3 输出格式

- 新增行前加绿色 `+`
- 删除行前加红色 `-`
- 上下文行灰色无前缀
- 多个 ContentBlock 修改间用 `~~~~~~~~` 分隔
- 彩色输出检测 `is_terminal`，管道/重定向时自动关闭颜色

---

## 6. 项目结构与文件组织

```
src/
├── main.rs              # CLI 入口（clap 参数解析，脚本路径校验）
├── lib.rs               # 库入口，导出所有公共模块
│
├── lexer.rs             # 词法分析器（!@Cmd / @/Cmd 识别 + 块内容提取）
├── parser.rs            # 语法分析器（Token → Command AST，命令注册表驱动）
├── engine.rs            # 执行引擎（状态机 + exec_cmds 管理 + 命令路由分发）
│
├── registry.rs          # 命令注册表定义 + 内置命令初始化（CommandRegistry, CommandEntry）
├── cmd_content.rs       # 命令间数据传递（CmdContent, CmdLine, CommandResult）
│
├── matcher.rs           # 核心匹配算法（Location 匹配：SearchScope, rows_match）
├── block.rs             # Block 解析器（花括号逐字符扫描 + 缩进层级判断）
│
├── model.rs             # 基础数据结构（Line, FileContent, ContentBlock,
│                        #   LocationContent, NewContent, DeleteContent, LineRange）
├── error.rs             # 所有错误类型集中定义（NcsError + 7 个子枚举）
├── output.rs            # 彩色终端输出 + 错误格式化 + DiffLine
│
├── file_io.rs           # 文件读写工具函数
│
├── commands/            # 各命令的执行实现（一个命令一个文件）
│   ├── mod.rs           # 命令模块入口 + 分发
│   ├── open.rs          # !@Open
│   ├── location.rs      # !@Location
│   ├── new.rs           # !@New
│   ├── delete.rs        # !@Delete
│   ├── raw.rs           # !@Raw
│   ├── bash.rs          # !@Bash
│   ├── exec.rs          # !@Exec
│   ├── read.rs          # !@Read
│   ├── write.rs         # !@Write
│   ├── include.rs       # !@Include
│   ├── work_path.rs     # !@WorkPath
│   └── get.rs           # !@Get
│
└── tests/               # 集成测试
    ├── data/            # 测试用真实源码文件
    ├── scripts/         # .ncs 测试脚本
    └── integration_test.rs
```

---

## 7. 代码风格指导

### 7.1 文件级限制

| 规则 | 上限 | 说明 |
|------|------|------|
| 单文件建议行数 | 800 行 | 含文档注释和空行。超过则优先拆分 |
| 单文件严格上限 | 1200 行 | 任何 `.rs` 文件不得超出（测试文件除外） |
| 一个文件一个核心类型 | — | 允许辅助类型共存，但主类型只能有一个 |

### 7.2 文件级注释

每个 `.rs` 文件开头必须包含：
- 本文件的基本功能说明
- 核心实现逻辑概述（3-5 条要点）
- 对应开发文档的章节引用

```rust
//! 词法分析器 (Lexer)
//!
//! 负责将输入的 .ncs 脚本内容扫描为 Token 流。
//!
//! ## 实现逻辑
//!
//! 1. 逐行读取脚本内容，识别 `!@` 标识符作为命令起始
//! 2. 根据命令名在 CommandRegistry 中查找，确定执行类型（行/块）
//! 3. 块执行命令按终止规则提取后续内容行
//! 4. `!@Raw` 和 `!@Get` 作为仅展开命令，不触发块终止
//! 5. 输出有序的 Token 序列供 Parser 使用
//!
//! ## 对应文档
//!
//! 详见 ncs_dev.md §2.3 "块执行命令的内容提取规则" 及 §4.1 "词法分析"
```

### 7.3 类型与函数文档注释

每个 `struct`、`enum`、`fn`、`trait` 必须带有 `///` 文档注释。

```rust
/// 命令注册表的入口项
///
/// 包含一条命令的完整元信息：名称、类型、模式、从属/所属关系。
struct CommandEntry {
    /// 命令名（全大写，标准化后的名称）
    name: String,
    /// 外部命令的执行路径（内置命令为 None）
    exec_path: Option<PathBuf>,
    /// 命令类型（权限 + 执行方式）
    cmd_type: CommandType,
    /// 模式注册表
    modes: HashMap<String, ModeEntry>,
    /// 从属命令/模式
    subs: Vec<(String, Vec<String>)>,
    /// 所属命令/模式
    owners: Vec<(String, Vec<String>)>,
}
```

### 7.4 数据结构原则

- **一个 struct 只负责一类数据**：不允许一个 struct 同时承载文件和命令状态
- **字段名直白，禁止缩写**：`line_number` 而非 `ln`，`block_stack` 而非 `bs`
- **优先使用 newtype**：`LineNumber(usize)` 而非裸 `usize`
- **所有错误类型集中在 `error.rs`** 中，其他模块通过 `use crate::error::*` 引用

### 7.5 方法原则

- **每个方法不超过 30 行**（不含文档注释和空行）
- **一个方法只做一个实际逻辑**
- **相同逻辑必须复用**：出现第二次相似代码时立即提取为独立方法
- **非测试代码禁止 `unwrap()` / `expect()`**，用 `Result` 传播错误

### 7.6 命令模块组织

每个命令的实现放在 `src/commands/<name>.rs`，遵循统一接口：

```rust
// commands/open.rs — Open 命令
// 详见 ncs_dev.md §5.1

use crate::engine::Engine;
use crate::error::NcsError;

/// Open 命令的执行入口
pub fn execute(engine: &mut Engine, mode: OpenMode, path: &str, args: &HashMap<String, String>) -> Result<(), NcsError> {
    match mode {
        OpenMode::Normal => execute_normal(engine, path, args),
        OpenMode::Dir => execute_dir(engine, path, args),
    }
}

fn execute_normal(engine: &mut Engine, path: &str, args: &HashMap<String, String>) -> Result<(), NcsError> {
    // ...
}
```

### 7.7 禁止项

| 禁止 | 替代做法 |
|------|----------|
| 缩写字段名/变量名 | 全拼：`line_number`, `command_name` |
| 一个 struct 承载多个职责 | 拆分为多个独立 struct |
| 一个方法超过 30 行 | 拆分为多个子方法 |
| 复制粘贴相似逻辑 | 提取为共享方法/函数 |
| 裸 `String` 报错 | 使用 `error.rs` 中定义的具体错误类型 |
| `unwrap()` / `expect()` 在非测试代码 | 用 `Result` 传播 |
| 魔法数字 | 定义为 `const` 并注释含义 |
| 单文件超过 1200 行 | 拆分模块 |

---

## 8. 测试策略

1. **单元测试**：每个模块 `#[cfg(test)] mod tests`
2. **命令执行测试**：`commands/` 下每个命令独立测试
3. **集成测试**：准备 `.ncs` 脚本 + 输入文件 + 预期输出，端到端验证
4. **边界测试**：
   - 空脚本 / 空命令参数
   - 超大文件（10000+ 行）
   - 嵌套 Location 10 层+
   - Dir 模式 + 多层文件匹配
   - Include 重名驳回
   - Bash 安全拦截
   - exec_cmds 退出的正确性
   - Capture → Get 数据一致性
   - Unicode / 编码 / tab-space 混用
