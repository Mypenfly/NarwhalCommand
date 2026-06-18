# Narwhal Command Script

基于 n_edit（详见 `./n_edit_dev.md`）的扩大化重构，将其从单一文件编辑工具扩展为**全系统可操作的命令脚本**。

核心开发理念：**系统一切皆是文本，一切命令操作都是基于字符**。

---

## 1. 关键变化

相对于 n_edit 的三项根本语法变更：

| 项目 | n_edit（旧） | NCS（新） |
|------|-------------|-----------|
| 命令前缀 | `//!@Cmd:` | `!@Cmd` |
| 内容分隔符 | `...` | 无独立分隔符（内容提取终止于下一个命令或 `@/Cmd`） |
| 关闭符号 | `//!@Off:Cmd` | `@/Cmd` |

语法设计思路：`!` 是脚本执行示意符号，`@` 标识定向执行命令。若脚本中嵌入的代码内容（如 New 块中的代码）可能包含 `!@` 字面量，使用 `!@Raw` 转义；若整个脚本主体是原始内容写入，使用 `!@Write Raw` 模式。

---

## 2. 执行流

### 2.1 总体流程

```
用户执行: nc script.ncs
         │
         ▼
    ┌─────────────┐
    │ 1. 文件校验   │ — 检查 .ncs 后缀，否则报错退出
    └──────┬──────┘
           │
           ▼
    ┌─────────────┐
    │ 2. 加载脚本   │ — 读取全文，按行分割为列表
    └──────┬──────┘
           │
           ▼
    ┌─────────────┐
    │ 3. 查找首命令 │ — 逐行扫描，找到第一个 `!@Cmd` 行
    │              │   若未找到 → 抛出空命令脚本错误
    └──────┬──────┘
           │
           ▼
    ┌─────────────┐
    │ 4. 解析命令   │ — 命令解释器（详见 2.2）
    │   (逐行)     │
    └──────┬──────┘
           │
           ▼
    ┌─────────────┐
    │ 5. 执行命令   │ — 按命令和模式执行对应逻辑
    └──────┬──────┘
           │
           ▼
    ┌─────────────┐
    │ 6. 管理状态   │ — 更新 exec_cmds，继承 CmdContent 给从属命令
    └──────┬──────┘
           │  有下一行且非 EOF
           ├──── 回到步骤 4 ────┘
           │
      EOF  │
           ▼
    ┌─────────────┐
    │ 7. 落盘收尾   │ — 隐式 @/Open，将所有操作结果作用于系统
    └─────────────┘
```

### 2.2 命令解释器（步骤 4~5 的展开）

```
输入: 当前行文本
输出: 解析成功的 Command，或解析错误

1. 识别 `!@Cmd` 前缀，按空格分割 → [Cmd, pre_mode, args...]
2. 在 CommandRegistry 中查找 Cmd:
   - 未注册 → 报错 "Not found {Cmd} in line {line_num}"
              "Hint: you may want to use {SimilarCmd}" (基于字符相似度提示)
3. 根据 pre_mode 在命令的模式注册表中匹配:
   - 精确匹配 → 设定 mode
   - 无匹配 → pre_mode 被视为普通参数并入 args，mode 默认为 Normal
4. 根据 mode 获取对应的参数类型表:
   - 解析 args → 必要的参数类型（详见各命令定义）
   - 缺失必要参数 → 报错 "{Cmd} mode {mode} lacks arg(s): ..."
   - 多余参数 → 警告，程序继续执行
5. 构建 Command 对象，交给 Executor 执行
```

### 2.3 块执行命令的内容提取规则

**行执行命令**只读取命令所在行。**块执行命令**需提取多行内容作为参数/操作对象。

提取规则：
- 从命令所在行的下一行开始，收集所有内容
- **终止于以下条件之一**：
  1. 遇到下一个 **非仅展开命令** 的 `!@Cmd` 行（即非 `!@Raw`、非 `!@Get` 的命令）
  2. 遇到对应的 `@/Cmd` 关闭符号
- 提取的内容中，`!@Raw` 和 `!@Get`（仅展开命令）不触发终止，而是**展开为原始字符**融入提取内容

