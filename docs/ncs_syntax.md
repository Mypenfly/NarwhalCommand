# NCS 语法手册

NCS（Narwhal Command Script）——用脚本编辑代码和操作系统的命令行工具。

---

## 一、快速开始

下面这个脚本打开 `src/main.rs`，定位 `fn main()`，在其后插入一行 `println!`，保存退出：

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

---

## 二、语法速览

### 2.1 命令格式

```
!@Cmd [mode] [key=value...] [positional_arg...]
```

所有命令以 `!@` 开头。命令名大小写不敏感（`Open`、`OPEN`、`opEn` 等效）。

`mode` 是可选的模式名。若省略则默认 `Normal`。

`key=value` 是键值对参数。不含 `=` 的 token 视为位置参数。

```ncs
!@Open Dir ./src depth=3 ignore="*.bin"
#     ^^^ ^^^^        ^^^^^^
#     Cmd mode        key=value pairs
#          positional arg (path)
```

### 2.2 行执行 vs 块执行

| 类型 | 说明 | 示例 |
|------|------|------|
| **行执行** | 只读取命令所在行 | `!@Open ./file.rs`、`!@Bash echo hi` |
| **块执行** | 提取命令下一行到终止条件之间的全部内容 | `!@Location`、`!@New`、`!@Delete`、`!@Write` |

块执行终止于：
- 下一个非仅展开的 `!@Cmd` 行
- 对应的 `@/Cmd` 关闭符号

```ncs
!@Location              # ← 块开始
fn authenticate() {
    check_password()
}
!@New                   # ← 上一块终止于此；New 块开始
    log::info!("done");
@/Open                  # ← New 块终止于此
```

### 2.3 关闭符号

```
@/Open      # 关闭 Open，将修改写回文件
@/Location  # 关闭 Location，弹出定位块栈
@/New       # 关闭 New
```

脚本末尾的未关闭命令会**自动隐式关闭**。例如只写了 `!@Open` 之后的所有编辑命令，末尾没有 `@/Open`——NCS 自动执行写回。

### 2.4 缩进规则

Location / New / Delete 的缩进匹配中，**Tab 不计入 taps**——只统计 ASCII 空格。所有示例统一使用空格缩进。

`diff_taps` = 当前行前导空格数 − Location 内容首行前导空格数。Location 匹配时会同时比对该值。

---

## 三、场景示例

以下示例全部来自 `ncs/tests/scripts/`，可直接执行。

### 3.1 为 Rust 结构体添加方法和删除旧方法

**目标文件** (`tests/data/rust_service.rs`)：

```rust
pub struct User {
    pub id: u64,
    pub name: String,
    pub email: String,
    pub active: bool,
}

impl UserService {
    pub fn get_user(&self, id: u64) -> Option<&User> {
        self.users.get(&id)
    }

    pub fn count_by_domain(&self, domain: &str) -> usize {
        self.users.values().filter(|u| u.email.ends_with(domain)).count()
    }
}
```

**脚本** (`rust_nested_edit.ncs`)——添加 `update_email`、删除 `count_by_domain`、在测试模块内插入新用例：

```ncs
!@Open ./tests/data/rust_service.rs

// -- 在 get_user() 之后新增 update_email 方法 --
!@Location
    pub fn get_user(&self, id: u64) -> Option<&User> {
        self.users.get(&id)
    }
!@New
    pub fn update_email(&mut self, id: u64, new_email: String) -> Result<(), String> {
        match self.users.get_mut(&id) {
            Some(user) => {
                user.email = new_email;
                Ok(())
            }
            None => Err(format!("User {} not found", id)),
        }
    }
@/Location

// -- 删除 count_by_domain 方法 --
!@Location
    pub fn count_by_domain(&self, domain: &str) -> usize {
        self.users
            .values()
            .filter(|u| u.email.ends_with(domain))
            .count()
    }
!@Delete
    pub fn count_by_domain(&self, domain: &str) -> usize {
        self.users
            .values()
            .filter(|u| u.email.ends_with(domain))
            .count()
    }
@/Location

// -- 嵌套 Location：在 tests 模块内插入新测试 --
!@Location
    mod tests {
!@Location
        use super::*;
!@New
        #[test]
        fn test_update_email() {
            let mut svc = UserService::new();
            let user = svc.create_user("Alice".into(), "a@ex.com".into());
            svc.update_email(user.id, "b@ex.com".into()).unwrap();
            assert_eq!(svc.get_user(user.id).unwrap().email, "b@ex.com");
        }
@/Location
@/Location

@/Open
```

