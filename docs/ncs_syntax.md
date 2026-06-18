# NCS 语法参考手册

NCS（Narwhal Command Script）的脚本语法，涵盖命令结构、执行流、数据传递。

## 1. 快速开始

```ncs
!@Open ./src/main.rs
!@Location
fn main() {
!@New
    println!("Hello, NCS!");
@/Open
```

保存为 `hello.ncs`，执行：

```bash
cargo run -p ncs -- hello.ncs
```

## 2. 基本语法

### 2.1 命令前缀

所有命令以 `!@` 开头：

```
!@Open ./path/to/file.rs
```

| 元素 | 说明 |
|------|------|
| `!@` | 命令标识符（`!` 示意脚本执行，`@` 标识定向命令） |
| `Cmd` | 命令名（大小写不敏感，如 `Open`、`opEn`、`OPEN` 等效） |

### 2.2 命令格式

```
!@Cmd [mode] [args...]
```

- **mode**: 模式名（可选）。若省略则默认 `Normal`
- **args**: 键值对参数，格式 `key=value`。多个参数空格分隔

示例：

```ncs
!@Open Normal tests/data/config.rs start=1 end=10
!@Open Dir ./src depth=3 ignore="*.bin" filter="*.rs,*.py"
!@New Start
!@Location Block
```

### 2.3 行执行 vs 块执行

| 类型 | 行为 | 示例 |
|------|------|------|
| **行执行** | 只读取命令所在行 | `!@Open ./file.rs` |
| **块执行** | 提取从下一行到终止条件的全部内容 | `!@Location` + 内容行 + 终止 |

**块执行终止条件**（遇到第一条即停）：
1. 下一个非仅展开的 `!@Cmd` 行
2. 对应的 `@/Cmd` 关闭符号

```ncs
!@Location                    # 块执行开始
fn authenticate() -> bool {
    check_password()
}
!@New                         # 上一个 Location 内容在此终止
    log::info!("done");
@/Open                        # New 内容在此终止
```

### 2.4 关闭符号 `@/Cmd`

关闭对应的命令，触发写回/清理操作：

```ncs
@/Open      # 关闭 Open，写回文件
@/Location  # 关闭 Location，弹出 block_stack
@/New       # 关闭 New
@/Delete    # 关闭 Delete
@/Write     # 关闭 Write
```

脚本末尾的未关闭命令会**隐式关闭**（自动 `@/Open` 写回文件）。

### 2.5 仅展开命令

`!@Raw` 是仅展开命令——遇到时**不触发块终止**，其内容展开为原始字符融入父命令。

> **阶段说明**：`!@Get` 命令的独立语法（`!@Get pool_name [like=...]`）Phase 3+ 实现。当前 Capture 指令（`@/Cmd | Capture pool_name`）已可用，详见 §2.6。

```ncs
!@New
    let config = load_config();
!@Raw
    // 此行不会被当成命令标记，而是原样写入
    // 即使在内容中包含 !@ 也不会被解析
@/Open
```

### 2.6 Capture 指令 — 捕获命令输出

在关闭符号后通过管道将命令输出存入全局数据池，供后续 `!@Get` 提取：

```
@/Open | Capture my_result
```

- Capture 发生在 `@/Cmd` 行中
- 被捕获的命令输出在退出 `exec_cmds` 前复制到 `pools`
- 键名不可重复（重复则覆盖，打印警告）

## 3. 文件编辑命令

### 3.1 Open — 打开文件/目录

```
!@Open [mode] <path> [options...]
```

| 模式 | 说明 |
|------|------|
| `Normal`（默认） | 打开文本文件，加载到内存 |
| `Dir` | 打开目录，递归扫描（开发中） |

**Normal 模式选项**:

| 参数 | 类型 | 默认 | 说明 |
|------|------|------|------|
| `start` | Number | 1 | 起始行号（1-based） |
| `end` | Number | 文件末 | 结束行号 |

```ncs
!@Open ./src/main.rs                  # 打开整个文件
!@Open ./src/main.rs start=10         # 从第 10 行开始
!@Open ./src/main.rs start=10 end=50  # 第 10 到 50 行
```

### 3.2 Location — 定位代码位置

```
!@Location [mode]
定位匹配内容...
```

| 模式 | 说明 |
|------|------|
| `Normal`（默认） | 基于去空白内容 + diff_taps 匹配 |
| `Block` | 匹配后用 BlockParser 获取精确块边界 |
| `Path` | 在指定文件中执行匹配（开发中） |

**匹配规则**:
1. 取 Location 首行**去空白**后，用 O(1) 哈希索引查找候选行
2. 对每个候选，逐行比对**去空白内容**和 **diff_taps**（相对缩进差异）
3. 要求恰好 1 个匹配，否则报错