**`!@Write Raw` 模式是唯一例外**：遇到此模式后，从下一行起到文件末尾的**全部内容**直接提取为写入内容，其中的 `!@`、`@/` 等所有标记全部作为原始字符处理，不再解析任何命令。脚本在此 Write 执行完毕后直接退出。

---

## 3. 核心数据结构

### 3.1 命令注册表（CommandRegistry）

程序启动时构建，内置所有命令的基础信息。运行时通过 `!@Include` 动态扩展。

```rust
/// 命令注册表 — 全局管理的命令元信息
struct CommandRegistry {
    /// 命令名 → 命令入口
    entries: HashMap<String, CommandEntry>,
}

struct CommandEntry {
    /// 命令名（全大写，标准化后的名称）
    name: String,
    /// 命令的执行路径（内置命令为 None，外部命令为 Option<PathBuf>）
    exec_path: Option<PathBuf>,
    /// 命令类型（权限类型 + 执行类型）
    cmd_type: CommandType,
    /// 模式注册表：模式名 → 模式信息
    modes: HashMap<String, ModeEntry>,
    /// 从属命令/模式关系表：决定哪些命令可以使用本命令的输出
    /// 格式: [(从属命令名, 从属模式列表)]
    /// 空列表表示该从属命令的所有模式都可以
    subs: Vec<(String, Vec<String>)>,
    /// 所属命令/模式关系表：决定本命令必须在哪些命令之后执行
    /// 格式: [(所属命令名, 所属模式列表)]
    /// 空列表表示该所属命令的所有模式都可以
    owners: Vec<(String, Vec<String>)>,
}

struct CommandType {
    /// 权限类型
    permission: PermissionType,
    /// 执行类型
    execution: ExecutionType,
}

/// 权限类型 — 决定命令的安全性分类
enum PermissionType {
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
enum ExecutionType {
    /// 行执行：只读取命令所在行，不提取后续内容
    LineExec,
    /// 块执行：提取从命令下一行到终止条件的内容
    BlockExec,
    /// 值输出：输出结果不保留，仅打印后丢弃
    ValueOutput,
    /// 流输出：输出结果保留在内存中，可供后续命令使用
    StreamOutput,
    /// 仅展开：遇到时不触发块终止，而是展开为原始字符
    /// （仅用于 !@Raw 和 !@Get）
    ExpandOnly,
}

struct ModeEntry {
    /// 模式名
    name: String,
    /// 参数定义列表
    params: Vec<ParamDef>,
    /// 从属命令/模式（模式级别的覆盖）
    subs: Vec<(String, Vec<String>)>,
}

struct ParamDef {
    /// 参数名
    name: String,
    /// 是否必须
    required: bool,
    /// 参数类型
    param_type: ParamType,
    /// 默认值
    default: Option<String>,
}

enum ParamType {
    String,
    Path,
    Number,
    Bool,
    StringList,
    KeyValue,
}
```

### 3.2 已执行命令列表（exec_cmds）

```rust
/// exec_cmds — 记录所有已执行且仍在生效的命令（及其模式）
///
/// 每个命令执行完成后加入此列表。
/// 后续命令执行前检查其 owner 是否在此列表中，以判断：
/// 1. 本命令是否可以执行
/// 2. 是否可以使用上一个命令输出的 CmdContent
///
/// 脚本从上往下逐行执行，因此从属命令一定在 owner 之后，
/// 退出时可以按栈顺序一次性清理。
struct ExecutedCommand {
    /// 命令名
    cmd_name: String,
    /// 模式名
    mode_name: String,
    /// 是否为独立命令（无 owners 或 owner 已退出）
    is_independent: bool,
}
```

**加入时机**：每个命令执行完成后立即 `exec_cmds.push(...)`。

**退出时机**：遇到 `@/Cmd` 时：
- 在 `exec_cmds` 中从末尾向前查找，找到第一个匹配的 Cmd
- 将从这个 Cmd 开始到列表末尾的、非独立命令的所有条目全部移除
- 独立命令在它的 `@/Cmd` 关闭时一并清除

### 3.3 命令间数据传递 — CmdContent

所有内置命令的数据结构中必须留有 `raw_content` 字段。两个命令之间的数据传递，其数据格式始终是一致的。

