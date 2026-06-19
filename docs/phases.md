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

### Phase 4：新增独立命令 ✅

**662 tests 通过**（n_edit 297 + NCS 365），clippy 0 warnings，fmt clean。

#### 已实现命令

| 命令 | 文件 | 行数 | 关键特性 |
|------|------|:---:|------|
| **Write** | `commands/write.rs` | ~35 | Normal 模式写文件（自动创建父目录）；Raw 模式 Lexer 收集到 EOF 全量原样写入；输出 `written {path} {size}` |
| **Bash** | `commands/bash.rs` | ~90 | `bash -c` 执行 + `security_check()` 安全审查（拦截 sudo/rm -rf //chmod 777 //mkfs/dd/forkbomb）；stdout→CmdContent(stderr→result)；输出黄色 `Bash:` 前缀 |
| **Exec** | `commands/exec.rs` | ~35 | `script -c` 直连终端，支持彩色/交互/流式输出；值输出 |
| **Read** | `commands/read.rs` | ~135 | 复用 `FileContent::from_path()`；`syntect` 语法高亮（Solarized dark 主题 + 24-bit 终端颜色）；行号灰色右对齐；Dir 模式列出目录（目录名蓝加粗） |
| **Include** | `commands/include.rs` | ~150 | 动态注册外部命令到 `CommandRegistry`；所有位置参数拼接为执行指令；根据 `work_path` 展开 `./` `../` 相对路径；`ExecMethod::{Default,Bash,Script}` 三种执行策略；alias 冲突检测 |
| **WorkPath** | `commands/work_path.rs` | ~30 | 验证路径存在 → 更新 `engine.work_path` + `set_current_dir()`；默认取自脚本父目录 |

#### 架构新增

| 模块 | 新增内容 |
|------|----------|
| `registry.rs` | `ExecMethod` 枚举（Default/Bash/Script）；`CommandEntry.exec_method` 字段 |
| `parser.rs` | `ReadMode` 枚举（Normal/Dir）；`Command::Read.mode`；`Command::External { name, positional_args }`；`auto_detect_open_mode`/`auto_detect_read_mode` 根据路径自动识别模式 |
| `engine/mod.rs` | `work_path: PathBuf`（路径基准）；`had_output: bool`（控制默认输出消息）；`print_command_output()`（命令终端打印）；`execute()` 签名 `registry: &mut CommandRegistry` |
| `lexer.rs` | 未知命令宽容处理（不报错，创建 line-exec Token）；`extract_block_content()` Write Raw 到 EOF 特殊逻辑 |
| `main.rs` | 脚本父目录传入 `engine.work_path`；默认消息 `"(no output)"` |

#### 与设计偏差的修正

| 偏差 | 修正 |
|------|------|
| Include 只取第一个位置参数 | 改为全部 positional_args 拼接为执行指令 |
| Include 缺少执行策略 | 新增 `ExecMethod`，根据 `exec` 参数选 Default/Bash/Script |
| Read/Open 无自动模式检测 | 无 mode 时根据文件系统自动判断 Normal/Dir |
| Lexer/Parser 对未知命令报错 | 改为宽容处理，由 Engine 运行时根据 Registry 校验（支持 Include 动态注册） |
| 命令无终端输出 | 新增 `print_command_output()` + `had_output` 统一管理 |

---

## Phase 5：Get 高级特性 + like 伪装 + 块内展开

### 当前状态

`!@Get pool_name` 基本读取已实现（从 `engine.pools` 取值，透传 CmdContent）。

### 5.1 块内展开

**目标**：在 New/Delete 块内遇到 `!@Get pool_name` 时，展开为 raw_content 融入父命令内容。

**建议实现路径**：
- Lexer 已将 `!@Get` 标记为 `ExpandOnly` Token（不触发块终止），Parser 转为 `Command::Get`
- 在 Engine 的 `convert()` 阶段检测 pending 中的 `Command::Get`，从 `engine.pools` 读取并展开 `raw_content` 为内容行，标记 `is_raw`
- 展开后的内容融入父命令的 `pending_new_lines` / `pending_delete_lines`

### 5.2 like 伪装模式

**目标**：`!@Get pool_name like=Open` 让后续命令以伪装身份执行。

**建议实现路径**：
- `engine/mod.rs` `execute_core` 的 `Command::Get` 分支：若 `like` 为 Some：
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

## Phase 6：错误处理 + 终端输出打磨

**说明**：可与 Phase 5 部分并行。

### 关键任务

| 任务 | 位置 | 说明 |
|------|------|------|
| `RegistryError` 补全 | `error.rs` | `CommandNotFound` 添加字符相似度（Levenshtein）提示候选命令 |
| `CommandExecError` 补全 | `error.rs` | `Timeout` 变体接入超时机制；`IncludeFailed` 完善原因分类 |
| Diff 从 ContentChange 构建 | `engine/executor.rs` | 替代当前从原始 block 快照收集 diff 的桥接（`apply_content_to_file` 时从 changes 生成 DiffLine） |
| New::Start/End 桥接消除 | `engine/mod.rs` | 将文件级直接 `file.lines` 操作迁移至 File source 的 `apply_content_to_file()` |
| `--quiet` 标志补全 | `main.rs` | 当前已定义但仅抑制 diff 消息，应扩展为抑制所有非错误输出 |
| Open Dir 模式实现 | `commands/open.rs` | 递归扫描目录，存储 `RawPaths`，供 Location Path 模式使用 |
| Location Path 模式实现 | `commands/location.rs` | 在 Open Dir 的 `RawPaths` 中指定文件内执行 Normal 匹配 |
| Bash/Exec 超时机制 | `commands/bash.rs`, `commands/exec.rs` | 当前无超时控制，长时间命令会永久阻塞 |
| Read 高亮稳定性 | `commands/read.rs` | `syntect` 初始化线程安全（当前用 `OnceLock` 但首次加载有并发竞争风险） |

---

## 当前架构速览

```
ncs/src/
├── main.rs             CLI 入口 (clap)                           (~140 行)
├── lib.rs              库入口
├── lexer.rs            词法 → Token 流                           (~1000 行)
├── parser.rs           Token → Command AST + convert()           (~1510 行)
├── engine/
│   ├── mod.rs          状态机 + execute_core/out + 输出管理       (~2170 行)
│   └── executor.rs     纯函数辅助                                (~570 行)
├── commands/
│   ├── mod.rs          命令模块入口
│   ├── open.rs         !@Open                                    (~217 行)
│   ├── location.rs     !@Location                                (~224 行)
│   ├── new.rs          !@New                                     (~343 行)
│   ├── delete.rs       !@Delete                                  (~651 行)
│   ├── raw.rs          !@Raw                                     (~50 行)
│   ├── bash.rs         !@Bash                                    (~90 行)
│   ├── exec.rs         !@Exec                                    (~35 行)
│   ├── read.rs         !@Read                                    (~135 行)
│   ├── write.rs        !@Write                                   (~35 行)
│   ├── include.rs      !@Include                                 (~155 行)
│   └── work_path.rs    !@WorkPath                                (~30 行)
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

**测试总计**：662（n_edit 297 + NCS lib 298 + NCS main 4 + integration 63）

**验证命令**：
```bash
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --check
```