**关键点**：
- 嵌套 Location：先定位到 `mod tests {`，再定位到 `use super::*;`，最后插入测试函数
- `@/Location` 必须与 `!@Location` 数量匹配（两个 Location，两个 @/Location）

---

### 3.2 Dart 类方法重构

**目标文件** (`tests/data/dart_auth.dart`)——使用 2-space 缩进（Dart 惯例）：

```dart
class AuthService {
  Future<AuthResult> refresh(String refreshToken) async {
    final response = await _client.post(
      Uri.parse('$baseUrl/auth/refresh'),
      body: {'refresh_token': refreshToken},
    );
    ...
  }

  Future<bool> validateSession(String token) async {
    final response = await _client.get(
      Uri.parse('$baseUrl/auth/validate'),
      headers: {'Authorization': 'Bearer $token'},
    );
    return response.statusCode == 200;
  }
}
```

**脚本** (`dart_refactor.ncs`)——新增 `logout()` 方法，删除 `validateSession()`：

```ncs
!@Open ./tests/data/dart_auth.dart

// -- 在 refresh() 之后插入 logout() --
!@Location
  Future<AuthResult> refresh(String refreshToken) async {
    final response = await _client.post(
      Uri.parse('$baseUrl/auth/refresh'),
      body: {'refresh_token': refreshToken},
    );
    if (response.statusCode != 200) {
      throw AuthException('Refresh failed');
    }
    final data = jsonDecode(response.body);
    return AuthResult(
      token: data['access_token'],
      refreshToken: data['refresh_token'],
      expiresAt: DateTime.now().add(
        Duration(seconds: data['expires_in']),
      ),
    );
  }
!@New
  Future<void> logout(String token) async {
    await _client.post(
      Uri.parse('$baseUrl/auth/logout'),
      headers: {'Authorization': 'Bearer $token'},
    );
  }
@/Location

// -- 删除 validateSession() --
!@Location
  Future<bool> validateSession(String token) async {
    final response = await _client.get(
      Uri.parse('$baseUrl/auth/validate'),
      headers: {'Authorization': 'Bearer $token'},
    );
    return response.statusCode == 200;
  }
!@Delete
  Future<bool> validateSession(String token) async {
    final response = await _client.get(
      Uri.parse('$baseUrl/auth/validate'),
      headers: {'Authorization': 'Bearer $token'},
    );
    return response.statusCode == 200;
  }
@/Location

@/Open
```

**关键点**：
- Location 的缩进必须与原文件完全一致（此处为 2-space）
- Delete 的匹配内容必须和 Location 定位到的行**连续且紧邻**

---

### 3.3 Markdown 文档重构

**目标文件** (`tests/data/docs_guide.md`)：

```markdown
## Quick Start
...
## Deployment
...
## Troubleshooting
```

**脚本** (`markdown_sections.ncs`)——替换标题、插入章节、更新代码块：

```ncs
!@Open ./tests/data/docs_guide.md

// -- 标题替换 --
!@Location
## Quick Start
!@Delete
## Quick Start
!@New
## Getting Started
@/Location

// -- 在 Deployment 之后插入 Security 章节 --
!@Location
## Deployment
!@New
## Security

For production security, follow these best practices:

- Enable HTTPS with TLS 1.3
- Rotate API keys every 90 days
- Use environment variables for secrets
@/Location

// -- 更新安装代码块 --
!@Location
```bash
git clone https://github.com/example/project.git
cd project
cargo build --release
```
!@Delete
```bash
git clone https://github.com/example/project.git
cd project
cargo build --release
```
!@New
```bash
git clone https://github.com/example/project.git
cd project
cargo build --release
cargo test
cargo install --path .
```
@/Location

@/Open
```

**关键点**：
- 一个 `!@Open` 下可有多个 `!@Location`——每对 Location 是对文件的独立编辑
- 代码块也可以作为 Location 的匹配内容（注意区分 markdown 的 ```` ``` ```` 和 NCS 语法）

---

### 3.4 Bash 执行 + 文件编辑

```ncs
// -- 执行 bash，输出到终端 --
!@Bash echo "Test timestamp:" && date -u

// -- 打开文件，在末尾追加内容 --
!@Open ./tests/data/docs_guide.md
!@Location
## Troubleshooting
!@New
## Test Results

This section was added by the NCS bash test.
The test ran successfully.
@/Location
@/Open
```

Bash 执行 stdout 会以黄色 `Bash:` 前缀打印到终端。`!@Bash` 结果自动转为 `CmdContent`，可被后续命令（如 Capture）使用。

---

### 3.5 目录操作（Dir 模式）

`!@Open Dir` 将目录序列化为**树形文本**，后续可像操作文件一样使用 Location/New/Delete 管理目录结构。

**查看目录**：

```ncs
!@Read Dir ./src depth=2 filter="*.rs"
```

输出示例：
```
src:
  lib.rs
  main.rs
  engine:
    mod.rs
    executor.rs
