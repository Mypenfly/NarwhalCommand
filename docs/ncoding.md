# N-coding 设计要求文档

N-coding 是一个基于 TUI 的 coding agent 工具，专为 DeepSeek API 设计。核心采用从 narwhal 项目中抽离的 Command System——一种基于模型输出文本正则匹配的命令机制，不依赖原生 tool_call/function_call。

(注意本项目的思路来源的草案在./narwhal_dev.md,有关的提示词示例文档在./ncoding_prompt_example.md)

---

## 1. 项目定位

- **目标**：终端内运行的极简 TUI coding agent，辅助完成编码任务
- **交互**：TUI 终端界面（ratatui），无限滚动文本区 + 底栏输入框
- **技术栈**：Rust
- **Shell 环境**：bash（`/usr/bin/bash -c`），推荐 rg (ripgrep) 文件搜索
- **API 后端**：DeepSeek API（deepseek-v4-pro / deepseek-v4-flash，启用 thinking mode）
- **核心机制**：基于文本匹配的 Command System，模型输出中途异步执行命令，所有命令并发执行

---

## 2. Command System —— 核心语法

### 2.1 设计出发点

1. 解决 tool_call 导致模型输出流中断的问题——命令可在模型持续输出时异步执行
2. 命令调用和返回在 assistant/user 同级，模型通过文本直接理解调用过程
3. 支持模型"边想边做"——输出 token 的同时解析并执行已完成的命令块

### 2.2 标准语法

```
《[CommandName]|character|》
【key_1】value_1【key_1】
【key_2】value_2【key_2】
---
【key_1】value_3【key_1】
【key_2】value_4【key_2】
《[End]|CommandName|》
```

- `CommandName` — 命令名，不区分大小写，内部消除特殊字符后全大写匹配
- `character` — 模型身份标识，如 `n-coding` 或 `subagent`
- `【key】value【key】` — 键值对，key 必须对称闭合。未闭合时仍尝试执行但产生警告
- `《[End]|CommandName|》` — 必须与开头 `CommandName` 一致，**必需**，缺失时拒绝执行并返回错误
- `---` — 同一命令的多次调用简写，分割的多个参数块级别相同，各块独立并发执行
- `/-...-/` — 行内注释，不解析为参数
- `//-...-//` — 块注释，包含的命令整体被跳过

### 2.3 匹配规则

| 规则 | 说明 |
|------|------|
| 命令名匹配 | 提取 `《[...]` 中内容，消除空格、`_`、`-` 后全大写匹配。`Example_Command` / `exampleCommand` / `EXAMPLECOMMAND` 均识别为 `EXAMPLECOMMAND` |
| key/value 匹配 | `【key】value【key】` 格式，key 必须对称闭合。value 可以是任意长字符串（含换行、代码等）。未闭合时仍尝试解析但产生警告 |
| 截止符 `《[End]\|Command\|》` | **必需**，Command 必须与开头一致。缺失时返回错误 "Command needs a 《[End]\|Command\|》to close the Command block" |
| 分隔符 `---` | 同一命令的多次调用简写，分割的多个参数块级别相同，各自独立并发执行 |
| 注释 `/-...-/` | 行内注释，不解析为参数。`//-...-//` 为块注释，整个命令块被跳过 |

### 2.4 解析触发与即时执行

```
模型开始流式输出 token
  ├─ 转发给 TUI 显示（markdown 渲染）
  ├─ 追加到内存缓存 buffer
  └─ 异步监听器（CommandWatcher）持续扫描 buffer
       ├─ 检测到 <<<[Command]>>> → 开始解析参数块
       ├─ 遇到 --- / <<<[__END__]>>> / 下一个 Command → 参数块解析完成
       └─ tokio::spawn 异步执行该命令

模型输出结束
  └─ 等待所有命令执行完毕（async 命令除外）
  └─ 汇总结果拼入下一轮 user 系统软注入
  └─ 发起下一次 API 请求
```

---

## 3. Session 管理

### 3.1 设计原则

基于**工作目录**进行 session 管理，不做复杂的 flet 筛选和记忆召回。

### 3.2 文件结构

```
<工作目录>/
  .ncoding/
    sessions/
      <session_name>.json    # 每个 session 独立保存
    backups/
      <session_name>/        # 对应 session 的文件变更备份
        <timestamp>_<filename>     # 编辑前的原始文件副本
```

每个 session 文件（JSON）结构：

```json
{
  "name": "refactor-auth-module",
  "created_at": "2026-05-29T14:30:00Z",
  "updated_at": "2026-05-29T15:00:00Z",
  "messages": [
    { "role": "system", "content": "You are a coding assistant..." },
    { "role": "user", "content": "..." },
    { "role": "assistant", "reasoning_content": "...", "content": "..." }
  ]
}
```

messages 为完整的 OpenAI 兼容格式消息列表，会话恢复时直接加载用于构建上下文。

### 3.3 Session 命名策略

Session 名由用户的**第一条输入**自动生成，流程：

1. 用户首次输入后，用该输入发起一次**无系统提示词、无上下文**的 API 请求
2. 提示词："为以下对话生成一个简短的英文 slug 标题（2-5 词，小写，用连字符连接，不要带任何其他内容）：\n{user_input}"
3. 用 `deepseek-v4-flash` 执行此命名请求（节省成本）
4. 提取返回内容作为 session 文件名，若生成文件名已存在则追加数字后缀

### 3.4 Session 的上下文构建

