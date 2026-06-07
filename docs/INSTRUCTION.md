# N_Edit 实现指令

## 1. 总体设计路径

### 1.1 架构总览

```
                    ┌─────────────┐
   *.ned / *.nd ──▶ │  CLI 入口    │
                    └──────┬──────┘
                           │
                    ┌──────▼──────┐
                    │  词法解析器   │  —— 扫描 `//!@` 标识符，切分为 Token 流
                    └──────┬──────┘
                           │
                    ┌──────▼──────┐
                    │  语法解析器   │  —— 将 Token 流组装为 AST（命令序列）
                    └──────┬──────┘
                           │
                    ┌──────▼──────┐
                    │  执行引擎    │  —— 维护状态机，逐条执行 AST 节点
                    └──────┬──────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
        ┌─────▼─────┐ ┌───▼───┐ ┌─────▼─────┐
        │  文件 I/O  │ │ 匹配器 │ │Block解析器│
        └───────────┘ └───────┘ └───────────┘
```

### 1.2 分阶段实现路径

| 阶段 | 内容 | 说明 |
|------|------|------|
| **Phase 1** | Open / Location（纯文本匹配）/ Off | 最基础的读取-定位-写回流程，验证核心匹配算法 |
| **Phase 2** | New / Delete（在 ContentBlock 内操作） | 基于 Phase 1 的 ContentBlock，实现修改操作 |
| **Phase 3** | Location:Block + Delete:Block | 代码块识别（花括号/缩进），支持整块增删 |
| **Phase 4** | 嵌套 Location | Location 在 ContentBlock 内再次定位，递归缩小范围 |
| **Phase 5** | 行号 Location（`@66,120`） | 按指定行号直接定位，跳过匹配流程 |
| **Phase 6** | 错误美化 / 彩色输出 / 格式化 | 报错信息优化，终端高亮输出 |
| **Phase 7** | 扩展命令（Include 等） | 后续迭代 |

### 1.3 命令状态机

```
                  ┌──────────────────────────────────────────┐
                  │              全局状态                     │
                  │  file: FileContent                       │
                  │  block_stack: Vec<ContentBlock>           │
                  │  current_cmd: Option<Command>             │
                  └──────────────────────────────────────────┘

  Open ──▶ Location ──▶ [Location]* ──▶ New ──▶ ... ──▶ Off
            │                            │
            │                            ├──▶ Delete
            │                            ├──▶ Raw
            │                            │
            └──▶ [Location:Block] ───────┘
```

**状态约束：**
- `New` / `Delete` 之前必须存在至少一个 `Location`（`block_stack` 非空），或使用 `New:Start` / `New:End` 特殊指令
- `Delete:Block` 要求前一个 `Location` 也使用了 `Block` 指令
- `Raw` 命令用于写入字面量 `...` 避免歧义，与 New 配合使用（详见 难点5）
- `Off:Location` 弹出栈顶 ContentBlock，将修改写回上一层
- `Off:Open` 弹出所有 block，将最终 FileContent 写回磁盘
- 脚本末尾若无任何 Off 命令，自动执行隐式 `Off:Open` 写回文件

---

## 2. 详细数据结构

### 2.1 命令解析阶段 (AST)

```rust
/// 一条完整的命令语句（对应一行或多行 `//!@` 内容）
enum Command {
    Open {
        file_path: String,
    },
    Location {
        block: bool,                              // 是否为 Location:Block
        line_range: Option<(usize, usize)>,        // Phase 5: 行号定位 @start,end
        content: LocationContent,                  // 用于匹配的定位内容
    },
    New {
        position: NewPosition,                    // Normal / Start / End
        content: NewContent,
    },
    Delete {
        block: bool,                              // 是否为 Delete:Block
        content: Option<DeleteContent>,            // 非 Block 时有匹配内容
    },
    Raw {
        content: String,                          // 字面量内容（如 "..."）
    },
    Off {
        target: OffTarget,                        // Open / Location / New
    },
}
```

```rust
enum NewPosition {
    Normal,          // 插入到 Location 最后一行之后
    Start,           // 文件开头
    End,             // 文件末尾
}

