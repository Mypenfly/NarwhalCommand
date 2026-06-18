# NCS 实现阶段

## 已完成阶段

### Phase 0-2：基础设施与核心命令 ✅

Lexer（`!@Cmd` 词法）→ Parser（12 种命令 AST）→ Engine（状态机 + `exec_cmds`/`block_stack`/`pools`）+ 5 个文件编辑命令（`open`/`location`/`new`/`delete`/`raw`），从 n_edit 迁移了 `matcher`/`block`/`model`/`output`。启动 `cargo run -p ncs -- file.ncs` 即可执行 `.ncs` 脚本。

### Phase 3：CmdContent 数据流 + 变更追踪 ✅

三步流水线完成，命令不再直接修改 `ContentBlock`：

```
Open     → convert(empty) → execute_core(读文件) → out() → CmdContent(snapshot=文件行, source=File)
Location → convert(input) → execute_core(匹配定位) → out() → CmdContent(snapshot=block行, source=Block)
New      → convert(input) → execute_core(record_insert) → out() → CmdContent(changes+=Insert)
Delete   → convert(input) → execute_core(快照匹配+record_delete) → out() → CmdContent(changes+=Delete)
@/Cmd    → handle_close → apply_content_to_file() → apply_changes() → block.lines替换 → write_back
```

**关键架构要点**：
- `CmdContent` 新增 `snapshot_lines`（Location 原始快照，不可变）、`changes: Vec<ContentChange>`（变更记录）、`source_info: Option<ContentSource>`（Block/File/CommandOutput）、`pending_new_lines`（三步流水线内部传递）
- `Command::convert()` 在 `parser.rs`，`execute_core()`/`out()` 在 `engine/mod.rs`（避免循环依赖）
- `ContentChange::{Insert, Delete}` 携带 `source_cmd` 追踪变更来源
- `apply_changes()` 在 Owner 关闭时统一将 snapshot + changes 计算为最终 lines
- `apply_content_to_file()` 目前仅处理 `Block` source；`File` source（New::Start/End）仍为桥接模式

**608 tests 通过**（n_edit 297 + NCS 311），clippy 0 warnings，fmt clean。

---

## Phase 4：新增独立命令

**状态**：所有命令已在 `CommandRegistry` 中注册（`registry.rs`），Lexer/Parser 可解析，`Engine.execute()` 走到 `NotImplemented` 分支。需接入 CmdContent 管道。

### 实现顺序

| 优先级 | 命令 | 理由 |
|:------:|------|------|
| 1 | **Write** | 最独立，`Normal`（块写入文件）、`Raw`（EOF 原文写入），不依赖其他命令 |
| 2 | **Bash** | 行执行，需安全审查；输出通过 CmdContent 进入管道 |
| 3 | **Exec** | 行执行，`script -c` 直连终端；值输出（不保留） |
| 4 | **Read** | 复用 Open 的文件读取，值输出，带高亮 |
| 5 | **Include** | 扩展 `CommandRegistry`，需校验重名 |
| 6 | **WorkPath** | 无类型元命令，修改进程工作目录 |

### 各命令流水线设计

每个命令需实现三步流水线，按现有模式在 `engine/mod.rs` 的 `impl Command` 中填充对应 match arm：

#### Write

```
convert(): CmdContent::empty()（无上游依赖）
execute_core():
  - Normal: 将 internal 的 pending 行写入 engine.file（调用 file_io::write_file）
  - Raw: 从 next line 到 EOF 全部原样收集，写入后程序退出
out(): CmdContent::empty()（值输出，不保留）
```

**注意**：`Write Raw` 模式在 Lexer 阶段应标记为特殊提取（到 EOF），与现有的 `!@Write Raw` Token 处理关联。

#### Read

```
convert(): CmdContent::empty()
execute_core(): 调用 file_io::read_file，读取内容存入 CmdContent.lines
out(): result（带格式化的文件内容行）
```

**复用**：Open 的文件读取逻辑（`FileContent::from_path()`）可直接复用。

#### Bash

```
convert(): CmdContent::empty()
execute_core():
  1. 从 self.command 获取命令字符串
  2. 安全审查：拦截 sudo/rm -rf /chmod 777 / 等高危模式
  3. std::process::Command::new("bash").arg("-c").arg(command).output()
  4. stdout → CmdContent::from_raw_text()
  5. stderr → 追加到 CmdContent.result
out(): result（source_info = CommandOutput）
```

**安全审查位置**：建议放在 `engine/executor.rs` 新增 `fn security_check(command: &str) -> Result<(), CommandExecError>`。

#### Exec

```
convert(): CmdContent::empty()
execute_core(): std::process::Command::new("script").arg("-c").arg(command).status()
out(): CmdContent::empty()（值输出）
```

#### Include

```
convert(): CmdContent::empty()
execute_core(): 解析 self.args（alias/block/type/exec/owners/subs），
                构建 CommandEntry，调用 registry.register(entry)
out(): CmdContent::empty()（无输出）
```

**校验**：alias 不与内置命令重名 → `RegistryError::AliasConflict`。

#### WorkPath