每次发起 API 请求时，直接使用 session 文件中的 `messages` + 当前系统提示词作为上下文。**不做自动截断**——依赖 DeepSeek 的 cache 机制保持高性能。用户可通过 `/clear` 或 `/compact` 手动压缩上下文。

---

## 4. 内置命令定义

### 4.1 Shell —— Shell 命令执行

coding agent 最重要的命令。使用 **bash** 执行，非 nushell。

#### 提示词告知

系统提示词中明确告知模型：

```
你的 shell 环境是 bash。管道、重定向等请使用标准 bash 语法。
系统已安装以下工具：rg (ripgrep 代码搜索), jj (jujutsu 版本控制)。
```

#### 语法

```
<<<[Shell]>>>
「command」:「「cargo test -- --nocapture」」
「is_async」:「「false」」
---
「command」:「「cargo build」」
「is_async」:「「true」」
<<<[__END__]>>>
```

#### 参数

| 参数 | 必填 | 说明 |
|------|------|------|
| `command` | 是 | 要执行的 nushell 命令 |
| `is_async` | 否 | 是否异步执行，默认 `false`。`true` 时命令后台执行，避免阻塞 |

#### 数据结构

```rust
struct ShellCommand {
    command: String,
    is_async: bool,    // 默认 false
}
```

#### 同步执行流程（is_async = false）

1. 通过 `std::process::Command` 调用 `bash -c "{command}"` 执行
2. 当前工作目录为项目根目录
3. 捕获 stdout + stderr
4. 默认超时 120 秒（可配置）
5. 多个 `---` 分割的命令**并发执行**

#### 异步执行流程（is_async = true）

适用于 `cargo build`、`docker build` 等长命令：

1. 立即执行命令，非阻塞
2. **即时返回**：第一行 stdout + 提示 "async exec the command, will notify when done"
3. 命令在后台继续执行
4. 命令结束后，将完整输出回传给 CommandWatcher
5. 若当时正在等待下一轮 API 请求，则拼入系统软注入；否则在 TUI 中单独显示结果
6. 输出太长时自动修剪（保留前 50 行 + 后 50 行 + 中间省略行数提示）

**异步结果注入格式**：

```
【|SYSTEM|】
[ShellResult]
command: cargo build
is_async: true
status: completed
exit_code: 0
output (trimmed, total 450 lines):
  ... (trimmed output) ...
【|SYSTEM|】
```

#### 安全审查

在命令执行前进行安全检查，以下情况**直接拒绝执行**：

| 规则 | 拒绝原因 | 错误信息 |
|------|----------|---------|
| 包含 `sudo` | 不允许提权 | "你不能执行此命令，因为：包含 sudo 提权操作。请自行在终端中执行。" |
| 包含 `rm` 且目标路径在工作目录外 | 不允许删除工作目录外的文件 | "你不能执行此命令，因为：rm 目标在工作目录外。建议用户自行执行。" |
| 包含 `rm -rf /` 或 `rm -rf /*` | 危险操作 | "你不能执行此命令，因为：这是一个危险操作。已拦截。" |
| 包含 `chmod 777` 且路径为系统目录 | 安全风险 | "你不能执行此命令，因为：存在安全风险。建议用户自行评估。" |

> `rm` 工作目录内的文件/目录是允许的（如 `rm -rf ./target`）。

#### 返回格式

```
【|SYSTEM|】
[ShellResult]
status: OK
exit_code: 0
stdout:
...
stderr:
...
---
[ShellResult]
status: TIMEOUT (120s)
...
【|SYSTEM|】
```

### 4.2 FilesOperator —— 文件操作

基于业界 coding agent 标准模式设计：**read 获取内容（含行号）** → **edit 通过精确字符串替换修改**。参考 OpenCode / Claude Code。

#### 4.2.1 Read 模式

读取文件内容，返回时带行号，供模型定位精确的编辑区间。

```
<<<[FilesOperator]>>>
「mode」:「「read」」
「path」:「「src/main.rs」」
「limit」:「「80」」
「offset」:「「30」」
<<<[__END__]>>>
```

| 参数 | 必填 | 说明 |
|------|------|------|
| `mode` | 是 | 固定值 `read` |
| `path` | 是 | 相对/绝对路径（基于工作目录） |
| `offset` | 否 | 起始行号（1-indexed），默认 1 |
| `limit` | 否 | 读取行数，默认 2000，最大 2000 |

**返回**：每行带行号前缀 `{line}: {content}`，开头和结尾标注实际起止行。

#### 4.2.2 Write 模式

创建新文件或全量覆盖已有文件。

```
<<<[FilesOperator]>>>
「mode」:「「write」」
「path」:「「src/new_module.rs」」
「content」:「「pub fn hello() {\n    println!(\"hello\");\n}」」
<<<[__END__]>>>
```

| 参数 | 必填 | 说明 |
|------|------|------|
| `mode` | 是 | 固定值 `write` |
| `path` | 是 | 目标文件路径 |
| `content` | 是 | 写入的完整内容 |

若文件已存在则覆盖，TUI 中给出提示。

#### 4.2.3 Edit 模式（精确字符串替换）

编辑文件的核心方式。模型需**先 read 文件获取内容**，然后提供 `old_str`（需要在文件中精确匹配到的文本）和 `new_str`（替换后的文本）。