enum OffTarget {
    Open,
    Location,
    New,
}
```

### 2.2 Location 匹配数据结构

```rust
/// 从 Location 命令后的内容解析得到（提取直到 `...` 分隔符或下一个 `//!@`）
/// `...` 是全局分隔符：同时作为 Location/New/Delete 的内容提取终止符
struct LocationContent {
    lines: Vec<LocationLine>,
}

struct LocationLine {
    index: usize,             // 从 0 开始的序号（第一行为 0）
    diff_taps: Option<usize>, // 缩进差异量（以 index=0 行为基准）
    content: String,          // 原始内容（保留缩进和空格）
    line_num: Option<usize>,  // 对应原文行号，未解析时为 None
}
```

```rust
/// 第一行纯字符匹配后得到的候选结果集合
struct FirstMatchContents {
    contents: Vec<FirstMatchContent>,
}

struct FirstMatchContent {
    start_line: usize,       // 匹配到的首行在原文中的行号
    lines: Vec<MatchLine>,   // 从 start_line 起向后取与 LocationContent 等行数的内容
}

struct MatchLine {
    line_num: usize,         // 原文行号
    taps: usize,             // 该行在原文中的缩进量（空格数）
    diff_taps: usize,        // 缩进差异（以本组第一行为基准）
    content: String,         // 原始内容
}
```

### 2.3 文件 / Block 数据结构

```rust
/// Open 命令解析文件后得到的完整文件内容
struct FileContent {
    lines: Vec<Line>,
}

/// 一个代码块（可能为整个文件、一个方法、一个循环体等）
struct ContentBlock {
    start_line: usize,
    lines: Vec<Line>,
}

struct Line {
    line_num: usize,    // 行号（从 0 或 1 开始，统一约定）
    taps: usize,        // 缩进空格数量
    diff_taps: usize,   // 相对于 block 首行的缩进差异
    content: String,    // 原始行内容
}
```

### 2.4 New / Delete 数据结构

```rust
struct NewContent {
    lines: Vec<NewLine>,
}

struct NewLine {
    diff_taps: usize,   // 相对于插入位置的缩进差异
    content: String,    // 去除首部缩进后的内容（保留内部空格）
    is_raw: bool,       // 是否为 Raw 命令指定的字面量（此时 diff_taps 被忽略）
}

/// Delete 命令后的匹配内容（到 `...` 分隔符或下一个命令为止）
struct DeleteContent {
    lines: Vec<DeleteLine>,
}

struct DeleteLine {
    content: String,    // 用于匹配的原始文本
    is_raw: bool,       // 是否为 Raw 命令指定的字面量
}
```

---

## 3. 关键算法流程

### 3.1 Location 匹配算法（核心）

```
输入: file_content: &FileContent, loc_content: &LocationContent
输出: ContentBlock 或 Error

1. 取 loc_content.lines[0].content，
   去除所有空白字符 → stripped_first_line

2. 遍历 file_content.lines，对每一行：
     移除空白字符 → stripped_line
     若 stripped_line == stripped_first_line  →  记录行号，作为候选起点

3. 对每个候选起点：
     a. 从 file_content 中取与 loc_content 等行数的内容
     b. 构建 FirstMatchContent（逐行计算 taps, diff_taps）
     c. 跳过空行后，逐行比对：
        - content 去空白后必须完全一致
        - diff_taps 必须完全一致
     任一条件不满足则丢弃此候选

4. 若剩余候选数 != 1 → 抛出匹配异常（附带详细诊断信息）

5. 基于唯一匹配结果，确定 ContentBlock 边界：
     ※ 详见 3.2 Block 解析算法