```rust
/// 命令间传递的统一数据结构
///
/// 所有内置命令：
/// 1. 必须实现 convert(cmd_content: CmdContent) → 内部数据结构
/// 2. 必须实现 out(内部数据) → CmdContent
/// 3. convert 的输入格式与 out 的输出格式完全一致
struct CmdContent {
    /// 原始文本（供命令解析）
    raw_content: String,

    /// 按行解析的通用数据
    lines: Vec<CmdLine>,

    /// 打印权限，由前一个命令的类型决定
    is_print: bool,

    /// 打印内容（可包含颜色等格式信息）
    result: Vec<CmdLine>,
}

/// 通用的行数据结构
struct CmdLine {
    /// 行号，对应于最初内容的格式
    /// （如果是 Open 得到的，则对应于文件中的行号）
    line_num: usize,
    /// 行内容
    content: String,
}

impl CmdContent {
    /// 序列化为最原始的字符串，用于作为外部命令调用的最后一个参数
    fn send(&self) -> String;

    /// 若 is_print 为 true，将 result 输出到终端
    fn print(&self);
}

/// 命令执行结果
///
/// 每个命令执行完毕后返回此结构。
struct CommandResult {
    /// 输出的 CmdContent（供从属命令使用）
    content: CmdContent,
    /// 是否为流输出（true 则保留在内存，false 则仅打印后丢弃）
    is_stream: bool,
}
```

**数据流动示例 — New 命令**：

1. 上一个命令（通常是 Location）执行 `out()` → 得到 `CmdContent`
2. New 命令执行 `convert(CmdContent)` → 得到内部数据结构（如 ContentBlock）
3. New 执行插入逻辑 → 修改内部数据结构
4. New 执行 `out()` → 得到新的 `CmdContent`
5. `exec_cmds` 中加入 `("New", "Normal")`
6. 下一个命令若 owner 包含 `("New", "Normal")`，则可接收此 `CmdContent`

### 3.4 全局数据池（pools）

```rust
/// 全局数据池 — 存储被 Capture 指令捕获的命令输出
///
/// 用于跨作用域的数据复用。
struct NcsData {
    pools: HashMap<String, CmdContent>,
}
```

---

## 4. 命令解析与 AST

### 4.1 词法分析（Lexer）

```
输入: .ncs 脚本文本
输出: Vec<Token>

逐行扫描：
1. 跳过非 `!@` 开头的行（这些是块执行命令的内容，在前一个命令提取时已消费）
2. 遇到 `!@` 开头 → 识别命令名、模式、参数
3. 遇到 `@/` 开头 → 识别为关闭符号 Token
4. 对于块执行命令，提取后续内容行直到终止条件
```

```rust
enum Token {
    /// 命令语句
    Command {
        /// 命令名
        name: String,
        /// 模式名（如 "Normal", "Block", "Dir" 等）
        mode: String,
        /// 参数列表（键值对）
        args: HashMap<String, String>,
        /// 行号
        line: LineNumber,
        /// 块执行命令的内容行（行执行为空）
        content_lines: Vec<String>,
        /// 是否为块执行
        is_block: bool,
    },
    /// 关闭符号
    Close {
        /// 关闭的命令名
        name: String,
        /// 行号
        line: LineNumber,
    },
    /// Capture 指令：捕获命令输出到 pools
    /// 格式: @/Open | Capture pool_name
    Capture {
        /// 存入 pools 的键名
        pool_name: String,
        /// 行号
        line: LineNumber,
    },
}
```

### 4.2 语法分析（Parser）

```
输入: Vec<Token>
输出: Vec<Command>

1. 对每个 Command Token，在 CommandRegistry 中查找对应的命令定义
2. 校验模式是否存在于该命令的模式注册表中
3. 根据模式定义的参数表解析 args
4. 对块执行命令，将其 content_lines 解析为对应内容结构
5. 校验命令的 owner 是否在 exec_cmds 中（运行时校验，非 Parser 阶段）
```

