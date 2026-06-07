# N_Edit 脚本语法手册

`.ned` 脚本使用 **注释前缀命令** 精确修改源代码文件。每条命令以 `//!@` 开头，内容提取到下一个 `//!@` 命令或 `...` 分隔符为止。

---

## 快速入门

```ned
//!@Open: src/main.rs
//!@Location:
fn main() {
//!@New:
    println!("hello");
//!@Off:Open
```

这份脚本做的事：
1. 打开 `src/main.rs`
2. 定位到 `fn main() {` 这一行
3. 在这个位置后面插入 `println!("hello");`
4. 关闭文件，写回修改

---

## 命令参考

### `//!@Open: <文件路径>`

打开目标文件。必须参数：文件路径。

```ned
//!@Open: ./src/lib.rs
```

**常见错误：**

| 错误信息 | 原因 | 解决 |
|----------|------|------|
| `Open命令缺少文件路径参数` | `//!@Open:` 后没有路径 | 加上文件路径 |
| `Open命令的给定路径 ... 不存在` | 文件路径无效 | 检查路径拼写 |
| `无法打开文件 ...` | 文件存在但无权限或损坏 | 检查文件权限 |

---

### `//!@Location:`

定位代码位置。**这是修改的前提**——之后的 `New` / `Delete` 都在这个位置范围内操作。

```ned
//!@Location:
fn process_data(items: &[Item]) -> Vec<Output> {
    let mut results = Vec::new();
```

定位规则：
- **第一行去空白匹配**：去掉所有空格后，在目标文件中找相同的第一行
- **逐行校验**：后续每一行（跳过空行）的去空白内容 + 缩进差异都必须一致
- **必须唯一**：只能匹配到一个位置，否则报错

**`Location` 可接受空内容**（`//!@Location:` 后不跟任何内容行）：
- 空 Location 的搜索范围为**完整文件当前作用域**（顶层 = 整个文件，嵌套 = 父 Block）
- 适用于"想在文件中自由搜索 Delete 目标"的场景
- `matched_line_count` 为 0，Delete 的邻接检查被跳过

**常见错误：**

| 错误信息 | 原因 | 解决 |
|----------|------|------|
| `Location命令未找到任何匹配` | 定位内容在文件中完全不存在 | 检查拼写，确认代码确实存在 |
| `Location命令匹配到 N 个结果` | 定位内容太短导致歧义 | 增加更多上下文行 |
| `New命令前缺少Location定位` | `New` / `Delete` 前面出现了 `...` 分隔符 | 删除 `...` 或将命令紧跟在 Location 后 |

---

### `//!@New:`

在定位位置后插入内容。插入位置的缩进会被保留。

```ned
//!@Location:
fn example() {
    let x = 0;
//!@New:
    log::info!("processing");
    validate_input()?;
```

**变体：**

| 命令 | 说明 | 需要 Location? |
|------|------|---------------|
| `//!@New:` | 在定位位置后插入 | **是** |
| `//!@New:Start` | 在文件/当前 Block 开头插入 | 否 |
| `//!@New:End` | 在文件/当前 Block 末尾追加 | 否 |

> **注意**：`New:Start` / `New:End` 如果前面有 Location，则在 **当前 Block** 的开头/末尾操作，而非整个文件。

**常见错误：**

| 错误信息 | 原因 | 解决 |
|----------|------|------|
| `New命令发生在一个不确定的位置` | 使用了 `New:` 但没有前面的 `Location` | 先使用 `//!@Location:` 定位 |

---

### `//!@Delete:`

删除定位范围内匹配到的**连续行**。匹配逻辑和 `Location` 一致（去空白比对）。

```ned
//!@Location:
fn update_user(&self, user_id: u64) {
//!@Delete:
    log::warn!("deprecated call");
    self.deprecated_update(user_id)
```