6. 返回 ContentBlock
```

### 3.2 Block 解析算法（简化的类 LSP 代码分级识别）

> 目标：通过逐字符扫描构建代码层级树（tree），从而准确识别代码块边界。
> 花括号语言和缩进语言采用不同的树构建策略，但统一输出 ContentBlock。

```
输入: first_match: &FirstMatchContent, file_content: &FileContent
输出: ContentBlock

1. 判断目标语言类型（按优先级）：
   a. 检查首行及上下文是否包含 '{' '}' 结构 → 花括号语言
   b. 检查内容 diff_taps 是否不全为 0（有缩进层级）→ 缩进语言
   c. 其他 → Block 不可解析（纯文本 / Markdown 等）

2. 花括号语言 — 逐字符扫描建树：
   a. 从首行首个 '{' 开始扫描，维护：
      - depth: 括号嵌套深度（初始 depth = 1，因为已进入首行 block）
      - in_string: 是否在字符串字面量内（遇到 \" 或 \\ 转义需跳过）
      - in_comment: 是否在注释内（// 行注释、/* */ 块注释）
   b. 向后逐字符推进：
      - 遇到 '{' 且不在字符串/注释中 → depth += 1
      - 遇到 '}' 且不在字符串/注释中 → depth -= 1
      - depth == 0 → 当前行为 Block 结束行
   c. 提取 [start_line, end_line] 作为 Block

3. 缩进语言（Python / YAML 等）— 缩进层级建树：
   a. 取首行的 taps 为基准 indent
   b. 向后逐行扫描，跳过空行和纯注释行：
      - taps > indent  → 在 Block 内，继续
      - taps <= indent → Block 结束（此行不属于 block）
   c. 提取完整 Block

4. 若 Block 不可解析（纯文本等）：
     返回从 start_line 到文件末尾的所有内容作为 ContentBlock
     此时拒绝 Block 指令（Location:Block / Delete:Block 报错）
```

### 3.3 New 插入算法

```
输入: block: &mut ContentBlock, new_content: &NewContent, insert_after_line: usize
输出: 修改后的 ContentBlock

前提: insert_after_line 是 Location 匹配到的最后一行
      在 block 中的行号（不是 file_content 中的行号）

1. 找到 insert_after_line 在 block 中的索引 idx

2. 取 block.lines[idx] 的 taps 和 diff_taps 作为基准

3. 对 new_content.lines 中的每一行：
     - 若 is_raw → 直接使用 content 作为字面量，保持原有缩进，插入到 idx+1 位置
     - 否则 → 实际 taps = 基准 taps + 该行 diff_taps
              构建 Line { content, taps, ... }
     插入到 block.lines 的 idx+1 位置（按序插入）

4. 重新计算插入行之后所有行的 line_num（递增偏移）

5. 自检：对插入后的 block 重新执行 Location 匹配，
   确认新增内容格式正确（格式自检可跳过，仅开发阶段使用）
```

### 3.4 Delete 操作算法

```
输入: block: &mut ContentBlock, del_content: &DeleteContent
输出: 修改后的 ContentBlock

1. 在 block.lines 中按 Location 匹配逻辑逐行匹配 del_content
   - 要求匹配的行连续，不可跳行
   - `...` 作为 Delete 内容提取终止符，不参与匹配
   - is_raw 标记的行：按字面量逐字符匹配（不触发分隔符逻辑）

2. 若匹配失败，抛出错误（附带当前 block 内容）

3. 删除匹配到的连续行区间

4. 对剩余行重新分配 line_num（基于首行的 line_num 递增）

5. 输出删除结果（红色 "-" 标注）
```

### 3.5 行号 Location（Phase 5）

```
输入: line_range: (start, end)
输出: ContentBlock

1. 验证 start > 0, end >= start

2. 直接从 FileContent / 当前 ContentBlock 中提取
   lines[start-1 .. end]（行号 1-based → 0-based 索引）