```
<<<[FilesOperator]>>>
「mode」:「「edit」」
「path」:「「src/parser.rs」」
「old_str」:「「let cmd = Command::parse(input);\n    cmd.execute().await?;」」
「new_str」:「「let cmd = CommandParser::from_str(input.trim())?;\n    cmd.execute_async().await?;」」
---
「mode」:「「edit」」
「path」:「「src/main.rs」」
「old_str」:「「fn main() {」」
「new_str」:「「#[tokio::main]\nasync fn main() {」」
<<<[__END__]>>>
```

| 参数 | 必填 | 说明 |
|------|------|------|
| `mode` | 是 | 固定值 `edit` |
| `path` | 是 | 目标文件路径 |
| `old_str` | 是 | 文件中需要被替换的**精确**文本（含原有缩进和换行） |
| `new_str` | 是 | 替换后的新文本 |

**关键约束**（提示词中明确告知模型）：
1. `old_str` 必须是文件中**唯一**出现的文本，若有多个匹配则编辑失败，模型需加更多上下文使其唯一
2. `old_str` 的缩进、空行、注释都必须与文件中**完全一致**
3. 模型必须**先 read 文件**获取真实内容，再基于真实内容构造 `old_str`，不可凭空捏造
4. 提倡小范围的精确编辑，不要用 edit 做全量文件替换（全量替换用 write 模式）

**实现逻辑**：
1. 读取文件内容
2. 在文件中查找 `old_str` 的出现次数
3. 若出现 0 次：返回错误，提示模型重新 read 确认内容
4. 若出现 > 1 次：返回错误，列出所有匹配的行号范围，要求模型提供更多上下文
5. 若出现恰好 1 次：执行替换，写入文件

#### 4.2.4 安全性

- 路径解析基于工作目录，默认阻止 `../` 逃逸
- write 模式下文件已存在时，TUI 提示"将覆盖 xxx 文件"
- read 模式大文件自动截断（默认最大 2000 行）

#### 4.2.5 数据结构

```rust
struct FileOperation {
    mode: FileMode,
    path: PathBuf,
    content: Option<String>,      // write 模式
    old_str: Option<String>,      // edit 模式
    new_str: Option<String>,      // edit 模式
    offset: Option<usize>,        // read 模式
    limit: Option<usize>,         // read 模式
}

enum FileMode { Read, Write, Edit }
```

#### 4.2.6 返回格式

```
【|SYSTEM|】
[FileResult]
status: OK
path: src/main.rs
lines: 30..109
30: use std::collections::HashMap;
...

---
[FileResult]
status: OK
path: src/parser.rs
replaced: 1 occurrence at line 32
---
[FileResult]
status: OK
path: src/new_module.rs
action: created
---
[FileResult]
error: old_str matched 2 locations (lines 32, 156). Provide more context to make old_str unique.
【|SYSTEM|】
```

### 4.3 ToolCall —— 外部工具调用

用于调用配置中定义的外部程序/脚本（如 web_search）。Shell 和 FilesOperator 之外的能力扩展。

```
<<<[ToolCall]>>>
「tool_name」:「「web_search」」
「query」:「「rust ratatui best practices」」
「count」:「「10」」
<<<[__END__]>>>
```

### 4.4 SubAgentTask —— 子任务委派

降级自 narwhal 的 AgentCall，仅支持 subagent。subagent **直接使用当前同一模型**，无需指定 agent_name。

```
<<<[SubAgentTask]>>>
「prompt」:「「审查 src/command/parser.rs 中的错误处理逻辑，并报告问题」」
---
「prompt」:「「为 src/tui/render.rs 中的 markdown 渲染函数写单元测试」」
<<<[__END__]>>>
```

**关键约束**：
- subagent **禁止使用 SubAgentTask 命令**（防止循环调用），调用时返回错误
- subagent 使用独立临时上下文（仅含系统提示词 + prompt），不继承主 session 的 messages
- subagent 执行完毕后，取其最后一条 content 作为结果返回

### 4.5 AgentSkills —— 技能系统

替代 narwhal 的 DailyNotes。命令用于加载、查看可用技能，提示词中鼓励模型使用 skills。

```
<<<[AgentSkills]>>>
「mode」:「「list」」
---
「mode」:「「load」」
「skill_name」:「「test-driven-development」」
<<<[__END__]>>>
```

| mode | 说明 |
|------|------|
| `list` | 列出所有可用 skills（名称 + 简介） |
| `load` | 加载指定 skill 的完整内容，注入下一轮系统提示 |

**Skills 搜索路径**（按优先级）：

1. **工作目录安装**：`<workspace>/.ncoding/skills/` — 项目专属 skills
2. **全局安装**：`~/.config/ncoding/skills/` — 用户级通用 skills

加载时优先匹配工作目录，未找到则 fallback 到全局目录。同名 skill 工作目录版本覆盖全局版本。

**提示词中鼓励使用**：系统提示词末尾追加：

```
你可以使用 AgentSkills 命令加载更多能力。当遇到以下情况时请主动加载对应技能：
- 需要写测试 → test-driven-development
- 遇到 bug → systematic-debugging
- 任务完成需要审查 → code-review
使用「mode」:「「list」」查看当前可用的所有 skills。
```

---

## 5. 命令调用的双向性

| 来源 | 条件 | 行为 |
|------|------|------|
| assistant 输出 | 思考部分或正文中出现命令语法 | 异步执行，结果写入下一次 user 系统软注入 |
| user 输入 | 原始输入**仅包含**命令语法 | 执行命令，结果直接显示在 TUI，不写入 session |
| user 输入 | 命令语法 + 其他正文 | 按正常请求处理，不解析命令 |

---

## 6. 系统软注入

系统信息注入 user 消息，模型可识别：