**Delete 邻接规则：**
- Delete 首行必须紧邻 Location 最后一行之后（中间不能隔非空行）
- 如果中间隔了其他代码行，会报 `DeleteNotAdjacent` 错误
- **解决方法**：在两者之间插入嵌套 Location，精确桥接

```ned
// ❌ 错误：中间隔了 pipeline.add_stage(...) 等行
//!@Location:
pub fn run_app(config: AppConfig) {
//!@Delete:
    let result = pipeline.execute("test")?;

// ✅ 正确：用嵌套 Location 精确定位目标行
//!@Location:
pub fn run_app(config: AppConfig) {
//!@Location:
    let result = pipeline.execute("test")?;
//!@Delete:
    let result = pipeline.execute("test")?;
//!@New:
    let result = pipeline.execute("new test")?;
```

**常见错误：**

| 错误信息 | 原因 | 解决 |
|----------|------|------|
| `Delete命令未能在当前Block中找到匹配内容` | 要删除的内容在定位范围内不存在 | 检查内容拼写或调整 Location |
| `Delete匹配位置与Location不紧邻` | 要删除的内容和定位内容之间有其他代码 | 在中间加嵌套 Location 或扩大 Location 内容直到覆盖间隙 |

---

### `//!@Off:`

关闭当前作用域，将修改写回上一层。

| 命令 | 效果 |
|------|------|
| `//!@Off:Open` | 写回文件并关闭 |
| `//!@Off:Location` | 退出当前定位，逐层写回（嵌套时自动合并到父级） |
| `//!@Off:New` | 退出插入作用域（效果等同于 `...`） |

**重要**：如果脚本结束时没有遇到 `Off:Open`，程序会**自动执行**——不需要显式写。但嵌套 Location **必须**逐层 `Off:Location`，否则内层不会被写回。

---

### `//!@Location:Block` / `//!@Delete:Block`

识别**完整代码块**（函数、方法、类等）。

**花括号语言**（Rust / C / JS / Java）：通过逐字符扫描 `{` 和 `}`（正确处理字符串、行注释 `//`、块注释 `/* */`、转义 `\"` `\\`）确定边界。
**缩进语言**（Python / YAML）：通过缩进层级确定边界（跳过空行和注释行）。

```ned
//!@Location:Block
fn old_helper(data: &Data) -> Result<()> {
//!@New:
fn new_helper(data: &Data) -> Result<()> {
    data.validate()?;
    Ok(())
}
```

`Delete:Block` 删除整个 Block，**要求前一个 `Location` 也使用 `Block` 指令**。

```ned
//!@Location:Block
pub fn deprecated_parser(input: &str) -> ParseResult {
//!@Delete:Block
//!@Off:Open
```

`Location:Block` 与 `New:End` 配合可**在代码块末尾追加新方法/函数**：

```ned
// 在 impl DataPipeline 块末尾追加两个新方法
//!@Location:Block
impl DataPipeline {
//!@New:End
    pub fn clear_stages(&mut self) {
        self.stages.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }
```

**常见错误：**

| 错误信息 | 原因 | 解决 |
|----------|------|------|
| `Location被指定为一个Block但无法解析` | 定位内容是纯文本/Markdown，没有代码块结构 | 去掉 `:Block`，使用普通 `Location` |
| `Delete:Block要求前一个Location也使用Block指令` | `Delete:Block` 前面的 `Location` 没有 `:Block` | 将 Location 改为 `Location:Block` |

---

### 嵌套 Location（Phase 4）

可以在一个 `Location` 作用域内再次使用 `Location`，逐级缩小操作范围。适合深层代码结构的精确修改。

**二级嵌套示例：**

```ned
//!@Open: src/lib.rs
//!@Location:
fn outer() {
//!@Location:
    fn inner() {
//!@New:
        let extra = 2;
//!@Off:Location
//!@Off:Location
//!@Off:Open
```