3. 按常规流程确定 ContentBlock 边界
   （可结合 Block 解析，或直接以 [start, end] 为精确范围）

4. 后续 New/Delete 操作与匹配模式一致
```

### 3.6 Raw 命令（字面量转义）

```
用途: 在 New/Delete 内容中写入字面量 "..."，避免被解析为分隔符

1. 词法分析阶段：
     遇到 `//!@Raw:` → 提取后续内容作为字面量，标记为 RawToken

2. 语法分析阶段：
     RawToken 出现在 New 内容块中 → 作为 NewContent.lines 中的一行
     RawToken 出现在 Delete 内容块中 → 作为 DeleteContent.lines 中的一行

3. 执行阶段：
     New 插入时: Raw 指定的字面内容直接写入（不解析、不触发分隔符逻辑）
     Delete 匹配时: Raw 指定的字面内容参与字符匹配（等同于普通匹配行）
```

---

## 4. 重难点与性能考量

### 4.1 重难点

#### 难点1：多语言 Block 解析（简化的类 LSP 树构建）
- **目标**：通过逐字符扫描，构建代码层级树，准确识别 block 边界
- **花括号语言**（Rust, C, JS, Java...）：
  - 关键难点：必须正确跳过字符串字面量和注释中的 `{` `}`
  - 需要维护 `in_string` / `in_comment` 状态，处理转义（`\"`, `\\`）
  - 行注释 `//` 和块注释 `/* */` 都需要识别
- **缩进语言**（Python, YAML）：
  - tab/space 混用会导致缩进计算错误
  - 空行和纯注释行需要跳过而不干扰缩进层级判断
- **取舍**：先支持花括号语言（带字符串/注释跳过）和纯缩进判断。Ruby `do...end`、Lua `do...end`、Shell `if...fi` 等特殊块语法暂不支持。

#### 难点2：去空白匹配的性能
- **问题**：对每行做 `remove_whitespace()` 操作，大文件下开销可观
- **优化方向**：
  - 预计算：打开文件时对每行预存 `stripped_content` 字段
  - 首行匹配用哈希预过滤（构建 HashMap<String, Vec<usize>> 索引）
  - 对超大文件（>10000 行），首行索引可大幅减少候选集

#### 难点3：匹配歧义
- **问题**：代码中常有重复的模式（如多个相同的 `if`、`for` 开头）
- **现状**：文档通过 diff_taps + 逐行字符比对解决，但这要求用户提供足够长的 Location 内容
- **改进思路**：未来可考虑在错误信息中展示模糊匹配的上下文，帮助用户/LLM 调整 Location

#### 难点4：嵌套 Location 的状态管理
- **问题**：Location 嵌套时，block_stack 需要正确 push/pop
- **注意**：
  - `Off:Location` 必须明确知道关闭的是哪一级
  - LLM 生成的脚本可能漏写 `Off`，需要健壮的容错处理
  - 隐式 Off 行为：脚本结束时如果栈不为空，按从内到外逐级关闭

#### 难点5：`...` 分隔符的二义性与 Raw 命令
- **`...` 的本质**：伪代码省略符号，因其省略特性被程序用作**全局分隔符**（终止 Location/New/Delete 的内容提取）
- **作为分隔符的场景**：
  - `//!@Location:` 后，提取内容直到遇到独立的 `...` 或下一个 `//!@` 命令
  - `//!@New:` 后，提取内容直到遇到 `...` 或下一个 `//!@` 命令
  - `//!@Delete:` 后，提取内容直到遇到 `...` 或下一个 `//!@` 命令
- **作为字面量的场景**：需要写入真正的 `...` 内容时，使用 `Raw` 命令
- **`//!@Raw: ...` 命令**：
  - 放在 New 命令内部，表示在该位置直接写入字面量 `...`
  - 执行时机：与 New 配合，在 New 的内容中遇到 Raw 时不做分隔符解析，而是将其内容作为字面文本插入
  - 同理可用于 Delete，表示要匹配的字面内容包含 `...`
