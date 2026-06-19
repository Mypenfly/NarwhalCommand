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
| **Bash** | `commands/bash.rs` | ~90 | `bash -c` 执行 + `security_check()` 安全审查（拦截 sudo/rm -rf //chmod 777 //mkfs/dd/forkbomb）；stdout→CmdContent(stderr→result)；输出黄色 `Bash:` 前缀 |
| **Exec** | `commands/exec.rs` | ~35 | `script -c` 直连终端，支持彩色/交互/流式输出；值输出 |
| **Read** | `commands/read.rs` | ~320 | Normal 模式支持 start/end（默认最多 1000 行）；`syntect` 语法高亮（base16-ocean.dark 主题 + 24-bit 终端颜色）；Dir 模式树形结构，支持 depth/ignore/filter 参数 |
| **Include** | `commands/include.rs` | ~150 | 动态注册外部命令到 `CommandRegistry`；所有位置参数拼接为执行指令；根据 `work_path` 展开 `./` `../` 相对路径；`ExecMethod::{Default,Bash,Script}` 三种执行策略；alias 冲突检测 |
| **WorkPath** | `commands/work_path.rs` | ~30 | 验证路径存在 → 更新 `engine.work_path` + `set_current_dir()`；默认取自脚本父目录 |
| **Get** | `commands/get.rs` | ~30 | 从 `engine.pools` 按 pool_name 取值并克隆返回；like 伪装模式待 Phase 5 |
| **Open Dir** | `commands/open.rs` | ~240 | 递归扫描目录 → 序列化为树形文本（`dirname:\n  file1.rs\n  subdir:\n    file3.py`）→ 存入 CmdContent；支持 depth/ignore/filter 参数；退出时反序列化树形文本 → 创建/删除文件；diff 输出与文件操作一致 |

#### 架构新增

| 模块 | 新增内容 |
|------|----------|
| `registry.rs` | `ExecMethod` 枚举（Default/Bash/Script）；`CommandEntry.exec_method` 字段 |
| `parser.rs` | `ReadMode` 枚举（Normal/Dir）；`Command::Read.mode`；`Command::External { name, positional_args }`；`auto_detect_open_mode`/`auto_detect_read_mode` 根据路径自动识别模式；移除 `LocationMode::Path` |
| `engine/mod.rs` | `work_path: PathBuf`（路径基准）；`had_output: bool`；`print_command_output()`；`is_dir_mode: bool`；`dir_snapshot: Option<String>`；`write_back_dir()` 目录反序列化写回 |
| `engine/command_pipeline.rs` | ⚡ 从 engine/mod.rs 拆分：Command::execute_core() + out()，大臂拆为独立子方法 |
| `lexer.rs` | 未知命令宽容处理（不报错，创建 line-exec Token）；`extract_block_content()` Write Raw 到 EOF 特殊逻辑 |
| `main.rs` | 脚本父目录传入 `engine.work_path`；默认消息 `"(no output)"` |
| `model.rs` | `FileContent::from_text()` 从字符串构建（不依赖文件系统） |

#### 与设计偏差的修正

| 偏差 | 修正 |
|------|------|
| Include 只取第一个位置参数 | 改为全部 positional_args 拼接为执行指令 |
| Include 缺少执行策略 | 新增 `ExecMethod`，根据 `exec` 参数选 Default/Bash/Script |
| Read/Open 无自动模式检测 | 无 mode 时根据文件系统自动判断 Normal/Dir |
| Lexer/Parser 对未知命令报错 | 改为宽容处理，由 Engine 运行时根据 Registry 校验（支持 Include 动态注册） |
| 命令无终端输出 | 新增 `print_command_output()` + `had_output` 统一管理 |
| Open Dir 模式未实现 | 实现树形文本序列化/反序列化，Dir 内容与文件内容一样的操作方式 |
| Location Path 模式 | **已废除**，改为直接在 Dir 树形文本中使用标准 Location/New/Delete |
| Get 缺少独立文件 | 提取为 `commands/get.rs` |

---

## Phase 5：Get 高级特性 + like 伪装 + 块内展开 ⚠️

### 当前状态

`!@Get pool_name` 基本读取已实现（从 `engine.pools` 取值，透传 CmdContent）。
但以下特性待实现：

### 5.1 块内展开 ❌

**目标**：在 New/Delete 块内遇到 `!@Get pool_name` 时，展开为 raw_content 融入父命令内容。

**建议实现路径**：
- Lexer 已将 `!@Get` 标记为 `ExpandOnly` Token（不触发块终止），Parser 转为 `Command::Get`
- 在 Engine 的 `convert()` 阶段检测 pending 中的 `Command::Get`，从 `engine.pools` 读取并展开 `raw_content` 为内容行，标记 `is_raw`
- 展开后的内容融入父命令的 `pending_new_lines` / `pending_delete_lines`

