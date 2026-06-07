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

---

### `//!@Location:`

定位代码位置。**这是修改的前提**——之后的 `New` / `Delete` 都在这个位置范围内操作。

```ned
//!@Location:
fn process_data(items: &[Item]) -> Vec<Output> {
    let mut results = Vec::new();
...
```

定位规则：
- **第一行去空白匹配**：去掉所有空格后，在目标文件中找相同的第一行
- **逐行校验**：后续每一行（跳过空行）的去空白内容 + 缩进差异都必须一致
- **必须唯一**：只能匹配到一个位置，否则报错

**常见错误：**

| 错误信息 | 原因 | 解决 |
|----------|------|------|
| `Location命令未找到任何匹配` | 定位内容在文件中完全不存在 | 检查拼写，确认代码确实存在 |
| `Location命令匹配到 N 个结果` | 定位内容太短导致歧义 | 增加更多上下文行 |
| `New命令前缺少Location定位` | `New` 前面出现了 `...` 分隔符 | 删除 `...` 或将 `New` 紧跟在 `Location` 后 |

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
...
```

**变体：**

| 命令 | 说明 | 需要 Location? |
|------|------|---------------|
| `//!@New:` | 在定位位置后插入 | **是** |
| `//!@New:Start` | 在文件开头插入 | 否 |
| `//!@New:End` | 在文件末尾追加 | 否 |

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
...
```

**常见错误：**

| 错误信息 | 原因 | 解决 |
|----------|------|------|
| `Delete命令未能在当前Block中找到匹配内容` | 要删除的内容在定位范围内不存在 | 检查内容拼写或调整 Location |
| `Delete匹配位置与Location不紧邻` | 要删除的内容和定位内容之间有其他代码 | 在中间加嵌套 Location 或扩大 Location |

---

### `//!@Off:`

关闭当前作用域，将修改写回上一层。

| 命令 | 效果 |
|------|------|
| `//!@Off:Open` | 写回文件并关闭 |
| `//!@Off:Location` | 退出当前定位，写回上层 Block |
| `//!@Off:New` | 退出插入作用域（等同于 `...`） |

**重要**：如果脚本结束时没有遇到 `Off:Open`，程序会**自动执行**——不需要显式写。

---

### `//!@Location:Block` / `//!@Delete:Block`

识别**完整代码块**（函数、方法、类等）。

**花括号语言**（Rust / C / JS / Java）：通过扫描 `{` 和 `}`（正确处理字符串、注释）确定边界。
**缩进语言**（Python / YAML）：通过缩进层级确定边界。

```ned
//!@Location:Block
fn old_helper(data: &Data) -> Result<()> {
//!@New:
fn new_helper(data: &Data) -> Result<()> {
    data.validate()?;
    Ok(())
}
...
```

`Delete:Block` 删除整个 Block，**要求前一个 `Location` 也使用 `Block` 指令**。

```ned
//!@Location:Block
pub fn deprecated_parser(input: &str) -> ParseResult {
//!@Delete:Block
//!@Off:Open
```

**常见错误：**

| 错误信息 | 原因 | 解决 |
|----------|------|------|
| `Location被指定为一个Block但无法解析` | 定位内容是纯文本/Markdown，没有代码块结构 | 去掉 `:Block`，使用普通 `Location` |
| `Delete:Block要求前一个Location也使用Block指令` | `Delete:Block` 前面的 `Location` 没有 `:Block` | 将 Location 改为 `Location:Block` |

---

## 分隔符 `...`

`...` 有**两种含义**，取决于上下文：

| 上下文 | 含义 |
|--------|------|
| 在 `Location` 内容中 | 占位符（省略号），表示"这里还有代码" |
| 独立一行 | **分隔符**，终止上一个命令的内容提取 |

```ned
//!@Location:
fn main() {
    let a = 1;
    let b = 2;
    ...
//  ↑ 这里的 ... 是位置占位符，不影响匹配
//!@New:
    do_stuff();
...
//  ↑ 独立的 ... 终止 New 的内容提取
```

> **关键规则**：如果 `New` 或 `Delete` 前面出现了独立的 `...`，程序会报 `缺少 Location 定位`——因为 `...` 截断了 Location 的上下文。

---

## 命令状态机

```
Open ──→ Location ──→ [Location]* ──→ New ──→ Off
            │                         │
            │                         ├─→ Delete ──→ Off
            │                         │
            └──→ [Location:Block] ────┘
```

- `New` / `Delete` 前必须有 `Location`（或使用 `New:Start` / `New:End`）
- `Delete:Block` 前必须有 `Location:Block`
- 脚本结尾自动 `Off:Open`

---

## 匹配算法

```
输入: 目标文件 + 定位内容
输出: 唯一的 ContentBlock 或 详细错误

1. 提取定位内容第一行 → 去空白 → 在文件中找所有匹配行
2. 对每个候选:
   a. 从该行起取等长内容
   b. 逐行比对：去空白内容 + diff_taps（缩进差异）
   c. 任一不匹配则丢弃
3. 若剩余候选 != 1 → 报错（附带候选列表）
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