```

**删除文件**：

```ncs
!@Open Dir ./src
!@Location
  lib.rs
!@Delete
  lib.rs
@/Open
```

**创建新文件**：

```ncs
!@Open Dir ./src
!@Location
  main.rs
!@New
  new_module.rs
@/Open
```

`@/Open` 时，树形文本的变更反序列化为文件系统操作——新增行对应创建文件，删除行对应删除文件。

---

### 3.6 外部命令（Include）

通过 `!@Include` 注册外部程序为 NCS 命令。以下示例注册一个 Python 脚本并调用：

**Python 脚本** (`tests/data/python_echo.py`)：

```python
#!/usr/bin/env python3
import sys
if len(sys.argv) > 1:
    print(" ".join(sys.argv[1:]))
else:
    print("(no args)")
```

**NCS 脚本** (`include_python.ncs`)：

```ncs
// -- 注册外部命令（exec=default 使用 shebang 执行） --
!@Include ./tests/data/python_echo.py alias=echop

// -- 调用外部命令 --
!@echop Hello from NCS via Include Python!
```

**Include 参数**：

| 参数 | 必填 | 默认值 | 说明 |
|------|:---:|--------|------|
| `alias` | 是 | — | 命令别名（禁止与内置命令重名） |
| `block` | 否 | `false` | 是否支持块执行 |
| `exec` | 否 | `default` | 执行方式：`default`（直接执行）、`bash`（`bash -c`）、`script`（`script -c`） |

---

### 3.7 Capture + Like：捕获输出并伪装执行

Capture 管在 `@/Cmd` 处将命令的最终内容存入全局数据池。Like 将池中内容以伪装身份注入后续流程。

```ncs
// -- Step 1: 打开文件，捕获内容 --
!@Open ./tests/data/docs_guide.md
@/Open | Capture doc_pool

// -- Step 2: 伪装为 Open 命令，让后续编辑有 Open 上下文 --
!@Like doc_pool like=Open

// -- Step 3: 伪装身份下继续编辑 --
!@Open ./tests/data/docs_guide.md
!@Location
## Troubleshooting
!@New
### Performance Issues

If you see high CPU usage, check the worker thread count.
@/Location
@/Open
```

**数据流**：

```
Open → @/Open | Capture doc_pool  →  pools["doc_pool"] = CmdContent
                                        │
!@Like doc_pool like=Open         →  exec_cmds += ("Open", "Normal")
                                        │
!@Open ... (后续编辑正常进行)         ←  check_owner 通过，因为 "Open" 在 exec_cmds 中
```

---

### 3.8 WorkPath + Write + Read

`!@WorkPath` 切换工作目录，影响后续所有相对路径。`!@Write` 写入文件（自动创建父目录），`!@Read` 带语法高亮读取。

```ncs
// -- 切换到测试数据目录 --
!@WorkPath ./tests/data

// -- 写入新配置文件 --
!@Write Normal _ncs_config.toml
# Auto-generated by NCS

[server]
host = "0.0.0.0"
port = 8080
@/Write

// -- 带语法高亮读取 --
!@Read _ncs_config.toml start=1 end=5
```

`!@Write Raw` 是特殊模式：从下一行到脚本末尾的**全部内容**原样写入，其中的 `!@`、`@/` 等标记全部作为原始字符。脚本在此 Write 执行完毕后直接退出。

```ncs
!@Write Raw ./output/script.sh
#!/bin/bash
echo "This will NOT be parsed as NCS"
!@NotACommand
@/NotARealClose
```

---

### 3.9 Raw 内容与 Get 展开

`!@Raw` 是仅展开命令——出现在 New/Delete 块内时不触发块终止，其内容融入父命令。Raw 行的缩进按命令行位置自动计算。

`!@Get pool_name` 从全局数据池取出内容展开到块内，同样按命令行缩进对齐。

```ncs
!@Open ./tests/data/rust_service.rs

// -- 使用 Raw 插入多行（缩进自动对齐） --
!@Location
    pub fn list_active(&self) -> Vec<&User> {
        self.users
            .values()
            .filter(|u| u.active)
            .collect()
    }
!@New
    pub fn find_by_name(&self, name: &str) -> Vec<&User> {
!@Raw self.users.values()
!@Raw     .filter(|u| u.name.contains(name))
!@Raw     .collect()
    }
@/Location