```rust
enum Command {
    Open {
        mode: OpenMode,
        path: String,
        args: HashMap<String, String>,
        // 块执行内容（Normal 模式无内容，因路径参数已包含文件信息）
        content_lines: Vec<String>,
    },
    Location {
        mode: LocationMode,
        content: Option<LocationContent>,
        args: HashMap<String, String>,
    },
    New {
        mode: NewMode,
        content: NewContent,
    },
    Delete {
        mode: DeleteMode,
        content: Option<DeleteContent>,
    },
    Raw {
        content: String,
    },
    Bash {
        command: String,
    },
    Exec {
        command: String,
    },
    Read {
        path: String,
        args: HashMap<String, String>,
    },
    Write {
        mode: WriteMode,
        path: String,
        content: Option<String>,
    },
    Include {
        path: String,
        args: HashMap<String, String>,
    },
    WorkPath {
        path: String,
    },
    Get {
        pool_name: String,
        like: Option<String>,
    },
    Close {
        name: String,
    },
}

enum OpenMode { Normal, Dir }
enum LocationMode { Normal, Block, Path }
enum NewMode { Normal, Start, End }
enum DeleteMode { Normal, Block }
enum WriteMode { Normal, Raw }
```

---

## 5. 命令定义

### 5.1 Open

```
!@Open [mode] <path> [options...]
```

**命令类型**: 文件系统读写、行执行（流输出）

**从属命令/模式**: Location（Normal、Block、Path）、New（Start、End）

| 模式 | 说明 |
|------|------|
| `Normal`（默认） | 打开文本文件，加载为 `CmdContent` |
| `Dir` | 打开目录，递归扫描得到 `RawPaths` |

**Normal 模式参数选项**:

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `start` | Number | 1 | 起始行号（1-based），读取从该行开始 |
| `end` | Number | 文件末尾 | 结束行号，读取到该行为止 |

**Dir 模式参数选项**:

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `depth` | Number | 3 | 递归深度 |
| `ignore` | String | `"*.bin"` | 忽略的文件/目录模式（支持正则，`,` 分割） |
| `filter` | String | 空 | 只保留匹配的文件类型（如 `"*.py,*.rs"`） |

**执行流**:
1. 识别路径，若路径不存在则报错
2. 若路径是文本文件（或 Normal 模式）→ 打开文件，以 `start`/`end` 限定范围，读入内存为 `CmdContent`
3. 若路径是目录（Dir 模式）→ 递归扫描，得到目录下所有文件路径列表 `RawPaths`，保存为 `CmdContent`
4. `CmdContent.lines` 记录文件行号和内容
5. 执行完成，加入 `exec_cmds`

**关闭**：遇到 `@/Open` 时，将所有修改写回原文件，清除 `exec_cmds` 中此 Open 及其从属命令。

---

### 5.2 Location

```
!@Location [mode] [args...]
定位内容行（块执行时）
...
```

**命令类型**: 文件系统读写、块执行、流输出

**所属命令**: Open

**从属命令/模式**: New（Normal）、Delete（Normal、Block）

| 模式 | 说明 |
|------|------|
| `Normal` | 基于内容和 diff_taps 匹配定位（与 n_edit 核心逻辑一致） |
| `Block` | 匹配后使用 BlockParser 获取精确代码块边界 |
| `Path` | 指定文件路径，在该文件中执行 Normal 模式定位 |

**Normal / Block 模式执行流**:
1. 若前一个 Open 为 Normal 模式 → 在当前文件内容中匹配（与 n_edit 逻辑一致）
2. 若前一个 Open 为 Dir 模式 → 遍历 `RawPaths` 中的所有文本文件，逐个执行匹配
3. 匹配流程：首行去空白匹配（O(1) 哈希索引）→ diff_taps + 去空白逐行筛选 → 唯一性校验
4. Block 模式额外调用 BlockParser 确定精确块边界
5. 匹配结果保存为 `LocationResult`（用 `CmdContent` 表示，`is_print = true`）
6. 执行完成，加入 `exec_cmds`

**Path 模式执行流**:
1. 若前一个 Open 为 Normal → 发布非必要模式警告，继续执行
2. 验证路径是否在 Open 的 `RawPaths` 之中，不在则报错
3. 全量打开该文件，后续按 Normal 模式执行

> **说明**：n_edit 原有的行号定位（`@66,120`）已移除。原因是多次 New/Delete 后行号偏移会破坏精确性。替代方案是使用多层嵌套 Location 来精确定位。Open 的 `start`/`end` 参数可用于缩小初始文件范围。

**流输出**：遇到 `@/Location` 时触发终端打印，输出带文件路径和行号的 `LocationResult`，内容为灰色。

