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

**608 tests 通过**（n_edit 297 + NCS 311），clippy 0 warnings，fmt clean。

---

### Phase 4：新增独立命令 + Open Dir 模式 ✅

**693 tests 通过**（n_edit 297 + NCS 396），clippy 0 warnings，fmt clean。

#### 已实现命令

| 命令 | 文件 | 行数 | 关键特性 |
|------|------|:---:|------|
| **Write** | `commands/write.rs` | ~35 | Normal 模式写文件（自动创建父目录）；Raw 模式 Lexer 收集到 EOF 全量原样写入；输出 `written {path} {size}` |
| **Bash** | `commands/bash.rs` | ~90 | `bash -c` 执行 + `security_check()` 安全审查；stdout→CmdContent(stderr→result)；输出黄色 `Bash:` 前缀 |
| **Exec** | `commands/exec.rs` | ~35 | `script -c` 直连终端，支持彩色/交互/流式输出；值输出 |
| **Read** | `commands/read.rs` | ~320 | Normal 模式支持 start/end（默认最多 1000 行）；`syntect` 语法高亮；Dir 模式树形结构，支持 depth/ignore/filter |
| **Include** | `commands/include.rs` | ~150 | 动态注册外部命令到 `CommandRegistry`；所有位置参数拼接为执行指令；`ExecMethod::{Default,Bash,Script}` |
| **WorkPath** | `commands/work_path.rs` | ~30 | 验证路径存在 → 更新 `engine.work_path` + `set_current_dir()`；默认取自脚本父目录 |
| **Get** | `commands/get.rs` | ~30 | 从 `engine.pools` 按 pool_name 取值并克隆返回；作为 `ExpandOnly` 命令在块内展开（缩进计算见 Phase 5） |
| **Open Dir** | `commands/open.rs` | ~240 | 递归扫描目录 → 序列化为树形文本，退出时反序列化创建/删除文件 |

---

## Phase 5：Get 展开 + Like 伪装 + Raw 缩进 ✅

### 5.1 Get 块内展开（缩进感知）✅

- `CmdLine` 新增 `expand_from_pool: Option<String>` 字段
- `NewLine` 新增 `expand_from_pool: Option<String>` 字段
- `parse_new_content()` 中对 `!@Get` 行计算正确的 diff_taps 并设置 `expand_from_pool`
- `convert()` 接收 pools 引用，调用 `expand_pool_content()` 按缩进规则展开
- 展开算法：首行 taps = 命令行前导空格，后续行按 pool 首行的 diff_taps 偏移

### 5.2 Like 伪装命令 ✅

- 新增 `Command::Like { pool_name, like_cmd, like_mode }` 变体
- 新增 `commands/like.rs`（~70 行）
- Parser：`!@Like pool_name like=CommandName [ModeName]`，like 为必填参数
- Engine：从 pools 取值 → 伪装条目写入 exec_cmds → 设置 last_result
- `ExecutedCommand` 通过 cmd_name 匹配；`@/Cmd` 关闭时正常清理

### 5.3 Raw 缩进计算 ✅

- `parse_new_content()` 中 Raw 行：`is_raw = false`，按命令行缩进计算 diff_taps
- `parse_delete_content()` 同理
- `merge_raw_commands()` 中的独立 Raw 命令暂保持 `is_raw = true`

### 5.4 设计决策

| 决策 | 说明 |
|------|------|
| Get/Like 分离 | `!@Get` 仅用于块内展开（ExpandOnly），`!@Like` 专注伪装执行（LineExec + StreamOutput） |
| `{}` 占位符替换 | 已废除，暂不实现 |
| like 不实际执行 | Like 仅在 exec_cmds 中写入伪装条目，不执行 like= 后面的命令 |

### 5.5 验证

```bash
cargo test -p ncs --lib        # 327 passed, 0 failed
cargo test -p n_edit --lib     # 244 passed, 0 failed
cargo test --test integration_test  # 70 passed（2 个 WorkPath 共享状态偶发失败，与本次无关）
cargo clippy --workspace -- -D warnings  # 0 warnings
cargo fmt --check              # clean
```

---

## Phase 5 交付总结

### 变更文件（13 个，+360 / -141 行）