```
【|SYSTEM|】
cur_time: 2026-05-29T14:30:00+08:00

[ShellResult]
status: OK
exit_code: 0
stdout:
...

---

[FileResult]
status: OK
path: src/main.rs
lines: 1..50
...

【|SYSTEM|】
```

---

## 7. TUI 设计

### 7.1 布局（自适应终端大小）

TUI 布局必须适应终端尺寸变化（包括 zellij 分屏导致的终端 resize）。各区域按比例或固定行数分配：

```
┌─ N-coding · model_t · session: refactor-auth ────────┐
│                                                       │
│  [文本区域 — 占据剩余所有高度]                            │
│                                                       │
│  ▌ 所有输出都以追加方式进入：                             │
│  ▌   - 用户输入 (青色前缀 >)                            │
│  ▌   - 模型输出 (markdown 流式渲染)                     │
│  ▌   - 命令调用块 (折叠样式，紫色边框)                    │
│  ▌   - 命令返回结果 (折叠样式，绿色边框)                  │
│  ▌   - 系统提示 (黄色前缀 [system])                      │
│                                                       │
│───────────────────────────────────────────────────────│
│  Tokens · In: 1,234  Out: 567  Cache: 890 (hit: 72%) │
│  Cost: $0.0012                                        │
│───────────────────────────────────────────────────────│
│  > _  ← 输入框 (可自动扩展高度，自动换行)                 │
│───────────────────────────────────────────────────────│
│  ~/projects/n-coding  ·  deepseek-v4-pro  ·  thinking │
└───────────────────────────────────────────────────────┘
```

**区域说明**（从上到下）：

| 区域 | 行数 | 说明 |
|------|------|------|
| 标题栏 | 1 | 项目名 + model name + session name |
| 文本区域 | 剩余高度 | 无限滚动，存储所有历史消息 |
| Token 信息栏 | 1 | API 每次返回后更新当前累计 |
| 输入框 | 1~5 行（自适应） | 自动扩展高度，自动换行，有光标 |
| 状态栏 | 1 | 工作目录 + 模型标识 + 思考状态 |

### 7.1a 输入框行为

- **自动扩展高度**：单行时 1 行，输入超宽自动换行并扩展（最多 5 行，之后内部滚动）
- **自动换行**：输入内容超过终端宽度时自动折行，光标跟随
- **光标控制**：支持 `←` `→` 移动光标，`Home` `End` 跳到行首/行尾
- **Enter**：发送当前输入
- **Shift+Enter**：插入换行符（多行输入）
- **Ctrl+U**：清空输入框

### 7.1b Token 信息栏

每次 API 流式响应结束（收到最后的 SSE chunk 中的 `usage` 字段）后，提取并更新：

```
Tokens · In: <prompt_tokens>  Out: <completion_tokens>  Cache: <cache_hit>/<total_input> (hit: <ratio>%)
Cost: $<total_cost>
```

- `In`: prompt_tokens（总输入 tokens）
- `Out`: completion_tokens（总输出 tokens）
- `Cache`: prompt_cache_hit_tokens / prompt_tokens（缓存命中 / 总输入），计算命中率
- `Cost`: 基于当前模型定价 × token 用量实时计算，保留 4 位小数

**颜色映射**：

| 字段 | 颜色 | 说明 |
|------|------|------|
| `In` | fg | 默认前景色 |
| `Out` | green | 输出消耗 |
| `Cache` | green（高命中）/ yellow（低命中） | 命中率 > 50% 为绿色 |
| `Cost` | yellow | 金额提示 |
| 分隔符 `·` | grey2 | 低调分隔 |

**定价参考**（当前 DeepSeek 官网，单位 $/1M tokens）：

| 模型 | Input (cache miss) | Input (cache hit) | Output | 备注 |
|------|-------------------|-------------------|--------|------|
| `deepseek-v4-pro` | $0.435 | $0.003625 | $0.87 | 当前 75% off 促销中 |
| `deepseek-v4-flash` | $0.14 | $0.0028 | $0.28 | |

> 促销结束后 v4-pro 价格为：Input $1.74, Cache hit $0.0145, Output $3.48。N-coding 硬编码此价格表，或从配置加载。

### 7.2 Everforest 配色方案

| 用途 | 颜色 | Hex |
|------|------|-----|
| 背景 | bg_dim | `#1e2326` |
| 用户输入前缀 `>` | blue | `#7fbbb3` |
| 模型输出正文 | fg (default) | `#d3c6aa` |
| 模型思考链 (reasoning) | grey2 | `#939f91` |
| 命令块边框 | purple | `#d699b6` |
| 命令返回结果 | aqua | `#83c092` |
| 系统提示/注入 | yellow | `#dbbc7f` |
| 错误信息 | red | `#e67e80` |
| 状态栏文字 / Token 信息 | orange | `#e69875` |
| Token 高命中率 | aqua | `#83c092` |
| Token 低命中率 | yellow | `#dbbc7f` |
| 输入框光标 | orange | `#e69875` |
| 工作目录（状态栏） | grey2 | `#939f91` |
| 分隔符 | grey0 | `#7a8478` |

### 7.3 渲染规则

- **模型输出 (content)**：markdown 流式渲染（代码块高亮、标题、列表、内联代码）
- **模型思考链 (reasoning_content)**：灰色字体，`<< >>` 包裹，初始折叠，Tab 展开
- **命令调用块**：紫色虚线边框 `┌── <<<[Command]>>> ────`，默认显示命令名，Tab 展开
- **命令返回结果**：绿色虚线边框，默认折叠
- **用户输入**：`> ` 前缀 + 青色
- **系统信息**：黄色 `[system]` 前缀
- **错误/警告**：红色前缀 `[error]`