---

### 5.3 New

```
!@New [mode]
插入内容行...
```

**命令类型**: 文件系统写、块执行

**所属命令**: Normal 模式属于 Location、Start 和 End 模式属于 Open

**从属命令/模式**: 无

| 模式 | 说明 |
|------|------|
| `Normal` | 在 Location 匹配位置之后插入 |
| `Start` | 在文件/Block 开头插入 |
| `End` | 在文件/Block 末尾插入 |

**执行流**（与 n_edit 逻辑一致）:
1. 检查 `exec_cmds` 中是否有所属命令，若无则报错
2. 接收上一个命令 `out()` 得到的 `CmdContent`
3. `convert()` 为内部数据结构
4. 按模式在指定位置插入内容
5. 内容每行的 diff_taps 作为绝对缩进量，以插入位置 taps 为基准计算最终缩进
6. `is_raw` 行保留原始格式不计算缩进
7. 修改完成后 `reindex()` 重排行号
8. `out()` 得到新的 `CmdContent`
9. **同步修改** `LocationResult`：新增行前加 `+`，内容标绿色

> **约束**：`!@New Normal` 之前最近的一个命令不能是 `@/more`（已移除）或任何会切断 Location 状态的 Token。前一个 Location 必须仍在 `exec_cmds` 中。

---

### 5.4 Delete

```
!@Delete [mode]
匹配内容行...
```

**命令类型**: 文件系统写、块执行

**所属命令**: Location

**从属命令/模式**: 无

| 模式 | 说明 |
|------|------|
| `Normal` | 在 ContentBlock 内匹配并删除连续行 |
| `Block` | 删除整个 ContentBlock（要求 Location 也使用 Block 模式） |

**执行流**（与 n_edit 逻辑一致）:
1. 检查 `exec_cmds` 中是否有 Location
2. 接收上一个 Location 命令 `out()` 得到的 `CmdContent`
3. 在 ContentBlock 内逐行去空白匹配删除内容
4. 要求连续匹配，不可跳行
5. 删除要求紧邻 Location 最后一行
6. 删除后 `reindex()` 重排行号
7. **同步修改** `LocationResult`：删除行前加 `-`，内容标红色

---

### 5.5 Raw

```
!@Raw <raw_content>
```

**命令类型**: 程序执行、行执行、仅展开

**所属命令**: New、Delete

**从属命令/模式**: 无

**执行流**:
1. Raw 命令将其内容作为**字面量**融入上一个 New 或 Delete 命令
2. 在块提取时，`!@Raw` 不会触发终止，其内容展开为原始字符
3. New 上下文中：`is_raw = true`，插入时保留原始格式
4. Delete 上下文中：`is_raw = true`，按字面量逐字符匹配

---

### 5.6 Bash

```
!@Bash <bash_command>
```

**命令类型**: 程序执行、行执行、流输出

**所属命令**: 无（独立命令）

**从属命令/模式**: 无

**执行流**:
1. 提取完整的 bash 命令字符串
2. 通过 `bash -c "<command>"` 执行
3. 捕获 stdout/stderr 为 `CmdContent`
4. 终端打印结果
5. **安全审查**：截断 `sudo`、`rm -rf /` 等高危命令，报错拒绝执行

---

### 5.7 Exec

```
!@Exec <command>
```

**命令类型**: 程序执行、行执行、值输出

**所属命令**: 无（独立命令）

**从属命令/模式**: 无

**与 Bash 的区别**：Exec 通过 `script -c "<command>"` 执行，直连终端，支持彩色输出、流式输出和交互式命令。但结果是**值输出**（仅打印，不保留）。

---

### 5.8 Read

```
!@Read [mode] <path> [options...]
```

**命令类型**: 文件系统读、行执行、值输出

**所属命令**: 无（独立命令）

**从属命令/模式**: 无

**模式和参数**与 `!@Open` 完全一致。区别：
- 值输出，结果不保留
- 输出的文件内容**带高亮和行号**
- 输出的路径**带高亮和树状结构**
- 基本等同于一个独立 CLI 工具

---

### 5.9 Write

```
!@Write [mode] <path>
写入内容...
@/Write
```

**命令类型**: 文件系统写、块执行、值输出