执行过程：
1. 第一个 `Location` 在文件中定位 `fn outer`
2. 第二个 `Location` 在 `outer` 的 Block 内定位 `fn inner`（不是整个文件）
3. `New` 在 `inner` 内部插入新代码
4. 逐层 `Off:Location` 写回——内层→外层→文件

**三级嵌套示例（真实工程场景）：**

```ned
//!@Open: src/handler.rs
//!@Location:
impl RequestHandler {
//!@Location:
    fn process(&self) {
//!@Location:
        match self.status {
            Status::Active => {
//!@Delete:
                self.old_work();
//!@New:
                self.new_pipeline();
//!@Off:Location
//!@Off:Location
//!@Off:Location
//!@Off:Open
```

**跨层修改示例：**

外层用 `Location:Block + New:End` 追加方法，同时内层用嵌套 Location 修改细节：

```ned
//!@Open: src/pipeline.rs
// 外层：扩大 impl 块范围并追加新方法
//!@Location:Block
impl DataPipeline {
//!@New:End
    pub fn clear_stages(&mut self) {
        self.stages.clear();
    }
//!@Off:Location

// 内层：嵌套进入 execute 方法的 match 分支替换错误处理
//!@Location:
impl DataPipeline {
//!@Location:
    pub fn execute(&self, input: &str) -> Result<String, String> {
//!@Location:
                Err(e) => {
                    return Err(...);
                }
//!@Delete:
                Err(e) => {
                    return Err(...);
                }
//!@New:
                Err(e) => {
                    log::error!("stage error: {}", e);
                    return Err(format!("aborted: {}", e));
                }
//!@Off:Location
//!@Off:Location
//!@Off:Location
//!@Off:Open
```

**多次独立 Location 操作：**

同一个脚本中可以有多个不相关的 Location 操作（每次 `Off:Location` 后 block_stack 为空）：

```ned
//!@Open: src/config.rs
// 操作 1：struct 中添加字段
//!@Location:
pub struct AppConfig {
    pub name: String,
//!@New:
    pub description: String,
//!@Off:Location

// 操作 2：另一个 struct 中添加字段
//!@Location:
pub struct ConnectionPool {
    config: AppConfig,
//!@New:
    pool_id: u64,
//!@Off:Location
//!@Off:Open
```

**原理：**
- 嵌套 Location 使用 **搜索范围自动缩小**——若栈顶已有 ContentBlock，搜索范围限定为该 Block 而非整个文件
- 匹配到的 ContentBlock 始终保留**绝对文件行号**，写回时通过 `start_line` 差值计算正确偏移
- `Off:Location` 逐层弹出，内层修改先合并回外层，最外层最终写回文件

**常见错误：**

| 错误信息 | 原因 | 解决 |
|----------|------|------|
| `Location命令未找到任何匹配`（嵌套时） | 父 Block 中不存在要定位的内容 | 检查父 Block 范围是否覆盖目标行 |

---

## 分隔符 `...`

`...` 有**两种含义**，取决于上下文：

| 上下文 | 含义 |
|--------|------|
| 在 `Location` 内容中（嵌入行） | 占位符（省略号），表示"这里还有代码" |
| 独立一行 | **分隔符**，终止上一个命令的内容提取 |

### 内容提取规则

Lexer 读取命令内容时：
- 从命令行的剩余文本开始收集
- 继续读取后续行，直到遇到**下一个 `//!@` 命令**或**独立的 `...`**
- **`...` 提前终止**，不出现在命令内容中

```ned
//!@Location:
fn main() {
    let a = 1;
//!@New:
    do_stuff();
...
//!@Off:Open
```
- Location 内容 = `fn main() {` + `    let a = 1;`（遇到 `//!@New:` 停止）
- New 内容 = `    do_stuff();`（遇到 `...` 停止）

### `...` 的关键陷阱

**`...` 会重置 Location 追踪状态**——在 Location 和 New/Delete 之间出现独立的 `...` 会导致 `MissingLocation` 错误：

