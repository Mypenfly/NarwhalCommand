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

> **阶段说明**：`!@Get pool_name` 基本读取已可用（从 pools 提取内容）。`!@Get` 的块内展开和 `like=[!@Cmd]` 伪装模式见 Phase 5。

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
| `Dir` | 打开目录，递归扫描为树形文本 |

**Normal 模式选项**:

| 参数 | 类型 | 默认 | 说明 |
|------|------|------|------|
| `start` | Number | 1 | 起始行号（1-based） |
| `end` | Number | 文件末 | 结束行号 |

```ncs
!@Open ./src/main.rs                  # 打开整个文件
!@Open ./src/main.rs start=10         # 从第 10 行开始
!@Open ./src/main.rs start=10 end=50  # 第 10 到 50 行
!@Open Dir ./src depth=3 ignore="*.bin" filter="*.rs"
```

**Dir 模式**：

目录被序列化为**树形文本**，格式与文件内容一致，后续可使用 Location/New/Delete 命令操作：

```
dirname:
  file1.rs
  file2.txt
  subdir:
    nested.py
```

- `depth`（默认 3）：递归深度
- `ignore`（默认 `*.bin`）：忽略的文件/目录模式（`,` 分割，支持 `*` 通配符）
- `filter`：仅保留匹配的文件模式（如 `*.rs,*.py`）

对树形文本的 New/Delete 操作会在 `@/Open` 时反序列化为文件系统变更（创建/删除文件和目录）。diff 输出与文件操作格式一致（`+` 绿色新增，`-` 红色删除）。

### 3.2 Location — 定位代码位置

```
!@Location [mode]
定位匹配内容...
```

| 模式 | 说明 |
|------|------|
| `Normal`（默认） | 基于去空白内容 + diff_taps 匹配 |
| `Block` | 匹配后用 BlockParser 获取精确块边界 |

**匹配规则**:
1. 取 Location 首行**去空白**后，用 O(1) 哈希索引查找候选行
2. 对每个候选，逐行比对**去空白内容**和 **diff_taps**（相对缩进差异）
3. 要求恰好 1 个匹配，否则报错

> **⚠️ 重要：缩进必须用空格，禁止 Tab**
>
> `taps`（leading spaces count）只统计 ASCII 空格（`0x20`），**Tab 字符不计入 taps**。
> 若 Location 内容行混用 Tab 和空格，会导致 `diff_taps` 计算错误，匹配失败。
>
> ```ncs
> # ❌ 错误：Tab 缩进导致 taps=0，diff_taps 错位
> !@Location
> 	Close {           # Tab → taps=0, diff_taps=0
>         name: String, # 空格 → taps=8, diff_taps=8  ← 文件实际是 4
>
> # ✅ 正确：统一用空格
> !@Location
>     Close {          # 4空格 → taps=4, diff_taps=0
>         name: String,# 8空格 → taps=8, diff_taps=4 ← 与文件一致
> ```
>
> 此规则适用于所有涉及缩进匹配的命令（Location、New、Delete）。`!@Raw` 行不受影响（保持原样）。

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

### 3.6 Dir 模式 — 目录结构操作

目录被序列化为树形文本，可使用 Location/New/Delete 操作目录结构：