**所属命令**: 无（独立命令）

**从属命令/模式**: 无

| 模式 | 说明 |
|------|------|
| `Normal` | 块内容写入指定文件 |
| `Raw` | 从下一行到 EOF 的全部内容原样写入 |

**Normal 模式执行流**:
1. 检查路径是否为文件类型（非目录）
2. 路径不存在则创建一连串路径（包括父目录）
3. 提取块内容（终止于 `@/Write` 或下一个非仅展开命令）
4. 内容中的 `!@Raw` 展开为原始字符
5. 写入文件，打印 `success write to {path}` 或 `overwrite`

**Raw 模式执行流**:
1. 从命令行的下一行开始，到文件末尾的所有内容全部收集
2. 其中的 `!@`、`@/`、`!@Raw`、`!@Get` 等**全部作为原始字符**，不做任何解析
3. 收集完毕后写入目标文件
4. **脚本执行完毕，程序退出**（Raw 模式是终端操作）

---

### 5.10 Include

```
!@Include <exec_path> [options...]
```

**命令类型**: 程序执行、行执行

**所属命令**: 无（独立命令）

**从属命令/模式**: 由 Include 导入的所有外部命令

**参数选项**:

| 参数 | 必填 | 默认值 | 说明 |
|------|:---:|--------|------|
| `alias` | 是 | — | 外部命令的别名。**禁止与任何内置命令重名**，重名则报错 |
| `block` | 否 | `false` | 是否支持块执行 |
| `type` | 否 | `[OnlyPrint]` | 命令类型（可多选）: `OnlyPrint`、`StreamOutput`、`Request`、`SavePrint` |
| `exec` | 否 | `default` | 执行方式：`default`（Rust exec）、`script`（`script -c`）、`bash`（`bash -c`） |
| `owners` | 否 | 自动填充 | 所属命令列表。**默认值 `[include(cmd)]` 始终存在**，传入自定义 owners 时也会自动追加 |
| `subs` | 否 | 空 | 从属命令/模式列表 |

**执行流**:
1. 读取 Include 命令，提取所有参数
2. 校验 `alias` 不重名
3. 将此命令注册到 CommandRegistry 中

**导入的外部命令的调用方式**:
- **行执行**：将 `!@Cmd` 展开为导入的路径，通过指定方式执行
- **块执行**：提取块内容，展开 `!@Raw`，作为最后一个参数（字符串类型，用引号包裹）传入
- **流输出**：建立执行结果保存，供其他命令使用

---

### 5.11 WorkPath

```
!@WorkPath <path>
```

**命令类型**: 无

**所属命令**: 无（独立命令）

**从属命令/模式**: 无

**执行流**:
1. 识别路径——必须存在，否则报错
2. 若路径为文件，取父目录
3. 更改工作路径，影响后续所有 `./`、`../` 的展开，以及外部命令（Bash/Exec）的执行路径传递

**默认值**：脚本中未遇到 `!@WorkPath` 时，工作路径取脚本文件的父目录。

---

### 5.12 Get

```
!@Get <pool_name> [like=[!@Cmd ...]]
```

**命令类型**: 程序执行、行执行、仅展开

**所属命令**: 无（独立命令）

**从属命令/模式**: 无

**执行流**:
1. 从全局 `pools` 中提取 `pool_name` 对应的 `CmdContent`
2. 若指定 `like` 选项：
   - 在 `exec_cmds` 中写入 `like` 中指定的命令和模式（伪装成该命令的输出）
   - 下一个命令由于在 `exec_cmds` 中找到 owner，可接收此 `CmdContent`
   - 遇到对应的 `@/Cmd` 时，执行和正常关闭相同的逻辑（如 `@/Open` 会写回文件）
3. 若不指定 `like`：
   - 展开 `CmdContent.raw_content` 作为纯文本
   - **不记录到** `exec_cmds`（Get 本身被视同 `!@Raw` 仅展开）
   - 或作为行执行命令的参数中的占位符 `{}` 的替换源:
     ```
     !@Get open_result like=[!@Bash echo "{}"]
     ```

---

## 6. 捕获、流输出与数据传递

### 6.1 Capture 指令

```
@/Open | Capture pool_name
```

将命令关闭时的 `CmdContent` 存入全局 `pools` 中，键为 `pool_name`。