### 7.4 Slash 指令

输入以 `/` 开头触发 TUI 控制指令（不发给 API）：

| 指令 | 功能 |
|------|------|
| `/help` | 显示可用指令和快捷键列表 |
| `/clear` | 清空当前屏幕显示文本（不删除 session 文件） |
| `/undo` | 撤回上一轮对话（恢复文件变更 + 移除 message 记录） |
| `/model` | 打开模型/思考切换面板（列表选择） |
| `/model list` | 从 DeepSeek API 实时获取当前支持的模型列表并显示 |
| `/model <model_id>` | 直接切换模型（如 `/model deepseek-v4-flash`） |
| `/model thinking on\|off` | 切换思考模式 |
| `/model thinking effort high\|max` | 设置思考深度 |
| `/session list` | 列出当前目录下所有 session（名称 + 更新时间） |
| `/session switch <name>` | 切换/加载指定 session |
| `/session rename <new_name>` | 重命名当前 session |
| `/session delete <name>` | 删除指定 session（需确认，同时删除对应 backups） |
| `/session current` | 显示当前 session 信息 |
| `/skills` | 列出可用 skills（聚合工作目录和全局目录） |
| `/quit` | 退出（同 Ctrl+C） |
| `/cancel` | 取消当前模型输出（同 Esc） |

`/model` 面板示例：

```
┌─ Model Switch ──────────────────────────────────────┐
│  ○ deepseek-v4-pro    (旗舰，强代码能力)               │
│  ○ deepseek-v4-flash  (快速，成本低)                   │
│──────────────────────────────────────────────────────│
│  Thinking Mode:  [enabled]  [disabled]               │
│  Thinking Effort: [high]  [max]                      │
│──────────────────────────────────────────────────────│
│  Enter 确认  Esc 取消                                  │
└─────────────────────────────────────────────────────┘
```

`/model list` 启动时调用 `GET https://api.deepseek.com/models` 获取模型列表，展示：

```
┌─ Available Models (DeepSeek) ───────────────────────┐
│  ID                        Owner                     │
│  deepseek-v4-pro           deepseek                  │
│  deepseek-v4-flash          deepseek                  │
│  ...                                                │
│──────────────────────────────────────────────────────│
│  当前使用: deepseek-v4-pro                            │
│  /model <id> 切换    Esc 关闭                         │
└─────────────────────────────────────────────────────┘
```

### 7.5 快捷键

| 按键 | 作用 |
|------|------|
| `Enter` | 发送输入 |
| `Shift+Enter` | 在输入框中插入换行（多行输入） |
| `Ctrl+C` | 退出 |
| `Esc` | 取消当前模型输出 |
| `Ctrl+U` | 清空输入框 |
| `Ctrl+K` | 终止当前正在执行的 Shell 命令（含 async） |
| `Tab` | 展开/折叠光标所在的命令块或返回结果 |
| `←` `→` | 光标左右移动 |
| `Home` `End` | 光标跳到行首/行尾 |
| `↑` / `↓` | 滚动文本区域 |
| `PgUp` / `PgDn` | 翻页 |
| `Home` / `End`（文本区） | 跳到顶部 / 最新内容 |

### 7.6 /undo 撤回机制

用于撤回上一轮对话造成的影响，核心场景是恢复模型通过 FilesOperator/Shel 对工作目录做出的文件变更。

#### 备份策略

在每次模型调用 `FilesOperator` 的 `write` 或 `edit` **之前**，将原始文件（若存在）复制到备份目录：

```
.ncoding/backups/<session_name>/<timestamp>_<relative_path>
```

例如：
```
.ncoding/backups/refactor-auth/20260529T143000Z_src_main.rs
.ncoding/backups/refactor-auth/20260529T143005Z_src_parser.rs
```

备份文件名格式：`{RFC3339_timestamp}_{relative_path_with_underscores}`

#### /undo 执行流程

1. 检查当前 session 是否有上一轮的 assistant message（最后一对 user→assistant）
2. 查找 `.ncoding/backups/<session_name>/` 下时间戳在上一轮开始之后的备份文件
3. 对每个备份文件，将其恢复到原始路径（覆盖）
4. 删除已恢复的备份文件
5. 从 session messages 中移除上一轮的 user message 和 assistant message（回到发送前的状态）
6. TUI 显示撤回结果：`[undo] restored 3 files, removed 2 messages`

#### 限制

- 只能撤回最近一轮（连续执行 `/undo` 可逐轮回退）
- Shell 命令产生的副作用（如 `cargo build` 的 target 目录、git 操作）不自动撤回
- backup 文件在 `/session delete` 或手动清理时删除

### 7.7 终端大小自适应

N-coding 启动时查询终端尺寸，并在运行时响应 `SIGWINCH` 信号（终端 resize）重新计算布局：

- 文本区域：高度 = 终端行数 - 4（标题栏、Token 栏、输入框、状态栏各占 1 行，输入框可能多行）
- 输入框：最小 1 行，内容超出时自动扩展到最多 5 行
- 所有渲染内容在 resize 时重新 wrap（文本区域内容按新宽度重排）
- zellij 分屏导致终端列数变化时，markdown 渲染和输入框折行均实时响应

---

## 8. DeepSeek API 集成

### 8.1 接口格式