```ned
// ❌ 错误：... 在 Location 和 Delete 之间，切断了上下文
//!@Location:
fn example() {
    let old_code = 1;
...
//!@Delete:
    let old_code = 1;
```

```ned
// ✅ 正确：Delete 紧跟在 Location 内容之后，由下一个 //!@ 自动终止
//!@Location:
fn example() {
    let old_code = 1;
//!@Delete:
    let old_code = 1;
```

### `...` 的正确用法

`...` 只能用于**终止 New/Delete 内容**（当下一行不是 `//!@` 命令时）：

```ned
// ✅ New 内容由 ... 终止
//!@New:
    let x = 1;
    let y = 2;
...
//!@Off:Location

// ✅ Delete 内容由下一个 //!@ 命令终止（无需 ...）
//!@Delete:
    old_code();
//!@New:
    new_code();
```

### 记忆法则

| 场景 | 正确做法 |
|------|----------|
| Location 内容后跟 New/Delete | 不用 `...`，让下一个 `//!@` 自动终止 |
| Location 内容后跟嵌套 Location | 不用 `...`，让嵌套 `//!@Location:` 自动终止 |
| New 内容后跟 Off/Location | 不用 `...`，让下一个 `//!@` 自动终止 |
| New/Delete 内容后需要结束脚本 | 用 `...` 终止内容，然后跟 `//!@Off:Open` |

---

## 输出格式

### Diff 输出

修改成功后，程序输出带差异标记的行：

```
+ L12:     let new_field: String,      ← 绿色 + 新增行
- L15:     old_code();                 ← 红色 - 删除行
```

- **绿色 `+`**：新增的行，带行号前缀
- **红色 `-`**：删除的行，带行号前缀
- **管道/重定向时自动关闭颜色**（检测 `is_terminal`）

### 执行状态输出

- `--verbose` / `-v`：打印词法分析 Token 数 + 语法分析命令数
- `--quiet` / `-q`：只输出错误，不输出成功提示
- 默认：输出 `脚本执行成功: <文件名>` + diff 行

---

## 脚本编写注意事项

### 内容终止

1. **命令内容提取终止于下一个 `//!@` 行或独立 `...`**
2. **不要在 Location 和 New/Delete 之间放独立 `...`**——会切断追踪
3. 嵌套 Location 之间**不要放 `...`**——内层 Location 的 `//!@` 会自然终止外层内容提取

### 嵌套操作

1. **每个 `Off:Location` 只关闭一层**——N 层嵌套需要 N 个 `Off:Location`
2. **Delete 与 Location 紧邻**：Delete 内容在文件中的位置必须紧接 Location 最后匹配行之后。中间隔了其他非空行会报错。解决方法是扩大 Location 内容或使用嵌套 Location 桥接
3. **`New:Start` / `New:End` 在嵌套 Block 中会在当前 Block 开头/末尾操作**，而非整个文件

### 定位内容

1. Location 内容**越具体越安全**——内容太少可能匹配到多个位置
2. 匹配同时校验**缩进差异（diff_taps）**——即使字符相同，缩进层级不同也会被排除
3. 空行在匹配时**自动跳过**——Location 内容和文件中的空白行都不影响匹配

### 修改安全

1. **所有修改先在内存中进行**——执行失败时原文件不受影响
2. 脚本结尾**自动隐式 `Off:Open`**——不写 `//!@Off:Open` 也能写回
3. **建议始终写 `Off:Open`**——显式关闭更清晰，避免意外

---

## 匹配算法

```
输入: 目标文件 + 定位内容
输出: 唯一的 ContentBlock 或 详细错误

1. 提取定位内容第一行 → 去空白 → 在文件中找所有匹配行（O(1) 哈希索引）
2. 对每个候选:
   a. 从该行起取等长内容
   b. 逐行比对：去空白内容 + diff_taps（缩进差异）
   c. 任一不匹配则丢弃
3. 若剩余候选 != 1 → 报错（附带候选列表，最多展示 3 个）
4. 返回 ContentBlock
```