```ncs
# 匹配单个函数签名
!@Location
fn authenticate_user(credentials: &Credentials) -> Result<Token, AuthError>

# Block 模式：精确识别函数体边界
!@Location Block
fn authenticate_user(credentials: &Credentials) -> Result<Token, AuthError>

# 嵌套定位：先在外部类中定位，再在内部方法中定位
!@Location
impl UserService {
!@Location
    fn create_user(&mut self, name: &str)
!@New
        log::info!("creating user: {}", name);
@/Location
@/Location
```

### 3.3 New — 插入新内容

```
!@New [mode]
插入内容...
```

| 模式 | 说明 |
|------|------|
| `Normal`（默认） | 在 Location 匹配位置之后插入（需要前一个 Location） |
| `Start` | 在文件开头插入（独立命令，不依赖 Location） |
| `End` | 在文件末尾插入（独立命令，不依赖 Location） |

> **约束**：`New Normal` 要求前一个 Location 存在于 `exec_cmds` 中。`New Start` 和 `New End` 可直接在文件级操作，无需事先执行 Location。

**缩进规则**: New 内容的每行 `diff_taps` 作为绝对缩进量，以插入位置的 `taps` 为基准计算。

```ncs
# 在匹配位置后添加新字段
!@Location
pub struct AppConfig {
!@New
    pub log_level: String,

# 在文件头部添加许可证
!@New Start
// Copyright 2024 Example Corp.

# 在文件尾部添加测试
!@New End
#[cfg(test)]
mod tests { ... }
```

### 3.4 Delete — 删除内容

```
!@Delete [mode]
匹配内容...
```

| 模式 | 说明 |
|------|------|
| `Normal`（默认） | 在 ContentBlock 内逐行匹配并删除 |
| `Block` | 删除整个 ContentBlock（要求 Location 也用 Block） |

**约束**:
- 要求连续匹配，不可跳行
- Delete 首行必须紧邻 Location 最后一行（中间不能有非空行）

```ncs
# 删除一个方法
!@Location
/// Generate a random salt string
!@Delete
fn generate_salt(rounds: u32) -> String {
    use rand::Rng;
    ...
}

# Block 删除
!@Delete Block
```

### 3.5 Raw — 字面量内容

```
!@Raw <content>
```

仅展开命令。内容作为字面量融入上一个 New 或 Delete 命令，标记 `is_raw`，插入/匹配时保留原始格式。

```ncs
!@New
    fn example() {
!@Raw
        // 此行保持原样，不做缩进计算
        call_something();
!@Raw
        // 多个 Raw 可连续使用
    }
@/Open
```

## 4. 完整执行流示例

### 添加结构体字段

```ncs
!@Open ./src/config.rs
!@Location
pub struct AppConfig {
!@New
    /// Log level filter
    pub log_level: String,
@/Open
```

### 替换函数实现

```ncs
!@Open ./src/services.rs
!@Location
/// Generate a random salt string
!@Delete
fn generate_salt(rounds: u32) -> String {
    // old implementation
}
!@New
fn generate_salt(rounds: u32) -> Result<String, ServiceError> {
    // new implementation
}
@/Open
```

### 多操作复合

```ncs
!@Open ./src/config.rs

# 操作 1: 添加字段
!@Location
    pub min_password_length: u32,
!@New
    pub log_level: String,
@/Location

# 操作 2: 添加方法
!@Location
impl AppConfig {
!@New
    pub fn reload(&mut self) -> Result<(), String> {
        *self = AppConfig::from_env();
        Ok(())
    }
@/Location

@/Open
```

## 5. 错误输出格式

```
Error: <标题>
  <详情行 1>
  <详情行 2>
  Hint: <修复建议 1>
  Hint: <修复建议 2>
```

颜色：`Error:` 红色加粗，标题黄色，详情灰色，`Hint:` 绿色加粗。管道/重定向时自动关闭颜色。

## 6. 与 n_edit (.ned) 的语法对比

| 功能 | n_edit (.ned) | NCS (.ncs) |
|------|--------------|------------|
| 命令前缀 | `//!@Open: path` | `!@Open path` |
| 内容分隔 | `...` 显式分隔符 | 下一命令自动终止 |
| 关闭 | `//!@Off:Open` | `@/Open` |
| 行号定位 | `@66,120` | 已移除，用嵌套 Location 替代 |
| 扩展性 | 固定 5 个命令 | 12 个命令 + Include 动态扩展 |

## 7. 开发命令

以下命令 Phase 3+ 实现：

| 命令 | 功能 |
|------|------|
| `Bash` | 执行 bash 命令（安全审查） |
| `Exec` | 直连终端执行 |
| `Read` | 读取文件并高亮显示 |
| `Write` | 将块内容写入文件 |
| `Include` | 动态导入外部命令 |
| `WorkPath` | 设置工作路径 |
| `Get` | 从数据池获取内容 |

## 8. 参考文档

| 文档 | 内容 |
|------|------|
| [INSTRUCTION.md](INSTRUCTION.md) | 总体设计、数据结构、算法 |
| [ncs_dev.md](ncs_dev.md) | NCS 命令定义和执行流 |
| [phases.md](phases.md) | 实现阶段和进度 |