DeepSeek API 为 OpenAI 兼容格式，N-coding **不使用原生 tool_call/function_call**，所有工具调用通过 Command System 实现。

**端点**：`POST https://api.deepseek.com/chat/completions`

**请求体核心字段**：

```json
{
  "model": "deepseek-v4-pro",
  "messages": [...],
  "stream": true,
  "thinking": {
    "type": "enabled",
    "reasoning_effort": "high"
  },
  "max_tokens": 8192,
  "temperature": 1.0,
  "top_p": 1.0
}
```

**关键说明**：
- `stream: true` — SSE 流式响应
- `thinking.type` — `"enabled"` 启用思考模式，`"disabled"` 关闭（但一般对 coding 保持启用）
- `thinking.reasoning_effort` — `"high"`（默认）或 `"max"`（更强思考，但更慢，token 消耗更高）。可通过 `/model thinking effort max` 切换
- `frequency_penalty` / `presence_penalty` — DeepSeek 已废弃，**不要传**

### 8.2 流式响应格式（SSE）

```
data: {"choices":[{"delta":{"role":"assistant","reasoning_content":"...","content":""},"finish_reason":null}]}

data: {"choices":[{"delta":{"content":"Hello"},"finish_reason":null}]}

data: {"choices":[{"delta":{"content":" world"},"finish_reason":null}]}

data: {"choices":[{"delta":{"content":""},"finish_reason":"stop"}],"usage":{"total_tokens":26}}

data: [DONE]
```

**N-coding 处理逻辑**：

```rust
// 伪代码
while let Some(chunk) = stream.next().await {
    let delta = chunk.choices[0].delta;

    if let Some(ref rc) = delta.reasoning_content {
        if !rc.is_empty() {
            tui_tx.send(TuiEvent::ReasoningChunk(rc.clone()));
        }
    }

    if let Some(ref c) = delta.content {
        if !c.is_empty() {
            cmd_watcher_tx.send(CmdEvent::Token(c.clone()));
            tui_tx.send(TuiEvent::ContentChunk(c.clone()));
        }
    }

    if chunk.choices[0].finish_reason == Some("stop".into()) {
        cmd_watcher_tx.send(CmdEvent::OutputEnded);
        break;
    }
}
```

### 8.3 reasoning_content 的上下文拼接

**重要**：DeepSeek 模型下一轮请求**不会自动包含**上一轮的 reasoning_content。N-coding 需要：

1. 保存 assistant 消息时同时存储 `reasoning_content` 和 `content`
2. 下一轮构建 context 时，assistant 消息需同时携带两者：

```json
{
  "role": "assistant",
  "reasoning_content": "上一轮思考链...",
  "content": "上一轮正文..."
}
```

3. 若 reasoning_content 超长（> 32K tokens），截断保留前 16K + 后 8K tokens

### 8.4 模型说明

| 模型 ID | thinking | 说明 | 推荐场景 |
|---------|----------|------|---------|
| `deepseek-v4-pro` | enabled | 旗舰，最强代码能力 | 默认 coding agent |
| `deepseek-v4-flash` | enabled | 快速，成本低 | subagent、session naming |

---

## 9. 配置文件

### 9.1 完整示例（n_coding.kdl）

```kdl
// N-coding 配置文件
// 放置于工作目录根目录，或 ~/.config/ncoding/config.kdl 作为全局默认

api {
    base_url "https://api.deepseek.com"
    api_key_env "DEEPSEEK_API_KEY"
    model "deepseek-v4-pro"
    sub_model "deepseek-v4-flash"
    max_tokens 8192
    temperature 1.0
    top_p 1.0
}

thinking {
    enabled true
    reasoning_effort "high"     // "high" | "max"
}

session {
    sessions_dir ".ncoding/sessions"
    backups_dir ".ncoding/backups"
    max_context_messages 50     // 上下文最多保留的消息数（不含 system prompt）
    verbose false               // 是否显示详细系统信息
}

skills {
    // 工作目录优先级 > 全局目录
    // 全局目录: ~/.config/ncoding/skills/
    // 工作目录: .ncoding/skills/
    local_dir ".ncoding/skills"
    auto_load_list true
}

tools {
    "web_search" description="联网搜索，参数 query, count" exec={"python" "tools/web_search.py"}
    // 可扩展更多外部工具
}

safety {
    // 这些模式直接拒绝
    deny_patterns [
        "sudo ",
        "rm -rf /",
        "rm -rf /*",
        "chmod 777 /",
        "dd if=",
        "> /dev/sda"
    ]
    shell_timeout_secs 120
    file_max_read_lines 2000
}
```

### 9.2 配置加载优先级

1. 工作目录 `n_coding.kdl`（项目级覆盖）
2. `~/.config/ncoding/config.kdl`（用户全局默认）
3. 环境变量（`DEEPSEEK_API_KEY` 等）

高优先级覆盖低优先级，未配置的项使用程序内置默认值。

---

## 10. Rust 模块架构