---

## 缩进规则

- **taps** = 行首 ASCII 空格数（tab 不计为空格）
- **diff_taps** = 当前行 taps 减去定位块首行 taps
- 空行在匹配时跳过

示例：

```python
# 定位内容:
    def foo():         # taps=4, diff_taps=0
        pass           # taps=8, diff_taps=4

# 文件中:
    def foo():         # taps=4 ✓ diff_taps=0 ✓ stripped="deffoo()" ✓
        pass           # taps=8 ✓ diff_taps=4 ✓ stripped="pass" ✓
```

---

## 支持的语言

| 语言 | 花括号/缩进 | Block 支持 |
|------|-----------|-----------|
| Rust | 花括号 | ✓ |
| C / C++ | 花括号 | ✓ |
| JavaScript / TypeScript | 花括号 | ✓ |
| Java | 花括号 | ✓ |
| Python | 缩进 | ✓ |
| YAML | 缩进 | ✓ |
| Markdown / 纯文本 | 无 | Location 可用，Block 不可用 |

---

## 安全保证

- **全部在内存中修改**：执行失败时原文件不受影响，只有显式 `Off:Open` 或脚本成功结束时才写回
- **必须精确匹配**：无法匹配时不会误改，而是抛出详细错误
- **缩进感知**：匹配不仅比内容，还比缩进层级，防止写错位置
- **邻接保护**：Delete 要求与 Location 紧邻，防止误删其他位置的代码

---

## 命令状态机

```
Open ──→ Location ──→ [Location]* ──→ New ──→ Off
            │                         │
            │                         ├─→ Delete ──→ Off
            │                         │
            ├──→ [Location:Block] ────┘
            │
            └──→ [Location (嵌套)] ──→ New/Delete ──→ Off:Location
```

- `New` / `Delete` 前必须有 `Location`（或使用 `New:Start` / `New:End`）
- `Delete:Block` 前必须有 `Location:Block`
- `...` 在 Token 流中重置 Location 状态（`last_was_location = false`）
- 脚本结尾自动 `Off:Open`

---

## 错误信息参考

### ParseError（解析错误）

| 错误 | 触发条件 | 行号 |
|------|----------|------|
| `MissingFilePath` | `//!@Open:` 后无路径 | — |
| `UnknownCommand` | 无法识别的命令头（如 `Off:Invalid`） | ✓ |
| `MissingLocation` | `New:` / `Delete:` 前无 Location（或被 `...` 截断） | ✓ |
| `UnexpectedSeparator` | 意外位置的独立 `...` | ✓ |
| `BlockRequiredForDelete` | `Delete:Block` 前未使用 `Location:Block` | ✓ |

### MatchError（匹配错误）

| 错误 | 触发条件 |
|------|----------|
| `NoMatch` | Location 内容在搜索范围内完全找不到 |
| `TooManyMatches` | Location 内容匹配到 ≥2 个候选（附带前 3 个候选行号） |
| `DeleteMatchFailed` | Delete 内容在当前 Block 中找不到连续匹配 |
| `DeleteNotAdjacent` | Delete 首行与 Location 末行之间隔了非空行 |
| `BlockNotParseable` | `Location:Block` 指定的内容无法解析为代码块 |

### FileError（文件错误）

| 错误 | 触发条件 |
|------|----------|
| `NotFound` | 文件路径不存在 |
| `CannotOpen` | 文件无法读取（权限等） |
| `WriteFailed` | 写回文件失败 |

### EngineError（引擎错误）

| 错误 | 触发条件 |
|------|----------|
| `MissingLocationForNew` | New/Delete 命令执行时 block_stack 为空 |
| `BlockStackEmpty` | `Off:Location` 时 block_stack 为空 |
| `BlockRequiredForDelete` | 引擎层面的 Block 指令不一致 |
| `ImplicitOffFailed` | 隐式 Off:Open 执行失败 |