- Capture 发生在 `@/Cmd` 行中，通过管道语法 `| Capture <name>` 声明
- 被捕获的命令输出在退出 `exec_cmds` 前复制到 `pools`
- 可供后续 `!@Get` 命令提取复用

### 6.2 流输出与值输出

| 类型 | 行为 | 使用场景 |
|------|------|----------|
| **流输出** | 执行结果保留在 `exec_cmds` 和内存中，`@/Cmd` 时触发打印，继续传递给从属命令 | Open、Location、Bash |
| **值输出** | 执行结果仅打印后丢弃，不保留不传递 | Read、Exec、Write |

### 6.3 exec_cmds 管理规则

1. **加入**：命令执行完成，立即加入 `exec_cmds`
2. **执行前检查**：解析到新命令时，检查其 `owner` 是否存在于 `exec_cmds` 中
   - 若不在 → 报命令所属关系错误
3. **退出（@/Cmd）**：
   - 在 `exec_cmds` 中从末尾向前查找第一个匹配的 Cmd
   - 将这个 Cmd 到尾部的所有**非独立命令**全部移除
   - 若有 Capture 指令，被清除前将对应命令的 `CmdContent` 存入 `pools`
4. **独立命令**：`owners` 为空的命令，其 `@/Cmd` 直接清除自身
5. **隐式关闭**：脚本结束时，对 `exec_cmds` 中剩余命令从后向前依次执行关闭

### 6.4 数据传递路径总结

```
Open(Dir) ──out()──► CmdContent ──convert()──► Location(Normal) ──out()──► CmdContent
                                                                              │
                    ┌─────────────────────────────────────────────────────────┘
                    ▼
              CmdContent ──convert()──► New(Normal) ──out()──► CmdContent
                    │                                              │
                    │     ┌────────────────────────────────────────┘
                    │     ▼
                    │   CmdContent ──convert()──► Delete(Normal)
                    │
                    ▼
              Capture → pools["result"] ──Get──► CmdContent ──► 后续命令
```

---

## 7. 错误处理体系

沿用并扩展 n_edit 的 `NEditError` 体系，重命名为 `NcsError`。

### 7.1 根错误类型

```rust
enum NcsError {
    /// 词法/语法解析错误
    Parse(ParseError),
    /// 匹配相关错误
    Match(MatchError),
    /// 文件 I/O 错误
    File(FileError),
    /// 引擎执行错误
    Engine(EngineError),
    /// 命令注册表错误（新增）
    Registry(RegistryError),
    /// 命令执行错误（新增：Bash、Exec 等外部命令失败）
    CommandExec(CommandExecError),
}
```

### 7.2 新增错误类型

```rust
enum RegistryError {
    /// 命令未注册
    CommandNotFound {
        cmd_name: String,
        line: LineNumber,
        /// 基于相似度的候选命令名
        suggestion: Option<String>,
    },
    /// 模式未注册
    ModeNotFound {
        cmd_name: String,
        mode_name: String,
        line: LineNumber,
    },
    /// 所属命令不在 exec_cmds 中
    OwnerNotExecuted {
        cmd_name: String,
        owner_name: String,
        line: LineNumber,
    },
    /// Include alias 重名
    AliasConflict {
        alias: String,
        existing_cmd: String,
        line: LineNumber,
    },
}

enum CommandExecError {
    /// Bash/Exec 执行失败
    ExecutionFailed {
        command: String,
        exit_code: Option<i32>,
        stderr: String,
    },
    /// 安全审查拒绝
    SecurityDenied {
        command: String,
        reason: String,
    },
    /// 超时
    Timeout {
        command: String,
        timeout_secs: u64,
    },
    /// Include 外部命令失败
    IncludeFailed {
        path: String,
        reason: String,
    },
}
```

### 7.3 错误输出格式

沿用 n_edit 的错误输出格式：
```
Error: <title>
  <detail>
  Hint: <hint_1>
  Hint: <hint_2>
```

终端输出时使用颜色：
- `Error:` 红色加粗
- 标题：黄色
- 详情：灰色
- `Hint:` 绿色加粗
- 管道/重定向时自动关闭颜色

---

## 8. 命令注册表初始化

程序启动时，以下命令自动注册到 CommandRegistry：