@/Open
```

**缩进规则**：
- `!@Raw` 行的 taps = 命令行前导空格数
- 后续 Raw 行按 diff_taps 偏移
- `!@Get` 展开同理：第一行 taps = `!@Get` 命令行的缩进，后续行按 pool 内容首行的 diff_taps 偏移

---

### 3.10 全命令流水线

一次脚本中组合 Bash、Open、Location、New、Delete Block、New End：

```ncs
// -- Bash 时间戳 --
!@Bash echo "Full Pipeline Test" && date -u

// -- 打开 Rust 文件 --
!@Open ./tests/data/rust_service.rs

// -- 为 User 添加 Display 实现 --
!@Location
pub struct User {
    pub id: u64,
    pub name: String,
    pub email: String,
    pub active: bool,
}
!@New
impl std::fmt::Display for User {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "User({}, {}, active={})", self.id, self.name, self.active)
    }
}
@/Location

// -- Block 删除：删除整个 count_by_domain 方法 --
!@Location
    pub fn count_by_domain(&self, domain: &str) -> usize {
        self.users
            .values()
            .filter(|u| u.email.ends_with(domain))
            .count()
    }
!@Delete Block
@/Location

// -- 在文件末尾新增 total_users 方法 --
!@New End
/// Returns the total number of registered users.
pub fn total_users(&self) -> usize {
    self.users.len()
}

@/Open
```

**关键点**：
- `!@Delete Block` 删除整个 Location 定位到的代码块
- `!@New End` 直接追加到文件末尾，无需 Location
- `!@New End` 的 `@/New` 可以省略——脚本末尾的 `@/Open` 会隐式关闭

---

## 四、命令速查表

### 文件编辑

| 命令 | 格式 | 说明 |
|------|------|------|
| **Open** | `!@Open [Normal\|Dir] <path> [start=N] [end=N]` | 打开文件/目录 |
| **Location** | `!@Location [Normal\|Block]` + 内容 | 定位代码位置 |
| **New** | `!@New [Normal\|Start\|End]` + 内容 | 插入新内容 |
| **Delete** | `!@Delete [Normal\|Block]` + 内容 | 删除匹配内容 |
| **Raw** | `!@Raw <content>` | 字面量内容（仅展开） |

### 系统命令

| 命令 | 格式 | 说明 |
|------|------|------|
| **Bash** | `!@Bash <command>` | bash -c 执行 |
| **Exec** | `!@Exec <command>` | script -c 直连终端 |
| **Read** | `!@Read [Normal\|Dir] <path>` | 带高亮读取文件/目录 |
| **Write** | `!@Write [Normal\|Raw] <path>` + 内容 | 写入文件 |

### 元命令

| 命令 | 格式 | 说明 |
|------|------|------|
| **Include** | `!@Include <path> alias=X [exec=Y]` | 注册外部命令 |
| **WorkPath** | `!@WorkPath <path>` | 切换工作目录 |
| **Get** | `!@Get <pool_name>` | 从数据池取值 |
| **Like** | `!@Like <pool_name> like=Cmd [Mode]` | 伪装执行 |
| **Capture** | `@/Open \| Capture <name>` | 捕获命令输出到数据池 |

### 关闭符号

| 符号 | 作用 |
|------|------|
| `@/Open` | 写回文件并退出 |
| `@/Location` | 弹出定位块栈 |
| `@/New` | 完成插入操作 |
| `@/Delete` | 完成删除操作 |

---

## 五、错误输出格式

```
Error: 标题（黄色）
  详情行（灰色）
  详情行（灰色）
  Hint: 修复建议（绿色）
  Hint: 修复建议（绿色）
```

常见错误：

| 错误 | 原因 | 解决 |
|------|------|------|
| `Location 命令未找到任何匹配` | 定位内容在文件中找不到 | 检查缩进是否与目标文件一致 |
| `Location 命令找到多个匹配` | 定位内容不够精确 | 多写几行内容做精确匹配 |
| `Block 栈为空` | @/Location 数量与 !@Location 不匹配 | 确保成对出现 |
| `OwnerNotExecuted` | 从属命令缺少前置的 owner | New 前面必须有 Location |

管道/重定向时自动关闭颜色输出。

---

## 六、与 n_edit (.ned) 的对比

| 功能 | n_edit | NCS |
|------|--------|-----|
| 命令前缀 | `//!@Open: path` | `!@Open path` |
| 内容分隔 | `...` 显式分隔符 | 下一命令自动终止 |
| 关闭 | `//!@Off:Open` | `@/Open` |
| 行号定位 | `@66,120` | 已移除（用嵌套 Location 替代） |
| 命令数 | 5 | 13 + Include 动态扩展 |