```
n-coding/
├── Cargo.toml
├── n_coding.kdl                 # 默认配置文件
└── src/
    ├── main.rs                  # 入口，TUI 初始化
    ├── command/
    │   ├── mod.rs               # 命令系统入口 + 分发 + CommandWatcher
    │   ├── parser.rs            # <<<[Command]>>> 正则解析器
    │   ├── syntax.rs            # 数据结构 enum
    │   ├── shell.rs             # Shell 命令（bash 执行 + 安全审查 + is_async）
    │   ├── files_operator.rs    # FilesOperator（read/write/edit）
    │   ├── tool_call.rs         # ToolCall 外部工具
    │   ├── sub_agent_task.rs    # SubAgentTask
    │   └── agent_skills.rs      # AgentSkills + skills 目录扫描
    ├── session/
    │   ├── mod.rs
    │   ├── manager.rs           # Session 创建/读/写/切换/naming/undo
    │   └── backup.rs            # 文件备份与恢复机制
    ├── api/
    │   ├── mod.rs
    │   └── client.rs            # DeepSeek API 客户端（SSE 流式 + reasoning_content 处理）
    ├── config/
    │   ├── mod.rs
    │   └── loader.rs            # KDL 配置解析（工作目录 + 全局目录优先级）
    └── tui/
        ├── mod.rs
        ├── app.rs               # TUI 应用状态机
        ├── render.rs            # 渲染（markdown + everforest + 折叠展开）
        ├── input.rs             # 输入处理（普通输入 / slash 指令分发）
        ├── model_panel.rs       # /model 切换面板
        └── theme.rs             # everforest 主题常量
```

### 10.1 并发架构

```
┌─ Main Thread (TUI event loop) ──────────┐
│  ratatui: 键盘事件 + 渲染                 │
│  - 输入 Enter → spawn API Task           │
│  - ↑↓ → 滚动                             │
│  - 收到 TuiEvent → 更新 app 状态          │
└─────────┬───────────────────────────────┘
          │ tokio::spawn
          ▼
┌─ API Task ───────────────────────────────┐
│  reqwest SSE stream                      │
│  delta.reasoning_content → tui_tx        │
│  delta.content → tui_tx + cmd_tx         │
│  finish_reason=stop → cmd_tx(OutputEnded)│
└─────────┬───────────────────────────────┘
          │ tokio::sync::mpsc
          ▼
┌─ CommandWatcher Task ────────────────────┐
│  监听 buffer                              │
│  CommandParser::feed(token)              │
│  解析到完整命令 → spawn 执行任务            │
│  收到 OutputEnded:                        │
│    ├─ 等待 sync 命令全部完成               │
│    ├─ async 命令若已完成 → 拼入结果        │
│    └─ 汇总 → 发起下一轮 API 请求           │
│                                          │
│  监听 async 完成回调 mpsc:                 │
│    └─ 拼入结果或推送到 TUI                 │
└──────────────────────────────────────────┘
          │ vector of tokio::spawn
          ▼
┌─ CommandExecutor Tasks (并发) ────────────┐
│  Shell::execute / FilesOperator::execute  │
│  结果通过 mpsc 回传                        │
│                                          │
│  async 命令额外持有 CommandWatcher mpsc    │
│    完成后推送结果                          │
└───────────────────────────────────────────┘
```

### 10.2 核心数据结构

```rust
enum NCommand {
    Shell { blocks: Vec<ShellBlock> },
    FilesOperator { blocks: Vec<FileOpBlock> },
    ToolCall { blocks: Vec<ToolCallBlock> },
    SubAgentTask { blocks: Vec<SubAgentBlock> },
    AgentSkills { blocks: Vec<SkillsBlock> },
}

struct ShellBlock {
    command: String,
    is_async: bool,  // 默认 false
}

struct FileOpBlock {
    mode: FileMode,
    path: PathBuf,
    content: Option<String>,
    old_str: Option<String>,
    new_str: Option<String>,
    offset: Option<usize>,
    limit: Option<usize>,
}

struct ToolCallBlock {
    tool_name: String,
    args: HashMap<String, String>,
}

struct SubAgentBlock {
    prompt: String,
}

struct SkillsBlock {
    mode: SkillsMode,
    skill_name: Option<String>,
}

enum FileMode { Read, Write, Edit }
enum SkillsMode { List, Load }

// 命令执行结果
struct CommandResult {
    command_type: CommandType,
    block_index: usize,
    outcome: CommandOutcome,
}
```

---

## 11. 开发路线图

### Phase 1: 最小可用原型（MVP）

**目标**：能用 TUI 与 DeepSeek 对话，Shell 命令可同步执行

| 步骤 | 任务 | 产出 |
|------|------|------|
| 1.1 | `cargo init`，引入依赖（ratatui, reqwest, tokio, serde, serde_json, regex, kdl-rs） | Cargo.toml |
| 1.2 | 实现 KDL 配置加载器（支持工作目录 `n_coding.kdl` + 全局 `~/.config/ncoding/config.kdl` 优先级合并） | `config/loader.rs` |
| 1.3 | 实现 DeepSeek API 客户端：SSE 流式响应解析，reasoning_content 与 content 分离、下一轮拼接 | `api/client.rs` |
| 1.4 | 实现基础 TUI：单页文本区（无限滚动） + 输入框 + 状态栏，everforest 配色 | `tui/app.rs`, `tui/render.rs`, `tui/theme.rs` |
| 1.5 | 实现 CommandParser 正则引擎：解析 `<<<[Command]>>>` 语法块，支持分隔符 `---`，命令名标准化 | `command/parser.rs`, `command/syntax.rs` |
| 1.6 | 实现 Shell 命令执行器：bash -c 执行，安全审查，同步模式（is_async=false），超时处理 | `command/shell.rs` |
| 1.7 | 实现 CommandWatcher：流式监听 buffer + 即时解析 + 异步执行，OutputEnded 汇总 | `command/mod.rs` |
| 1.8 | 实现命令结果系统软注入 + 下一轮 API 请求串联 | `command/mod.rs` |
| 1.9 | 端到端集成测试：发一条 coding 请求，模型调用 Shell 执行，结果返回继续对话 | 手动测试 |