| 命令 | 权限类型 | 执行类型 |
|------|---------|---------|
| Open | FileRead + FileWrite | LineExec + StreamOutput |
| Location | FileRead + FileWrite | BlockExec + StreamOutput |
| New | FileWrite | BlockExec |
| Delete | FileWrite | BlockExec |
| Raw | — | LineExec + ExpandOnly |
| Bash | ProgramExec | LineExec + StreamOutput |
| Exec | ProgramExec | LineExec + ValueOutput |
| Read | FileRead | LineExec + ValueOutput |
| Write | FileWrite | BlockExec + ValueOutput |
| Include | ProgramExec | LineExec |
| WorkPath | None | LineExec |
| Get | — | LineExec + ExpandOnly |

---

## 9. 模块架构

```
src/
├── main.rs              # CLI 入口，参数解析（clap）
├── lib.rs               # 库入口，导出所有公共模块
├── lexer.rs             # 词法分析器（!@Cmd 识别 + 块内容提取）
├── parser.rs            # 语法分析器（Token → Command AST）
├── engine.rs            # 执行引擎（状态机 + exec_cmds 管理 + 命令分发）
├── registry.rs          # 命令注册表（CommandRegistry 定义 + 初始化）
├── cmd_content.rs       # CmdContent + CmdLine（命令间数据传递）
├── matcher.rs           # 核心匹配算法（Location 匹配，从 n_edit 迁移）
├── block.rs             # Block 解析器（花括号/缩进，从 n_edit 迁移）
├── model.rs             # 基础数据结构（Line, ContentBlock, FileContent 等）
├── error.rs             # 所有错误类型集中定义
├── output.rs            # 彩色终端输出 + 错误格式化
├── commands/            # 各命令的执行实现
│   ├── mod.rs
│   ├── open.rs
│   ├── location.rs
│   ├── new.rs
│   ├── delete.rs
│   ├── raw.rs
│   ├── bash.rs
│   ├── exec.rs
│   ├── read.rs
│   ├── write.rs
│   ├── include.rs
│   ├── work_path.rs
│   └── get.rs
└── file_io.rs           # 文件读写工具函数
```

---

## 10. 开发阶段

### Phase 1: NCS 骨架
- [ ] 建立新项目骨架（Cargo.toml, lib.rs, main.rs）
- [ ] 定义 `CmdContent`、`CommandRegistry`、`exec_cmds` 等核心数据结构
- [ ] 实现新的 Lexer（`!@Cmd` 语法识别）
- [ ] 实现新的 Parser（命令注册表驱动）

### Phase 2: 从 n_edit 迁移核心命令
- [ ] 迁移 Open（新增 Dir 模式、start/end 参数）
- [ ] 迁移 Location（移除行号定位，新增 Path 模式）
- [ ] 迁移 New / Delete / Raw（保持核心逻辑不变）
- [ ] 迁移 matcher、block 模块
- [ ] 实现 `@/Cmd` 关闭符号 + exec_cmds 管理

### Phase 3: 新增命令
- [ ] Bash / Exec
- [ ] Read / Write（含 Raw 模式）
- [ ] Include（命令注册表动态扩展）
- [ ] WorkPath

### Phase 4: 数据传递系统
- [ ] CmdContent convert/out/send/print 方法
- [ ] Capture 指令 + Get 命令
- [ ] pools 全局数据池

### Phase 5: 错误处理 + 输出
- [ ] 扩展错误类型体系
- [ ] 彩色终端输出
- [ ] verbose / quiet 模式

---

## 11. 与 n_edit 的关系

- NCS 是对 n_edit 的**扩大化重构**，非完全废止
- n_edit 的核心资产被保留和迁移：
  - Location 匹配算法（去空白 + diff_taps + 哈希索引）
  - Block 解析器（花括号/缩进）
  - ContentBlock / FileContent 数据结构
  - diff 输出格式（+ 绿色 / - 红色）
  - 错误提示风格（title + detail + hints）
- n_edit 的首行哈希索引 O(1) 优化、diff_taps 校验、嵌套 Location、reindex 机制全部保留
- n_edit 的 parser 歧义保护（Location-Separator 状态追踪）在新系统中通过 exec_cmds 机制实现等效保护