```
convert(): CmdContent::empty()
execute_core(): 验证 self.path 存在 → std::env::set_current_dir()
out(): CmdContent::empty()
```

### 接入流水线修改点

1. **`engine/mod.rs` `execute_core()`**：为每个 Phase 4 命令添加 match arm（当前落 `_ => NotImplemented`）
2. **`engine/mod.rs` `out()`**：Bash 需要 source_info = CommandOutput
3. **`engine/executor.rs`**：新增 `security_check()` 辅助函数
4. **`error.rs`**：`CommandExecError` 已有变体框架（`SecurityDenied`/`ExecutionFailed`/`Timeout`），按需填充 `title()`/`detail()`/`hints()`
5. **`cmd_content.rs`**：Bash/Read 的输出使用 `from_raw_text()` 构造

### 验证

```bash
cargo test -p ncs -- bash exec read write include work_path
cargo test --test integration_test   # 新增 Phase 4 场景脚本
cargo clippy -- -D warnings
cargo fmt --check
```

---

## Phase 5：Get 高级特性 + like 伪装

### 当前状态

`!@Get pool_name` 基本读取已实现（从 `engine.pools` 取值，透传 CmdContent）。缺失两项：

### 5.1 块内展开

**目标**：在 New/Delete 块内遇到 `!@Get pool_name` 时，展开为 raw_content 融入父命令内容。

**修改点**：
- `parser.rs`：在 `parse_new_content` / `parse_delete_content` 中检测 `!@Get` Token，将其对应的 pools 中的 `raw_content` 展开为内容行，标记 `is_raw`
- 需要在 Parse 阶段访问 pools？不——pools 在引擎运行时填充。Get 块内展开只能在引擎执行时处理：
  - 方案 A：在 `convert` 阶段检测 pending 中的 Get 标记，从 engine.pools 读取并展开
  - 方案 B：在 Lexer 输出中将 `!@Get` 标记为 `ExpandOnly` Token，Parser 保留为占位符，`execute_core` 时展开

### 5.2 like 伪装模式

**目标**：`!@Get pool_name like=Open` 让后续命令以伪装身份执行。

**修改点**：
- `engine/mod.rs`：在 `execute_core` 的 `Command::Get` 分支中，若 `like` 为 Some，将内容存入 `last_result` 并在 `exec_cmds` 中写入伪装的 `ExecutedCommand { cmd_name: like_value }`
- 后续命令的 `check_owner()` 会识别到伪装的 owner
- 关闭时 `@/Cmd` 按正常逻辑清理
- `{}` 占位符替换：在 CmdContent 中查找 `{key}` 模式并替换为对应的值

### 验证

```bash
cargo test -p ncs -- cmd_content get parser
cargo test --test integration_test
```

---

## Phase 6：错误处理 + 终端输出打磨

**说明**：可与 Phase 4/5 并行。

### 关键任务

| 任务 | 位置 | 说明 |
|------|------|------|
| `RegistryError` 补全 | `error.rs` | `CommandNotFound`（含字符相似度提示）、`ModeNotFound`、`AliasConflict` 已有框架，补全 hints |
| `CommandExecError` 补全 | `error.rs` | `ExecutionFailed`/`SecurityDenied`/`Timeout`/`IncludeFailed` 已有框架，按需填充 |
| Diff 从 ContentChange 构建 | `engine/executor.rs` | 替代当前从原始 block 快照收集 diff 的桥接（`apply_content_to_file` 时从 changes 生成 DiffLine） |
| New::Start/End 桥接消除 | `engine/mod.rs` | 将文件级直接 `file.lines` 操作迁移至 File source 的 `apply_content_to_file()` |
| `--quiet` 标志 | `main.rs` | 抑制成功消息和 diff，仅输出错误 |

---

## 当前架构速览

```
src/
├── main.rs          CLI 入口 (clap)
├── lib.rs           库入口
├── lexer.rs         词法 → Token 流     (~859 行)
├── parser.rs        Token → Command AST  (~1234 行)  含 convert()
├── engine/
│   ├── mod.rs       状态机 + execute_core/out  (~980 行)
│   └── executor.rs  纯函数辅助            (~570 行)
├── commands/
│   ├── mod.rs       命令模块入口
│   ├── open.rs      !@Open               (~217 行)
│   ├── location.rs  !@Location           (~224 行)
│   ├── new.rs       !@New                (~343 行)
│   ├── delete.rs    !@Delete             (~651 行)
│   └── raw.rs       !@Raw                (~50 行)
├── registry.rs      命令注册表 (12 命令)  (~858 行)
├── cmd_content.rs   数据管道 + 变更追踪   (~561 行)
├── model.rs         基础数据结构          (~761 行)
├── matcher.rs       匹配算法              (~814 行)
├── block.rs         Block 解析            (~646 行)
├── output.rs        终端 diff 输出        (~415 行)
├── error.rs         错误类型              (~927 行)
└── file_io.rs       文件读写工具          (~99 行)
```

**测试**：608 total（n_edit 297 + NCS lib 256 + NCS main 4 + integration 51）

**验证命令**：
```bash
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --check
```