- **实现规则**：
  - 词法分析阶段：遇到 `//!@Raw:` 时，将其后的内容视为字面量，不触发分隔符逻辑
  - 语法分析阶段：Raw 作为其所在上下文（New/Delete 内容块）中的一个字面插入点

#### 难点6：New/Delete 后的格式一致性
- **问题**：插入或删除后，block 内行的 line_num 和 diff_taps 需要重新计算
- **建议**：每次修改操作后执行一次 `reindex(block)` 函数，统一重新计算所有行的元信息

#### 难点7：错误恢复
- **问题**：执行中间某条命令失败，文件可能处于部分修改状态
- **策略**：
  - 默认策略：执行失败时不写回文件，原文件保持不变（所有修改在内存中进行）
  - 未来可选：事务性修改（先备份，失败回滚）

#### 难点8：Unicode 和编码
- **问题**：源代码可能包含 UTF-8 字符、全角空格、零宽字符、BOM 等
- **策略**：
  - 统一使用 UTF-8 编码读写
  - BOM 自动检测并处理
  - 缩进计算只计 ASCII 空格（0x20），tab 计为可配置的宽度
  - 全角空格不计入缩进

### 4.2 性能优化点

| 优化项 | 方法 | 优先级 |
|--------|------|--------|
| 预计算 stripped_content | FileContent 构建时对每行预存去空白版本 | 高 |
| 首行哈希索引 | HashMap<String, Vec<usize>> 避免全量扫描 | 高 |
| 懒解析 block | 只在需要 Block 指令时才解析 block 边界 | 中 |
| 增量更新 | 修改后只重算受影响行的元信息，不全量 reindex | 低（先简单实现） |
| 并行处理 | Open 多文件时可并行读取（为 Async 扩展做准备） | 低（扩展阶段） |

---

## 5. 错误信息规范

### 5.1 错误信息格式

所有错误应包含：
1. **错误类型** — 简短概括
2. **具体原因** — 一句话说明
3. **上下文** — 相关代码块/内容的格式化展示
4. **建议** — 如何修复的提示

示例：
```
Error: Location matched 3 results (expected 1)
  Given Location content:
    3| fn example() -> Option<()> {
    4|     let x = 0;
    5|     ...

  Matched candidates:
    L12: fn example() -> Option<()> {
    L45: fn example() -> Option<()> {
    L78: fn example() -> Option<()> {
    (0 more)

  Suggestion: Add more context to Location content to disambiguate,
  or use line-number Location: //!@Location:@12,16
```

### 5.2 输出格式

- 修改后的 ContentBlock 输出使用：
  - 绿色 `+` 标注新增行
  - 红色 `-` 标注删除行
- 彩色输出应检测终端是否支持（`is_terminal`），管道/重定向时自动关闭颜色

---

## 6. 项目结构建议

```
src/
  main.rs              # CLI 入口，参数解析
  lexer.rs             # 词法分析：识别 //!@ 标识符
  parser.rs            # 语法分析：Token → AST (Command 序列)
  engine.rs            # 执行引擎：状态机驱动，逐条执行 Command
  matcher.rs           # 核心匹配算法（Location 匹配）
  block.rs             # Block 解析（花括号/缩进）
  model.rs             # 所有数据结构定义
  error.rs             # 错误类型定义 + 格式化输出
  output.rs            # 彩色终端输出
  file_io.rs           # 文件读写
tests/
  integration_tests/   # 端到端 .ned 脚本测试
  unit_tests/          # 各模块单元测试
```

---

## 7. 测试策略