### 5.2 like 伪装模式 ❌

**目标**：`!@Get pool_name like=Open` 让后续命令以伪装身份执行。

**当前偏差**：Parser 已解析 `like` 参数，但 `engine/command_pipeline.rs:Get` 分支（`like: _like`）完全忽略该参数。

**建议实现路径**：
- `execute_core` 的 `Command::Get` 分支：若 `like` 为 Some：
  - 将 `like_value` 写入 `exec_cmds` 作为伪装的 `ExecutedCommand`
  - 将 pool 内容存入 `last_result`
  - 后续命令的 `check_owner()` 识别到伪装的 owner 后放行
  - `@/Cmd` 关闭时正常清理伪装条目
- `{}` 占位符替换：在 CmdContent 中查找 `{key}` 模式，替换为 pools 中的对应值

### 5.3 验证

```bash
cargo test -p ncs -- cmd_content get parser
cargo test --test integration_test
```

---

## Phase 6：错误处理 + 终端输出打磨 ⚠️

**说明**：错误骨架已就位（`Timeout`/`IncludeFailed` 变体已定义），但实际运行时逻辑尚未接入。

### 关键任务

| 任务 | 位置 | 状态 | 说明 |
|------|------|:--:|------|
| `RegistryError` 补全 | `error.rs` | ❌ | `CommandNotFound.suggestion` 字段已定义，但 Levenshtein 字符相似度计算未实现，实际 suggestion 始终为 None |
| `CommandExecError` 补全 | `error.rs` | ❌ | `Timeout` 变体已定义但 Bash/Exec 模块未接入超时机制；`IncludeFailed` 已定义但原因分类可进一步细化 |
| Diff 从 ContentChange 构建 | `engine/command_pipeline.rs` | ❌ | 当前从原始 block 快照收集 diff（`engine/mod.rs` 的 `record_diff_with_context`），apply_content_to_file 的 `#[allow(dead_code)]` 标记说明 ContentChange→DiffLine 路径未启用 |
| New::Start/End 桥接消除 | `engine/command_pipeline.rs` | ❌ | `execute_new_start()`/`execute_new_end()` 直接操作 `engine.file.lines`（桥接模式），应迁移至 File source 的 `apply_content_to_file()` |
| `--quiet` 标志补全 | `main.rs:113` | ❌ | 当前仅抑制 diff 消息，应扩展为抑制所有非错误输出 |
| Open Dir 模式实现 | `commands/open.rs` | ✅ | 递归扫描目录 → 树形文本序列化；支持 depth/ignore/filter 参数；退出时反序列化创建/删除文件 |
| Location Path 模式 | — | 🗑️ | **已废除**，改为在 Dir 树形文本中使用标准 Location/New/Delete 操作 |
| Bash/Exec 超时机制 | `commands/bash.rs`, `commands/exec.rs` | ❌ | 当前无超时控制，长时间命令会永久阻塞 |
| Read 高亮稳定性 | `commands/read.rs` | ❌ | `syntect` 初始化线程安全（当前用 `OnceLock` 但首次加载有并发竞争风险） |

---

## 当前架构速览

```
ncs/src/
├── main.rs             CLI 入口 (clap)                           (~220 行)
├── lib.rs              库入口
├── lexer.rs            词法 → Token 流                           (~1005 行)
├── parser.rs           Token → Command AST + convert()           (~1519 行)
├── engine/
│   ├── mod.rs          状态机 + Engine 方法 + 测试                (~1787 行)  ⚡
│   ├── command_pipeline.rs  Command execute_core/out + 子方法    (~481 行)   🆕
│   └── executor.rs     纯函数辅助 (diff/delete/match)            (~927 行)
├── commands/
│   ├── mod.rs          命令模块入口
│   ├── open.rs         !@Open (含 Dir serialization)             (~490 行)  🆕
│   ├── location.rs     !@Location                                (~224 行)
│   ├── new.rs          !@New                                     (~343 行)
│   ├── delete.rs       !@Delete                                  (~653 行)
│   ├── raw.rs          !@Raw                                     (~50 行)
│   ├── bash.rs         !@Bash                                    (~90 行)
│   ├── exec.rs         !@Exec                                    (~43 行)
│   ├── get.rs           !@Get                                     (~30 行)   🆕
│   ├── read.rs         !@Read                                    (~320 行)  🆕
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

tests/
├── data/               测试用源文件 (16 个)
├── scripts/            测试脚本 (45 个 .ncs)
└── integration_test.rs 集成测试 (~1250 行)
```

🆕 = 新增文件，⚡ = 重构文件

**测试总计**：693（n_edit 297 + NCS lib 322 + NCS main 4 + integration 70）

**验证命令**：
```bash
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --check
```