| 文件 | 变更 |
|------|------|
| `docs/phases.md` | 重划阶段五/六边界，补充设计决策 |
| `docs/ncs_dev.md` | 更新 Get/Like 定义、CmdContent 字段、模块架构、开发阶段 |
| `ncs/src/registry.rs` | Like 注册（LineExec + StreamOutput，必填 like 参数）；Get 移除 like 参数 |
| `ncs/src/parser.rs` | `Command::Like` 变体 + `parse_like()`；`parse_get()` 简化；`convert()` 接受 pools 参数 + `expand_pool_content()` 展开算法；`parse_new_content()` Raw/Get 缩进计算 |
| `ncs/src/commands/like.rs` | **新文件**：Like 命令执行 |
| `ncs/src/commands/mod.rs` | 导出 like 模块 |
| `ncs/src/cmd_content.rs` | `CmdLine.expand_from_pool` 字段 + `CmdLine::new()` 构造器 |
| `ncs/src/model.rs` | `NewLine.expand_from_pool` 字段 |
| `ncs/src/engine/mod.rs` | convert 调用传 pools；LIKE 加入 exec_cmds 排除列表 |
| `ncs/src/engine/command_pipeline.rs` | Like 分发（execute_core + out） |
| `ncs/src/commands/bash.rs` | 构造函数补全 |
| `ncs/src/commands/delete.rs` | 构造函数补全 |
| `ncs/src/commands/new.rs` | 构造函数补全 |
| `ncs/src/commands/read.rs` | 构造函数补全 |
| `ncs/src/engine/executor.rs` | 构造函数补全 |

### 新增命令清单

| 命令 | 语法 | 功能 |
|------|------|------|
| **Like** | `!@Like pool_name like=Cmd [Mode]` | 伪装执行：将 pool 内容以伪装身份注入后续命令的 owner 检查链 |

### 命令总数：13（12 内置 + Include 动态注册）

---

## Phase 6：错误处理 + 终端输出打磨 ⚠️

**说明**：错误骨架已就位（`Timeout`/`IncludeFailed` 变体已定义），但实际运行时逻辑尚未接入。

### 关键任务

| 任务 | 位置 | 状态 | 说明 |
|------|------|:--:|------|
| `RegistryError` 补全 | `error.rs` | ❌ | `CommandNotFound.suggestion` 始终为 None，需实现 Levenshtein 相似度计算 |
| `CommandExecError` 补全 | `error.rs` | ❌ | `Timeout` 已定义但 Bash/Exec 未接入超时；`IncludeFailed` 原因分类待细化 |
| Diff 从 ContentChange 构建 | `engine/command_pipeline.rs` | ❌ | 当前从原始 block 快照收集 diff，ContentChange→DiffLine 路径未启用 |
| New::Start/End 桥接消除 | `engine/command_pipeline.rs` | ❌ | `execute_new_start()/end()` 直接操作 `engine.file.lines`，应迁移至 `apply_content_to_file()` |
| `--quiet` 标志补全 | `main.rs` | ❌ | 当前仅抑制 diff 消息，应扩展为抑制所有非错误输出 |
| Bash/Exec 超时机制 | `commands/bash.rs`, `commands/exec.rs` | ❌ | 当前无超时控制 |
| Read 高亮稳定性 | `commands/read.rs` | ❌ | `syntect` 首次加载有并发竞争风险 |

---

## 当前架构速览

```
ncs/src/
├── main.rs             CLI 入口 (clap)                           (~220 行)
├── lib.rs              库入口
├── lexer.rs            词法 → Token 流                           (~1005 行)
├── parser.rs           Token → Command AST + convert()           (~1519 行)
├── engine/
│   ├── mod.rs          状态机 + Engine 方法 + 测试                (~1787 行)
│   ├── command_pipeline.rs  Command execute_core/out + 子方法    (~481 行)
│   └── executor.rs     纯函数辅助 (diff/delete/match)            (~927 行)
├── commands/
│   ├── mod.rs          命令模块入口
│   ├── open.rs         !@Open (含 Dir serialization)             (~490 行)
│   ├── location.rs     !@Location                                (~224 行)
│   ├── new.rs          !@New                                     (~343 行)
│   ├── delete.rs       !@Delete                                  (~653 行)
│   ├── raw.rs          !@Raw                                     (~50 行)
│   ├── bash.rs         !@Bash                                    (~90 行)
│   ├── exec.rs         !@Exec                                    (~43 行)
│   ├── get.rs           !@Get                                     (~30 行)
│   ├── like.rs          !@Like                                    (~70 行)   🆕
│   ├── read.rs         !@Read                                    (~320 行)
│   ├── write.rs        !@Write                                   (~36 行)
│   ├── include.rs      !@Include                                 (~160 行)
│   └── work_path.rs    !@WorkPath                                (~37 行)
├── registry.rs         命令注册表 + ExecMethod                    (~885 行)
├── cmd_content.rs      数据管道 + 变更追踪                        (~571 行)
├── model.rs            基础数据结构                               (~761 行)
├── matcher.rs          匹配算法                                   (~814 行)
├── block.rs            Block 解析                                 (~646 行)
├── output.rs           终端 diff 输出                             (~415 行)
├── error.rs            错误类型                                   (~874 行)
└── file_io.rs          文件读写工具                               (~99 行)
```

**当前测试**：698（n_edit 297 + NCS lib 327 + NCS main 4 + integration 70 — 其中 2 个因 WorkPath set_current_dir 共享状态偶发失败，单独运行通过）

**验证命令**：
```bash
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --check
```