1. **单元测试**：每个匹配函数、解析函数独立测试
2. **集成测试**：准备 .ned 脚本 + 对应的输入文件 + 预期输出文件，端到端验证
3. **边界测试**：
   - 空文件
   - 单行文件
   - 超大文件（10000+ 行）
   - 嵌套深度很大的 Location（10 层+）
   - 匹配歧义场景
   - tab/space 混用文件
   - Unicode 内容

---

## 8. 代码风格指导

### 8.1 文件级注释

每个 `.rs` 文件开头必须包含：
- 本文件的基本功能说明
- 核心实现逻辑概述
- 对应开发文档的章节引用

```rust
//! 词法分析器 (Lexer)
//!
//! 负责将输入的 .ned 脚本内容扫描为 Token 流。
//!
//! ## 实现逻辑
//!
//! 1. 逐行读取脚本内容，识别 `//!@` 标识符作为命令起始
//! 2. 根据命令头（Open/Location/New/Delete/Raw/Off）切分不同的命令块
//! 3. 命令块内的内容按行收集，遇到分隔符 `...` 或下一命令时终止
//! 4. 输出有序的 Token 序列供 Parser 使用
//!
//! ## 对应文档
//!
//! 详见 INSTRUCTION.md 第 1.1 节 "架构总览" 及 n_edit_dev.md "语法设计" 章节
```

### 8.2 类型与函数文档注释

每个 `struct`、`enum`、`fn`、`trait` 必须带有 `///` 文档注释，简要说明其职责。

```rust
/// 文件的完整内容表示
///
/// Open 命令成功读取文件后构建此结构，作为所有后续操作的根数据。
/// 它与 ContentBlock 共享相同的行表示（`Line`），
/// 因此可以统一传入 Location 匹配器。
struct FileContent {
    lines: Vec<Line>,
}

/// 逐行解析后的行数据
///
/// 每一行保留原始内容的同时，预计算缩进信息以加速匹配。
struct Line {
    /// 在文件中的行号（从 1 开始计数）
    line_num: usize,
    /// 行首空格数量（只计 ASCII 0x20，tab 按配置折算）
    taps: usize,
    /// 相对于所在 ContentBlock 首行的缩进差异
    diff_taps: usize,
    /// 该行的原始文本内容
    content: String,
}

impl Line {
    /// 返回去除所有空白字符后的内容，用于纯字符匹配
    fn stripped_content(&self) -> &str {
        // ...
    }
}
```

```rust
/// 命令执行引擎
///
/// 维护全局状态机，按顺序消费 Parser 输出的 AST 节点。
///
/// ## 状态流转
///
/// Open → Location (可嵌套) → New/Delete/Raw → Off
///
/// ## 错误恢复
///
/// 执行失败时保持在内存中修改，不写回原文件，
/// 确保原文件不受部分执行的影响。
struct Engine {
    file: Option<FileContent>,
    block_stack: Vec<ContentBlock>,
    current_line: usize,
}

impl Engine {
    /// 执行完整的 AST 命令序列
    ///
    /// 遍历 commands，逐条调用对应的处理方法。
    /// 执行完毕后自动处理隐式 Off:Open（若脚本末尾未显式关闭）。
    fn execute(&mut self, commands: Vec<Command>) -> Result<(), EngineError> {
        // ...
    }
}
```

### 8.3 数据结构简洁原则

- **一个 struct 只负责一类数据**：不允许一个 struct 同时承载文件内容和命令状态。
- **字段名直白，禁止缩写**：`line_number` 而非 `ln`，`block_stack` 而非 `bs`（仅公认缩写如 `vec`、`fn`、`config` 除外）。
- **优先使用 newtype**：含义不同的同类型数据应包装为独立类型，避免混淆。

```rust
// 正确：行号有独立类型，避免与索引混淆
/// 文件中的行号（从 1 开始）
struct LineNumber(usize);

/// 数组索引（从 0 开始）
struct LineIndex(usize);

// 错误：用裸 usize 无法区分语义
fn get_line(line_num: usize)  // 是行号还是索引？
```