```ncs
# 查看目录结构
!@Open Dir ./src depth=3
# 树形文本内容示例：
# src:
#   main.rs
#   lib.rs
#   engine:
#     mod.rs

# 删除文件
!@Location
  lib.rs
!@Delete
  lib.rs
@/Open

# 添加新文件
!@Open Dir ./src
!@Location
  main.rs
!@New
  new_module.rs
@/Open

# 删除子目录（Block 模式删除整个子树）
!@Open Dir ./src
!@Location Block
  engine:
!@Delete Block
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

## 7. 系统命令

### 7.1 Bash — 执行系统命令

```
!@Bash <command>
```

通过 `bash -c` 执行命令，捕获 stdout/stderr。**流输出**。

**安全审查**：以下模式被自动拦截：
- `sudo` — 禁止提权
- `rm -rf /` — 禁止递归删除根目录
- `chmod 777 /` — 禁止对根目录设置 777 权限
- `mkfs.` — 禁止格式化命令
- `dd if=` — 禁止直接操作磁盘
- `forkbomb` / `:(){ :|:& };:` — 禁止 fork 炸弹

```ncs
!@Bash echo "Current dir: $(pwd)"
!@Bash grep "fn main" ./src/main.rs
```

### 7.2 Exec — 直连终端执行

```
!@Exec <command>
```

通过 `script -c` 直连终端执行，支持彩色输出、流式输出和交互。**值输出**（结果不保留）。

```ncs
!@Exec cargo build --release
!@Exec git log --oneline -10
```

### 7.3 Read — 读取文件/目录

```
!@Read [mode] <path> [options...]
```

读取文件或目录内容并带格式显示。**值输出**（结果不保留）。路径基于 `work_path` 展开。

| 模式 | 说明 |
|------|------|
| `Normal`（默认） | 读取文本文件，syntect 语法高亮，带灰色行号（`base16-ocean.dark` 主题） |
| `Dir` | 目录树形结构，目录蓝加粗，文件普通显示 |

**Normal 模式选项**：

| 参数 | 类型 | 默认 | 说明 |
|------|------|------|------|
| `start` | Number | 1 | 起始行号（1-based） |
| `end` | Number | `start + 999` | 结束行号。默认最多显示 1000 行，超出时抛出警告并截断 |

> **注意**：`end` 超出文件总行数时自动截断到末行，不报错。文件行数超过 1000 时默认只显示前 1000 行，可通过 `end` 参数指定更大范围。

```ncs
!@Read ./src/main.rs                     # 默认显示前 1000 行
!@Read ./src/main.rs start=10            # 从第 10 行开始，最多到 1009 行
!@Read ./src/main.rs start=10 end=50     # 第 10 到 50 行
!@Read ./src/main.rs end=2000            # 显示前 2000 行
```

**Dir 模式选项**（与 Open Dir 一致）：

| 参数 | 类型 | 默认 | 说明 |
|------|------|------|------|
| `depth` | Number | 3 | 递归深度 |
| `ignore` | String | `"*.bin"` | 忽略的文件/目录模式（`,` 分割，支持 `*` 通配符） |
| `filter` | String | 空 | 仅保留匹配的文件类型（如 `"*.rs,*.py"`） |

```ncs
!@Read Dir ./src                                     # 目录树形输出（默认深度 3）
!@Read Dir ./src depth=1                             # 仅展示一级
!@Read Dir ./src ignore="*.bin,target"               # 忽略 .bin 文件和 target 目录
!@Read Dir ./src filter="*.rs"                       # 仅展示 .rs 文件
```

**Dir 模式输出示例**：

```
src:
  lib.rs
  main.rs
  engine:
    mod.rs
    executor.rs
```

### 7.4 Write — 写入文件

```
!@Write [mode] <path>
写入内容...
@/Write
```

| 模式 | 说明 |
|------|------|
| `Normal`（默认） | 将块内容写入文件（自动创建父目录） |
| `Raw` | 从下一行到脚本末尾的全部内容原样写入（程序退出） |

**Normal 模式**会检查路径是否为文件类型，路径不存在则自动创建父目录。**值输出**（结果不保留）。

```ncs
# 写入普通内容
!@Write Normal ./output/result.txt
第一阶段输出：
总计处理 1,234 条记录。
@/Write

# Raw 模式：以下所有内容原样写入到 EOF
!@Write Raw ./output/script.sh
#!/bin/bash
echo "this will not be parsed as NCS commands"
!@NotACommand just raw text
@/NotARealClose
# 脚本在此退出，不再执行后续命令
```

### 7.5 Include — 导入外部命令

```
!@Include <path> alias=<name> [block=true] [type=StreamOutput] [exec=script]
```

动态注册外部命令到命令注册表。

| 参数 | 必填 | 说明 |
|------|:---:|------|
| `alias` | 是 | 命令别名（禁止与内置命令重名） |
| `block` | 否 | 是否支持块执行（默认 `false`） |
| `type` | 否 | 输出类型：`StreamOutput` / `OnlyPrint`（默认 `OnlyPrint`） |
| `exec` | 否 | 执行方式：`default` / `script` / `bash`（默认 `default`） |

**校验**：alias 不与内置命令重名 → 返回 `AliasConflict` 错误。

```ncs
!@Include /usr/local/bin/mytool alias=MyTool type=StreamOutput
!@Include ./tools/formatter.sh alias=Format exec=bash block=true
```

### 7.6 WorkPath — 设置工作目录

```
!@WorkPath <path>
```

修改进程当前工作目录，影响后续所有 `./`、`../` 路径的展开。

- 路径必须存在，否则报错
- 若路径为文件，取其父目录
- 脚本中未遇到 `!@WorkPath` 时，工作路径默认为脚本文件的父目录

```ncs
!@WorkPath ./src/
!@Open main.rs          # 相对于新的工作路径
!@WorkPath /tmp/
!@Bash ls -la           # 在新的工作路径下执行
```

### 7.7 Get — 获取已捕获数据

```
!@Get <pool_name> [like=Cmd]
```

从全局 pools 中提取数据。基本读取（无 `like` 参数）已实现。

```ncs
@/Open | Capture my_result
!@Get my_result     # 读取捕获的数据
```

> `like=[!@Cmd]` 伪装模式和 `{}` 占位符替换见 Phase 5。

## 8. 参考文档

| 文档 | 内容 |
|------|------|
| [INSTRUCTION.md](INSTRUCTION.md) | 总体设计、数据结构、算法 |
| [ncs_dev.md](ncs_dev.md) | NCS 命令定义和执行流 |
| [phases.md](phases.md) | 实现阶段和进度 |