### Phase 2: 完善命令体系

| 步骤 | 任务 | 产出 |
|------|------|------|
| 2.1 | FilesOperator - Read（文件读取 + 行号返回） | `command/files_operator.rs` |
| 2.2 | FilesOperator - Write（创建/覆盖文件） | 同文件 |
| 2.3 | FilesOperator - Edit（精确字符串替换：唯一性检查、错误提示） | 同文件 |
| 2.4 | ToolCall 外部工具调用 | `command/tool_call.rs` |
| 2.5 | SubAgentTask（独立上下文，禁止循环） | `command/sub_agent_task.rs` |
| 2.6 | AgentSkills（list/load，工作目录 + 全局目录扫描） | `command/agent_skills.rs` |
| 2.7 | 为每条命令编写提示词模板（含使用示例和约束说明） | prompt 生成模块 |

### Phase 3: Session + Shell 进阶

| 步骤 | 任务 | 产出 |
|------|------|------|
| 3.1 | Session JSON 文件读写（`.ncoding/sessions/`） | `session/manager.rs` |
| 3.2 | Session 自动命名（首次输入 → API 请求 → slug，用 flash 模型） | `session/manager.rs` |
| 3.3 | Shell is_async 模式：后台执行 + 即时返回 + 完成回调 + 输出修剪 | `command/shell.rs` |
| 3.4 | Shell 安全审查完善（sudo / rm 工作目录外 / deny_patterns） | `command/shell.rs` |
| 3.5 | Slash 指令解析与路由（`/help /clear /session /model /skills /cancel /quit /undo`） | `tui/input.rs` |
| 3.6 | `/model` 切换面板（模型列表 + thinking 开关 + reasoning_effort 选择），`/model list` 从 API 获取模型 | `tui/model_panel.rs`, `api/client.rs` |
| 3.7 | Session 切换时上下文加载 + 上下文截断策略 | `session/manager.rs` |
| 3.8 | `/undo` 撤回机制（file backup + restore + message 移除）| `session/backup.rs`, `session/manager.rs` |
| 3.9 | Token 信息栏解析 usage 字段、缓存命中率计算、实时金额统计 | `tui/app.rs`, `tui/render.rs` |
| 3.10 | 终端 resize 自适应（SIGWINCH 监听，布局重算）| `tui/app.rs` |

### Phase 4: 打磨

| 步骤 | 任务 | 产出 |
|------|------|------|
| 4.1 | Markdown 渲染完善（代码块语法高亮、标题、列表、表格、内联代码） | `tui/render.rs` |
| 4.2 | 命令块/结果 折叠展开（Tab 键交互，记忆折叠状态） | `tui/render.rs` |
| 4.3 | 错误处理完善（API 错误重试、网络超时、命令执行错误格式） | 全局 |
| 4.4 | 日志系统（`.ncoding/n-coding.log`），分级日志 | `main.rs` |
| 4.5 | Shell 环境信息注入到 system prompt（告知 bash、rg、jj 可用） | prompt 生成 |
| 4.6 | 大文件 read 截断提示友好化 + edit 错误信息完善 | `command/files_operator.rs` |

### 后续可扩展（非 MVP）

- Async 命令执行历史查看（`/jobs`）
- Skills 热加载（watch 文件系统变更）
- 鼠标滚动支持
- Session 内搜索（`/search`）
- 配置中的多 agent character 支持
- 非 DeepSeek provider 兼容（OpenAI、Anthropic）

---

## 12. 关键设计决策

1. **Shell 使用 bash 而非 nushell**：bash 是模型最熟悉的 shell，减少语法适配负担。推荐 rg (ripgrep) 等现代 CLI 工具。
2. **FilesOperator edit 采用精确字符串替换**：参考 OpenCode / Claude Code 的标准化实践，要求先 read 再 edit 保证准确性
3. **不使用 DeepSeek 原生 tool_call**：保持文本命令系统的一致性，且 reasoning_content 模式与 tool_call 互不兼容
4. **Session 基于文件而非内存**：持久化可靠，方便切换，`.ncoding/sessions/*.json`
5. **不实现 narwhal 的 Mem Manager、DailyNotes Reflection**：保持 N-coding 轻量 coding agent 定位
6. **TUI 极简设计**：无限滚动文本 + 输入框，不拆分窗口/聊天气泡
7. **全面支持 thinking mode 控制**：模型不仅可切换，还可调节思考深度

---

## 13. 与 narwhal 的差异总结

| 维度 | narwhal | N-coding |
|------|---------|----------|
| 定位 | 多 agent 协作平台 | 单人 coding agent |
| UI | Web UI + WebSocket | TUI (ratatui) |
| Shell | bash (外部工具) | bash（内置 + async 支持） |
| 文件操作 | ToolCall 外部工具 | FilesOperator（内置 read/write/edit） |
| 上下文管理 | Flexible Context Window (flet + LQ_t) | Session 文件 (JSON)，无自动截断 |
| 记忆 | 向量化 Mem Manager (store/query) | 无 |
| 命令集 | ToolCall + AgentCall(完整) + DailyNotes + SwitchSession | Shell + FilesOperator + ToolCall + SubAgentTask + AgentSkills |
| 自我认知 | DailyNotes + Reflection | AgentSkills（skills 加载） |
| 模型控制 | 配置文件 | `/model` 面板 + 实时切换 |
| API | 通用 | DeepSeek 特供（thinking mode + reasoning_effort 控制） |