```rust
// 正确：一个 struct 只做一件事
/// Location 命令中用户提供的定位内容
struct LocationContent {
    lines: Vec<LocationLine>,
}

/// 从文件内容中匹配到的候选结果
struct FirstMatchContent {
    start_line: LineNumber,
    lines: Vec<MatchLine>,
}

// 错误：一个 struct 承载多种语义
struct MatchResult {
    location_lines: Vec<LocationLine>,   // 输入
    candidates: Vec<FirstMatchContent>,  // 输出
    error_message: Option<String>,       // 错误
    // 职责混乱
}
```

```rust
// 命令枚举：每个变体只携带该命令所需的最小数据
enum Command {
    Open { file_path: String },
    Location { block: bool, line_range: Option<LineRange>, content: LocationContent },
    New { position: NewPosition, content: NewContent },
    Delete { block: bool, content: Option<DeleteContent> },
    Raw { content: String },
    Off { target: OffTarget },
}
```

### 8.4 方法短小、单一职责、高复用

- **每个方法建议不超过 30 行**（不含文档注释和空行）。超过则拆分为子方法。
- **一个方法只做一个实际逻辑**：方法名 = 它做的事。
- **相同逻辑必须复用**：出现第二次相似代码时立即提取为独立方法。

```rust
// 正确：短方法，单一职责，可复用
impl LocationContent {
    /// 提取定位内容的第一行（去除空白后用于首行匹配）
    fn stripped_first_line(&self) -> &str {
        self.lines[0].stripped_content()
    }

    /// 定位内容的有效行数（跳过空行）
    fn non_empty_line_count(&self) -> usize {
        self.lines.iter().filter(|line| !line.content.trim().is_empty()).count()
    }

    /// 检查定位内容是否全部在同一缩进层级（用于判断是否需要 Block 解析）
    fn is_single_level(&self) -> bool {
        self.lines.iter().all(|line| line.diff_taps == Some(0))
    }
}
```

```rust
// 正确：将复杂逻辑拆分为独立小方法
impl LocationMatcher {
    /// 在文件内容中执行 Location 匹配，返回唯一 ContentBlock
    fn find_unique_block(
        file: &FileContent,
        location: &LocationContent,
    ) -> Result<ContentBlock, MatchError> {
        let candidates = self.collect_first_line_matches(file, location);
        let filtered = self.filter_by_full_match(candidates, location);
        self.expect_single_match(filtered, location)
    }

    /// 收集首行匹配的所有候选起点
    fn collect_first_line_matches(
        &self,
        file: &FileContent,
        location: &LocationContent,
    ) -> Vec<FirstMatchContent> {
        let target = location.stripped_first_line();
        file.lines
            .iter()
            .filter(|line| line.stripped_content() == target)
            .map(|line| self.build_candidate(file, line.line_num, location.lines.len()))
            .collect()
    }

    /// 对候选集进行逐行全量匹配筛选
    fn filter_by_full_match(
        &self,
        candidates: Vec<FirstMatchContent>,
        location: &LocationContent,
    ) -> Vec<FirstMatchContent> {
        candidates
            .into_iter()
            .filter(|candidate| self.rows_match(candidate, location))
            .collect()
    }

    /// 逐行比对：content（去空白）+ diff_taps 双重校验
    fn rows_match(
        &self,
        candidate: &FirstMatchContent,
        location: &LocationContent,
    ) -> bool {
        // 跳过空行后逐一比对
        // ...
    }

    /// 确认匹配结果唯一，否则构造详细错误信息
    fn expect_single_match(
        &self,
        candidates: Vec<FirstMatchContent>,
        location: &LocationContent,
    ) -> Result<ContentBlock, MatchError> {
        match candidates.len() {
            0 => Err(MatchError::no_match(location)),
            1 => Ok(self.resolve_block(candidates.into_iter().next().unwrap())),
            n => Err(MatchError::too_many_matches(location, n)),
        }
    }
}
```

```rust
// 错误：方法过长、做太多事
fn execute_command(cmd: &Command, engine: &mut Engine) -> Result<(), String> {
    match cmd {
        Command::Open { file_path } => {
            // 50 行：检查路径、读文件、解析内容、构建 FileContent...
        }
        Command::Location { .. } => {
            // 100 行：提取内容、首行匹配、逐行过滤、Block 解析、错误处理...
        }
        // ...
    }
    // 应拆分为 execute_open、execute_location 等独立方法
}
```

### 8.5 文件组织：一个文件一个核心类型

- **每个 `.rs` 文件建议只包含一个核心 struct/enum**（其辅助类型和方法可共存于同一文件）。
- **所有错误类型集中维护**在 `error.rs` 中，其他模块通过 `use crate::error::*` 引用。

```rust
// error.rs —— 集中管理所有错误类型
//
// 文件包含本项目所有错误类型定义。
// 每个错误类型实现 Display + Error trait，
// 并附带上下文信息用于构造用户友好的错误提示。

/// 匹配相关的错误
#[derive(Debug)]
enum MatchError {
    /// 未找到任何匹配
    NoMatch {
        location_content: String,
    },
    /// 找到过多匹配
    TooManyMatches {
        count: usize,
        candidates: Vec<String>,
        location_content: String,
    },
}

/// 命令解析错误
#[derive(Debug)]
enum ParseError {
    MissingFilePath,
    UnknownCommand { token: String, line: usize },
    UnexpectedSeparator { line: usize },
}

/// I/O 相关错误
#[derive(Debug)]
enum FileError {
    NotFound { path: String },
    CannotOpen { path: String, reason: String },
    WriteFailed { path: String, reason: String },
}

impl std::fmt::Display for MatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MatchError::NoMatch { location_content } => {
                write!(
                    f,
                    "Location 命令未找到任何匹配，请检查定位内容：\n{}",
                    location_content
                )
            }
            MatchError::TooManyMatches { count, candidates, location_content } => {
                write!(
                    f,
                    "Location 命令匹配到 {} 个结果（期望 1 个）\n\
                     Location 内容:\n{}\n\
                     匹配候选:\n{}",
                    count,
                    location_content,
                    candidates.join("\n")
                )
            }
        }
    }
}

impl std::error::Error for MatchError {}
```

```rust
// 文件组织示例
//
// src/
//   model.rs           # 核心数据结构：FileContent, ContentBlock, Line, LocationContent 等
//   error.rs           # 所有错误类型集中定义
//   lexer.rs           # 词法分析器（核心类型：Lexer）
//   parser.rs          # 语法分析器（核心类型：Parser）
//   engine.rs          # 执行引擎（核心类型：Engine）
//   matcher.rs         # 匹配算法（核心类型：LocationMatcher）
//   block.rs           # Block 解析器（核心类型：BlockParser）
//   file_io.rs         # 文件读写工具（无核心类型，纯函数模块）
//   output.rs          # 终端输出格式化（无核心类型，纯函数模块）
//   main.rs            # CLI 入口 + 参数解析
```

### 8.6 禁止项

| 禁止 | 替代做法 |
|------|----------|
| 缩写字段名/变量名（`ln`, `cnt`, `buf`） | 全拼：`line_number`, `count`, `buffer` |
| 一个 struct 承载多个职责 | 拆分为多个独立 struct |
| 一个方法超过 50 行 | 拆分为多个子方法 |
| 复制粘贴相似逻辑 | 提取为共享方法/泛型函数 |
| 裸 `String` 报错 | 使用 `error.rs` 中定义的具体错误类型 |
| `unwrap()` / `expect()` 在非测试代码中使用 | 用 `Result` 传播或用有意义错误信息 |
| 魔法数字（如 `4`, `80`, `100`） | 定义为 `const` 并注释含义 |
