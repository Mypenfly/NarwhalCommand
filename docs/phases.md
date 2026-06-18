# NCS 实现阶段拆分

## Phase 0: NCS 项目骨架搭建 ✅ 已完成

**目标**：可编译、可测试的最小 Rust 项目。

### 任务清单

- [x] 在本仓库中建立 workspace，新建 `ncs` crate，同时保留原有 `n_edit` crate
- [x] 引入依赖：`clap`（CLI）、`colored`（终端颜色）
- [x] 入口 `main.rs`：接收 `.ncs` 脚本路径参数，校验后缀，读入并打印
- [x] 搭目录骨架（13 个源文件 + commands/ 目录 + tests/ 目录）
- [x] `error.rs` 建立根错误类型 `NcsError`（含 6 个子错误枚举，均有 title/detail/hints 方法）
- [x] `model.rs` 建立 `LineNumber` newtype、`Line`、`FileContent`、`ContentBlock`、`LocationContent`、`NewContent`、`DeleteContent`、`SearchScope`、`LineRange` 等结构（从 n_edit 直接迁移，约 90% 复用）
- [x] `cmd_content.rs` 建立 `CmdContent`、`CmdLine`、`CommandResult` 结构
- [x] `registry.rs` 建立 `CommandRegistry`、`CommandEntry`、`CommandType`、`PermissionType`、`ExecutionType`、`ModeEntry`、`ParamDef`、`ParamType` 及 12 个内置命令的完整初始化代码
- [x] 单元测试：model (19)、error (12)、cmd_content (10)、registry (21)、main (3) — 共 67 个
- [x] 集成测试：4 个基础验证用例

### 交付文件清单

| 文件 | 状态 | 行数 | 说明 |
|------|:----:|------|------|
| `ncs/src/main.rs` | 完整 | ~85 | CLI 入口，`.ncs` 后缀校验，文件读取 |
| `ncs/src/lib.rs` | 完整 | ~15 | 库入口，导出 12 个模块 |
| `ncs/src/model.rs` | 完整 | ~530 | 从 n_edit 迁移的核心数据结构 + 单元测试 |
| `ncs/src/error.rs` | 完整 | ~780 | NcsError + 6 个子错误枚举 + 单元测试 |
| `ncs/src/cmd_content.rs` | 完整 | ~200 | 命令间数据传递 + 单元测试 |
| `ncs/src/registry.rs` | 完整 | ~810 | 命令注册表 + 12 内置命令初始化 + 单元测试 |
| `ncs/src/lexer.rs` | 完整 | ~750 | Phase 1: Token 流生成 + 块内容提取 |
| `ncs/src/parser.rs` | 完整 | ~1190 | Phase 1: Command AST 构建 + 内容解析 |
| `ncs/src/engine.rs` | Stub | ~80 | Phase 2 实现（含 exec_cmds/pools 结构） |
| `ncs/src/matcher.rs` | 完整 | ~660 | Phase 2: Location 匹配算法（19 tests） |
| `ncs/src/block.rs` | 完整 | ~640 | Phase 2: Block 解析器（19 tests） |
| `ncs/src/output.rs` | 完整 | ~410 | Phase 2: 终端输出格式化（10 tests） |
| `ncs/src/file_io.rs` | Stub | ~15 | Phase 2 实现 |
| `ncs/src/commands/mod.rs` | Stub | ~15 | 命令模块入口（待实现各命令文件） |
| `ncs/src/engine/` | 待建 | — | Phase 2: executor.rs 等引擎辅助模块 |
| `ncs/tests/integration_test.rs` | 基础 | ~55 | 4 个基础验证用例 |

### 验证结果

```bash
cargo build              ✅
cargo test --workspace   ✅ (166 ncs + 49 n_edit + 4 integration)
cargo clippy -- -D warnings  ✅
cargo fmt --check        ✅
cargo run -- file.ncs    ✅ 加载成功
cargo run -- file.txt    ✅ 正确拒绝非 .ncs 后缀
```

---

### Phase 1 实现路径提示

**核心任务**：实现 `!@Cmd` 语法的 Lexer 和 Parser。

**关键参考**：

| 内容 | 文档 |
|------|------|
| 块提取规则（终止条件） | ncs_dev.md §2.3 |
| 词法分析 Token 枚举 | ncs_dev.md §4.1, 已定义在 `lexer.rs:Token` |
| 语法分析流程 | ncs_dev.md §4.2 |
| 命令定义（模式/参数） | ncs_dev.md §5.1–§5.12 |
| 参数类型 | ncs_dev.md §3.1 (ParamType) |
| 命令 AST | 已定义在 `parser.rs:Command` |

**实现要点**：

1. **Lexer 核心**（`lexer.rs`）：
   - 逐行扫描 `!@` 开头的行 → 按空格切分为 `[Cmd, pre_mode, args...]`
   - 在 `CommandRegistry` 中查找 Cmd → 获得执行类型（行/块/仅展开）
   - 块执行命令 → 调用内容提取，终止于下一个非仅展开 `!@Cmd` 或 `@/Cmd`
   - `!@Raw` / `!@Get` 为仅展开 → 融入父命令内容而非触发终止
   - 识别 `@/Cmd` 和 `@/Cmd | Capture pool_name`
   - Token 携带行号供错误定位

2. **Parser 核心**（`parser.rs`）：
   - 消费 Token 流，在 `CommandRegistry` 中查找命令定义
   - 模式匹配：`pre_mode` 在命令的模式表中查找 → 精确匹配 / 回退为 Normal 模式参数 / ModeNotFound 错误
   - 参数解析：按 `ModeEntry.params` 校验必填参数 → 缺失报 `ParamMissing`
   - 构建对应的 `Command` AST 节点
   - `!@Raw` Token 融入上一个 `New`/`Delete` 的 ContentLines，标记 `is_raw`

3. **边界处理**：
   - 空命令脚本 → 报错
   - `!@Write Raw` 模式 → 从下一行到 EOF 全部原样提取，不做任何解析
   - `@/Cmd` 后无内容 → 视为有效关闭

**从 n_edit 可复用**：
- `lexer.rs` 中仅块内容提取辅助函数（约 20% 可复用，主要是 `extract_command_content` 思路）
- `parser.rs` 中仅 Location/New/Delete 内容解析（`build_location_content` 等，约 30% 可复用）

**验证命令**：
```bash
cargo test -p ncs -- lexer
cargo test -p ncs -- parser
cargo run -p ncs -- tests/scripts/minimal.ncs --verbose
```

## Phase 1: Lexer + Parser — 词法和语法分析 ✅ 已完成

**目标**：实现新的 `!@Cmd` 语法识别和命令解析。

### 1.1 Lexer — 词法分析

- [x] 识别 `!@Cmd` 标识符前缀
- [x] 识别命令名、模式、参数：
  - 按空格分割 → `[Cmd, pre_mode, args...]`
  - `pre_mode` 尝试在 CommandRegistry 中查找对应的命令模式注册表
- [x] 区分行执行和块执行命令：
  - 行执行 → 不提取后续行
  - 块执行 → 提取后续行直到终止条件（下一个非仅展开命令 / `@/Cmd`）
- [x] 识别关闭符号 `@/Cmd`
- [x] 识别 Capture 指令：`@/Open | Capture pool_name`
- [x] 输出 Token 枚举：`Command` / `Close` / `Capture`
- [x] Token 含行号信息，供错误定位

### 1.2 Parser — 语法分析

- [x] 消费 Token 流，在 CommandRegistry 中查找命令定义
- [x] 根据命令的模式注册表匹配模式，解析 args
- [x] 缺失必要参数 → 报 `ParamMissing` 错误
- [x] 多余参数 → 警告但继续执行
- [x] 构建 Command AST（见 INSTRUCTION.md §2.4）
- [x] 校验 `!@Write Raw` 作为特殊模式
- [x] `!@Raw` → 融入上一个 New/Delete 的 ContentLines，标记 `is_raw`

**超范围交付**：Location/New/Delete 的内容解析（`parse_location_content` 等）已实现在 parser.rs 中。

### 验证结果

```bash
cargo test -p ncs -- lexer   ✅ 22 tests
cargo test -p ncs -- parser  ✅ 29 tests
cargo clippy -- -D warnings  ✅
cargo fmt --check            ✅
```

### 交付文件清单（Phase 1 完成后更新）

| 文件 | 状态 | 行数 | 测试数 | 说明 |
|------|:----:|------|:------:|------|
| `ncs/src/lexer.rs` | 完整 | ~750 | 22 | Token 流生成 + 块内容提取 |
| `ncs/src/parser.rs` | 完整 | ~1190 | 29 | Command AST 构建 + 内容解析 |
| `ncs/src/matcher.rs` | 完整 | ~660 | 19 | Location 匹配算法 |
| `ncs/src/block.rs` | 完整 | ~640 | 19 | Block 解析器 |
| `ncs/src/output.rs` | 完整 | ~410 | 10 | 终端输出格式化 |
| `ncs/src/engine/executor.rs` | 完整 | ~570 | 14 | 引擎纯函数辅助 |
| `ncs/src/engine/mod.rs` | 基础 | ~90 | — | Engine 结构体 |
| `ncs/src/commands/` | 待实现 | — | — | 各命令执行文件 |

**全量测试**: 173 lib + 3 main + 4 integration = 180 tests passed

### 文档参考

| 内容 | 文档位置 |
|------|----------|
| 执行流 | ncs_dev.md §2.1–§2.2 |
| 块提取规则 | ncs_dev.md §2.3 |
| 词法分析 | ncs_dev.md §4.1 |
| 语法分析 | ncs_dev.md §4.2 |
| 命令定义 | ncs_dev.md §5 |
| 参数类型 | ncs_dev.md §3.1 (ParamType) |
| Lexer 实现逻辑 | INSTRUCTION.md §7.2 |

### 验证

```bash
cargo test lexer
cargo test parser
cargo run -- tests/scripts/minimal.ncs   # 基本 Token 识别
```

---

## Phase 2: 迁移 n_edit 核心命令（Open / Location / New / Delete / Raw） ✅ 已完成

**目标**：将 n_edit 已有的稳定命令逻辑迁移到 NCS 框架。

### 2.1 从 n_edit 直接迁移的模块 ✅ 已完成

- [x] `model.rs` — `FileContent`、`ContentBlock`、`LocationContent`、`NewContent`、`DeleteContent`、`LineRange`（Phase 0 已迁移）
- [x] `block.rs` — Block 解析器（BraceScanner, parse_brace_block, parse_indent_block, detect_language）— 19 tests
- [x] `matcher.rs` — Location 匹配算法（SearchScope, rows_match, expect_single_match）— 19 tests
- [x] `output.rs` — DiffLine, OutputFormatter, format_error — 10 tests

### 2.2 引擎拆分重构迁移 (进行中)

**n_edit 的 engine.rs 达 3083 行，严重超出项目 1200 行限制。迁移时按职责拆分为：**

| 文件 | 职责 | 预估行数 | 状态 |
|------|------|----------|:----:|
| `engine/mod.rs` | Engine 结构体、状态管理（block_stack/file/exec_cmds/pools）、命令分发路由、生命周期（exec_cmds 加入/退出、隐式关闭） | ~300 | 🟡 结构体就绪 |
| `engine/executor.rs` | 引擎辅助方法：get_search_scope, apply_block_to_file, reindex_file, build_new_lines, diff 收集、delete 匹配等共享逻辑 | ~570 | ✅ 14 tests |
| `commands/open.rs` | Open 命令：Normal（start/end）和 Dir（depth/ignore/filter）模式 | ~250 | ⏳ |
| `commands/location.rs` | Location 命令：Normal/Block/Path 模式，调用 matcher/block，嵌套支持 | ~300 | ⏳ |
| `commands/new.rs` | New 命令：Normal/Start/End 模式，缩进计算、reindex、流输出同步 | ~200 | ⏳ |
| `commands/delete.rs` | Delete 命令：Normal/Block 模式，邻接检查、连续匹配、reindex、流输出同步 | ~200 | ⏳ |
| `commands/raw.rs` | Raw 命令：仅展开，融入父命令内容 | ~60 | ⏳ |

**核心原则**：Location/New/Delete 的关键算法逻辑**不变动**，确保 n_edit/tests/scripts/ 的原有 .ned 脚本操作结果仍然可行且可靠。

**n_edit engine.rs 核心逻辑到 NCS 的映射**：

| n_edit 方法 | NCS 位置 | 说明 |
|------------|----------|------|
| `execute_open()` | `commands/open.rs` | 新增 Dir 模式、start/end 参数 |
| `execute_location()` | `commands/location.rs` | 新增 Path 模式、移除行号定位 |
| `execute_new()` | `commands/new.rs` | Normal/Start/End 模式，保留缩进逻辑 |
| `execute_delete()` | `commands/delete.rs` | Normal/Block 模式，保留邻接/连续检查 |
| `execute_off()` / `handle_implicit_off()` | `engine.rs` | 适配 `Close { name }` 和 exec_cmds 退出 |
| `get_search_scope()`, `apply_block_to_file()`, `reindex_file()`, `build_new_lines()` | `engine/executor.rs` | 共享辅助函数 |
| `find_delete_match()`, `check_delete_adjacency()`, `lines_continuously_match()` | `engine/executor.rs` | Delete 专用辅助 |
| `collect_block_context_above/below()`, `record_diff_with_context()`, `insert_separator_if_needed()` | `engine/executor.rs` | Diff 收集辅助 |

### 2.3 Open 命令增强

- [ ] `Normal` 模式：支持 `start` / `end` 参数限定读取范围
- [ ] `Dir` 模式：
  - 递归扫描目录，支持 `depth` / `ignore` / `filter` 参数
  - 得到 `RawPaths` 列表

### 2.4 Location 命令改造

- [ ] 移除行号定位（`@66,120`）——已被 `start`/`end` + 嵌套 Location 替代
- [ ] `Path` 模式：在 `RawPaths` 中的指定文件内执行 Normal 匹配
- [ ] Dir 模式下的 Location：遍历 `RawPaths` 中所有文本文件执行匹配

### 2.5 New / Delete / Raw

- [ ] New：Normal / Start / End 模式，保留 n_edit 的缩进计算和 reindex 逻辑
- [ ] Delete：Normal / Block 模式，保留 n_edit 的邻接检查和连续匹配
- [ ] Raw：仅展开融入父命令内容

### 2.6 流输出

- [ ] Location 的 `@/Location` 触发打印 `LocationResult`（带文件路径和行号，灰色内容）
- [ ] New 执行后同步修改 `LocationResult`（新增行 `+` 绿色）
- [ ] Delete 执行后同步修改 `LocationResult`（删除行 `-` 红色）

### 当前进度

| 步骤 | 状态 | 测试数 |
|------|:----:|:------:|
| 2.1 直接迁移（model/block/matcher/output） | ✅ | 48 tests |
| 2.2 引擎拆分重构 | ✅ | — |
| engine/executor.rs | ✅ | 14 tests |
| engine/mod.rs | ✅ | 命令分发 + 生命周期 |
| commands/raw.rs | ✅ | 2 tests |
| commands/open.rs | ✅ | 5 tests |
| commands/location.rs | ✅ | 5 tests |
| commands/new.rs | ✅ | 5 tests |
| commands/delete.rs | ✅ | 5 tests |
| 测试迁移（40 .ned → .ncs + 集成测试） | ✅ | 30 tests |

**全量验证**: 195 lib + 3 main + 30 integration = **228 tests**, clippy 0 warnings, fmt clean

### 引擎拆分结果

| 文件 | 行数 | 说明 |
|------|------|------|
| `engine/mod.rs` | ~280 | Engine 结构体、状态管理、命令分发路由、exec_cmds/block_stack 生命周期 |
| `engine/executor.rs` | ~570 | 纯函数：delete 匹配、文件/Block 写回、diff 收集、reindex |
| `commands/raw.rs` | ~60 | Raw 仅展开命令 |
| `commands/open.rs` | ~160 | Open Normal 模式（含 start/end） |
| `commands/location.rs` | ~210 | Location Normal/Block 模式 + 嵌套支持 |
| `commands/new.rs` | ~230 | New Normal/Start/End 模式 |
| `commands/delete.rs` | ~240 | Delete Normal/Block 模式 |

**全部文件行数均 ≤ 600，符合项目规范（≤1200，建议 ≤800）**

### 测试迁移结果

| 类别 | 数量 | 说明 |
|------|:----:|------|
| .ned → .ncs 脚本 | 40 | 通过 Python 转换器自动转换 |
| 集成测试 | 30 | 从 n_edit 端到端测试迁移，覆盖 Phase 1-3 |
| 测试数据文件 | 11 | 直接复制，包含 Rust/Python/YAML/Markdown/TXT |
| 已移入待实现 | 1 | multi_op_refactor（依赖 Phase 5 行号 Delete） |

### 核心验证结果

n_edit 的关键算法逻辑在 NCS 中保持一致：
- **Location 匹配**: find_unique_block, rows_match, stripped_content, diff_taps — 不变
- **Block 解析**: BraceScanner, detect_language, parse_brace_block, parse_indent_block — 不变
- **New 插入**: build_new_lines, reindex, is_raw 处理 — 不变
- **Delete 操作**: find_delete_match, check_delete_adjacency, lines_continuously_match — 不变
- **文件写回**: apply_block_to_file, apply_block_to_parent, write_back_to_file — 不变
- **Diff 输出**: DiffLine, format_diff_lines, format_error — 不变

### 验证策略

**核心验证**：n_edit/tests/scripts/ 下的 .ned 脚本通过语法转换后，在 NCS 中执行结果**必须与 n_edit 一致**。

转换规则：
| n_edit | NCS |
|--------|-----|
| `//!@Open: path` | `!@Open path` |
| `//!@Location:` | `!@Location` |
| `//!@New:` | `!@New` |
| `//!@Delete:` | `!@Delete` |
| `//!@Raw:` | `!@Raw` |
| `...` 分隔符 | 自动块提取（无显式分隔符） |
| `//!@Off:Open` | `@/Open` |
| `//!@Off:Location` | `@/Location` |

转换后的 .ncs 脚本放入 `ncs/tests/scripts/`，在集成测试中端到端验证。

### 文档参考

| 内容 | 文档位置 |
|------|----------|
| Location 匹配算法 | INSTRUCTION.md §3.1, n_edit_dev.md Location 章节 |
| Block 解析算法 | INSTRUCTION.md §3.2, n_edit_dev.md Location:Block 章节 |
| New 插入算法 | INSTRUCTION.md §3.3, n_edit_dev.md New 章节 |
| Delete 操作算法 | INSTRUCTION.md §3.4, n_edit_dev.md Delete 章节 |
| 重难点 | INSTRUCTION.md §4.1–§4.2 |
| exec_cmds 管理 | ncs_dev.md §3.2, §6.3 |
| Open 命令 | ncs_dev.md §5.1 |
| Location 命令 | ncs_dev.md §5.2 |
| New 命令 | ncs_dev.md §5.3 |
| Delete 命令 | ncs_dev.md §5.4 |
| Raw 命令 | ncs_dev.md §5.5 |

### 验证

```bash
cargo test matcher
cargo test block
cargo test engine
cargo test --test integration_test   # 迁移 n_edit 的测试脚本，适配新语法
```

---

## Phase 3: CmdContent 数据流重构（变更追踪模型） 🟢 核心功能已交付

**目标**：将所有命令的数据流动统一为 CmdContent + ContentChange 变更追踪模型，实现延迟应用、可追踪修改，从根本上解决 BUG-204（New/Delete 执行顺序冲突）。

**状态（2026-06-18）**：Delete 快照匹配 + New 变更记录 + 流输出控制 + Location 终端输出已交付。桥接架构（cmd 层双写 Block + CmdContent）稳定运行。待后续版本彻底化：命令签名改为返回 CommandResult + handle_close 激活延迟应用。

### 核心改造

将现有的"命令直接操作 `&mut ContentBlock`"模型替换为：

```
命令操作 → 追加 ContentChange 记录 → Owner 关闭时 apply_changes() 统一生效
```

### 3.0 CmdContent 扩展 + 变更追踪基础 ✅ 已完成

- [x] `ContentChange` 枚举（Insert / Delete + source_cmd）— `cmd_content.rs:66-87`
- [x] `ContentSource` 枚举（Block / File / CommandOutput）— `cmd_content.rs:96-105`
- [x] CmdContent 新增字段：`snapshot_lines` / `snapshot_raw` / `changes` / `source_info` — `cmd_content.rs:27-41`
- [x] CmdContent 新增方法：`record_insert()` / `record_delete()` / `apply_changes()` — `cmd_content.rs:149-193`
- [x] `CommandResult.content` 承载 CmdContent 传递 — `cmd_content.rs:231-234`
- [x] `CommandResult` 派生 `Clone` — `cmd_content.rs:231`
- [x] file_io.rs 补全（`read_file`, `write_file`, `path_exists`）— BUG-501 ✅

### 3.1 Engine 管道重写 ✅ 桥接实现

- [x] `Engine.last_result: Option<CommandResult>` — 追踪上一个命令的输出 — `engine/mod.rs:78`
- [x] `execute()` 重写：统一分发 `dispatch_command()` + `update_last_result()` — `engine/mod.rs:118-153`
- [x] `apply_content_to_file()` — 将 CmdContent.changes 写到 ContentBlock/FileContent（桥接阶段注释调用）— `engine/mod.rs:438`
- [x] `handle_close()` 整合 Capture 管道语法 — `engine/mod.rs`
- [x] 流/值输出接入 `CommandType.output`（BUG-103）— `engine/mod.rs:205-212`
- [x] `print_location_result()` + `--verbose`（BUG-403）— `engine/mod.rs:469-485`

### 3.2 命令方法骨架 ✅ 已完成

- [x] `Command::{cmd_name, mode_name}` 方法 — `parser.rs:121-146`
- [x] `Command::Close.capture` 字段 — `parser.rs:113`
- [x] `Token::Close.capture` 字段 + `parse_close_with_capture()` — `lexer.rs:44-50`
- [x] New: execute_core — `record_insert()` + 直接 Block 插入（双写桥接）— `commands/new.rs:108-120`
- [x] Delete: execute_core — 在 snapshot 匹配 + `record_delete()` + 直接 Block 删除（双写桥接）— `commands/delete.rs:91-155`

### 3.3 Pools & Capture (BUG-104/303) ✅ 已完成

- [x] Lexer: `Token::Close { capture: Option<String> }` — `lexer.rs:44-50`
- [x] Parser: `Command::Close { capture: Option<String> }` — `parser.rs:113`
- [x] Engine: `dispatch_command()` → `Command::Capture` 从 `last_result.take()` 取值 — `engine/mod.rs:171-177`
- [x] Engine: `handle_close()` → `@/Cmd | Capture` 存入 pools — `engine/mod.rs`
- [x] `!@Get` 基本实现（pools 读取 → `last_result`）— `engine/mod.rs:178-193`
- [ ] `!@Get` 块内展开（在 New/Delete 块中展开为 raw_content）— `parser.rs`
- [ ] Get `like=[!@Cmd]` 伪装模式 — Phase 5

### 3.4 Location 终端输出 (BUG-403) ✅ 已完成

- [x] `engine.print_location_result()` — `@/Location` 时输出带行号的匹配块（灰色）
- [x] `--verbose` 标志 CLI 暴露

---

### 🟢 当前部署状态

桥接架构稳定运行：cmd 层双写（直接修改 Block + CmdContent 变更记录），603 tests 通过。核心 BUG-204 已通过 snapshot_lines 匹配根本解决。

### 🔵 下一阶段方向（Phase 3 彻底化）

当前 cmd 层同时写 Block 和 CmdContent（双写）。下一步移除 cmd 层直接 Block 操作，统一到 handle_close 延迟应用：

| 任务 | 涉及文件 | 说明 |
|------|----------|------|
| 命令签名改为返回 CommandResult | `commands/*.rs` | `execute()` → `Result<CommandResult, E>` 替代 `Result<(), E>` |
| Open/Location 正式 convert/out | `commands/open.rs`, `location.rs` | 命令自身构建 CmdContent，移除 update_last_result 桥接 |
| handle_close 激活延迟应用 | `engine/mod.rs` | 打开 `apply_content_to_file()` 注释，移除 cmd 层 block.lines 直接操作 |
| Get 块内展开 + like 模式 | `parser.rs`, `engine/mod.rs` | `!@Get` 在块中展开为 raw_content

### 参考文档

| 内容 | 文档位置 |
|------|----------|
| 变更追踪模型 | ncs_dev.md §3.3 |
| New 执行流（新） | ncs_dev.md §5.3 |
| Delete 执行流（新） | ncs_dev.md §5.4 |
| 数据传递路径（新） | ncs_dev.md §6.4, §6.5 |
| 详细设计 | `docs/superpowers/specs/2026-06-18-phase3-cmdcontent-pipeline-design.md` |

### 验证

```bash
cargo test -p ncs -- engine
cargo test -p ncs -- commands
cargo test --test integration_test   # 全量回归 + BUG-204 严格断言
cargo clippy --workspace -- -D warnings
cargo fmt --check
```

---

## Phase 4: 新增独立命令（Bash / Exec / Read / Write / Include / WorkPath） ⏳

**目标**：实现 NCS 的新独立命令，在 CmdContent 管道就绪后接入。

### 实现顺序：
1. **Write**（最独立）— Normal/Raw 模式，不依赖其他命令
2. **Read** — 复用 Open 的文件读取逻辑，值输出
3. **Bash** — bash -c 执行 + 安全审查
4. **Exec** — script -c 直连终端执行
5. **Include** — 命令注册表动态扩展
6. **WorkPath** — 工作路径管理

### 4.1 Bash

- [ ] 行执行：提取命令字符串
- [ ] 通过 `bash -c` 执行
- [ ] 捕获 stdout/stderr 为 `CmdContent`
- [ ] 终端打印结果
- [ ] 安全审查：拦截 `sudo`、`rm -rf /`、`chmod 777 /` 等高危命令

### 4.2 Exec

- [ ] 行执行：提取命令字符串
- [ ] 通过 `script -c` 执行（直连终端，支持彩色/流式/交互式）
- [ ] 值输出：仅打印，结果不保留

### 4.3 Read

- [ ] 模式和参数与 `!@Open` 完全一致
- [ ] 值输出：结果不保留
- [ ] 输出带高亮和行号的文件内容
- [ ] 输出带高亮和树状结构的路径

### 4.4 Write

- [ ] `Normal` 模式：块内容写入文件
- [ ] `Raw` 模式：从下一行到 EOF 的全部内容原样收集，写入后程序退出

### 4.5 Include

- [ ] 解析 Include 参数（alias, block, type, exec, owners, subs）
- [ ] 校验 alias 不与内置命令重名
- [ ] 将外部命令注册到 CommandRegistry

### 4.6 WorkPath

- [ ] 验证路径存在
- [ ] 更改工作路径

### 文档参考

| 内容 | 文档位置 |
|------|----------|
| Bash | ncs_dev.md §5.6 |
| Exec | ncs_dev.md §5.7 |
| Read | ncs_dev.md §5.8 |
| Write | ncs_dev.md §5.9 |
| Include | ncs_dev.md §5.10 |
| WorkPath | ncs_dev.md §5.11 |

### 验证

```bash
cargo test commands::bash
cargo test commands::exec
cargo test commands::read
cargo test commands::write
cargo test commands::include
cargo test commands::work_path
cargo test registry
```

---

## Phase 5: Get 高级特性 + like 伪装

**目标**：实现 Get 的 like 伪装模式和高级数据传递。

### 5.1 Get like 模式

- [ ] 指定 `like` 选项：在 `exec_cmds` 中写入伪装的命令条目
- [ ] 后续命令可在 `exec_cmds` 中找到 owner
- [ ] 遇到对应的 `@/Cmd` 时执行正常关闭逻辑
- [ ] 支持 `{}` 占位符替换

### 5.2 CmdContent 完善

- [ ] `send()` 方法：序列化为最原始字符串（外部命令调用用）
- [ ] `print()` 方法：输出 `result` 字段内容到终端
- [ ] pools 键名冲突处理

### 文档参考

| 内容 | 文档位置 |
|------|----------|
| Get 命令 | ncs_dev.md §5.12 |
| 数据传递路径 | ncs_dev.md §6.4 |
| CmdContent | ncs_dev.md §3.3 |
| pools | ncs_dev.md §3.4 |

### 验证

```bash
cargo test cmd_content
cargo test commands::get
```

---

## Phase 6: 错误处理 + 终端输出优化

**目标**：用户体验完好的错误提示和终端输出。

### 6.1 错误处理扩展

- [ ] `RegistryError`：CommandNotFound（含字符相似度提示）/ ModeNotFound / OwnerNotExecuted / AliasConflict
- [ ] `CommandExecError`：ExecutionFailed / SecurityDenied / Timeout / IncludeFailed
- [ ] 完善 `NcsError` 统一分发：`title()` / `detail()` / `hints()`
- [ ] 错误输出格式统一（INSTRUCTION.md §5.2）

### 6.2 终端输出

- [ ] `output.rs`：封装 `colored` 库
  - 绿色 `+` 新增行，红色 `-` 删除行
  - `Error:` 红色加粗，标题黄色，详情灰色，`Hint:` 绿色加粗
- [ ] 检测 `is_terminal`，管道/重定向时关闭颜色
- [ ] ContextBlock 变更时自动插入 `~~~~~~~~` 分隔符

### 6.3 日志/详细模式

- [ ] `--verbose` 标志：打印每条命令的执行详情
- [ ] `--quiet` 标志：抑制成功消息和 diff 输出，只输出错误

### 文档参考

| 内容 | 文档位置 |
|------|----------|
| 错误体系 | ncs_dev.md §7 |
| 错误格式 | INSTRUCTION.md §5.2 |
| 终端输出 | INSTRUCTION.md §5.3 |

### 验证

```bash
cargo test error
cargo test output
cargo run -- tests/scripts/error_cases.ncs --verbose
cargo run -- tests/scripts/error_cases.ncs --quiet
cargo clippy -- -D warnings
cargo fmt --check
```

---

## 阶段依赖关系

```
Phase 0 ──▶ Phase 1 ──▶ Phase 2 ──▶ Phase 3 ──▶ Phase 4 ──▶ Phase 5
                                     │             │             │
                                     │  (CmdContent  (独立命令)    (Get高级)
                                     │   管道重构)                  │
                                     │                           │
                                     └───────────┬───────────────┘
                                                 │
                                                 ▼
                                            Phase 6
                                           (错误+输出打磨,
                                            可与 4/5 并行)
```

- **Phase 0 → Phase 1**：骨架是词法/语法分析的基础
- **Phase 1 → Phase 2**：Parser 出 AST 后，Engine 才能执行
- **Phase 2 → Phase 3**：核心命令稳定后，进行 CmdContent 数据流重构
- **Phase 3 → Phase 4**：CmdContent 管道就绪后，新独立命令接入
- **Phase 4 → Phase 5**：命令齐全后实现 Get 高级特性
- **Phase 6**：可与 Phase 4/5 并行，持续打磨

---

## Phase 0–2 补充：从 n_edit 可复用的资产

以下 n_edit 的源码文件和测试可以**直接迁移**或**少量适配后迁移**：

| n_edit 文件 | NCS 对应文件 | 复用程度 |
|-------------|-------------|----------|
| `model.rs` | `model.rs` | 90% — 新增 `CmdContent` 相关的字段 |
| `matcher.rs` | `matcher.rs` | 95% — 搜索范围接口不变 |
| `block.rs` | `block.rs` | 100% — 完全复用 |
| `error.rs` | `error.rs` | 70% — 保留 Match/File/Engine，新增 Registry/CommandExec |
| `output.rs` | `output.rs` | 95% — 几乎不变 |
| `engine.rs` | `engine.rs` | 50% — 执行逻辑保留，包裹在 CmdContent convert/out 中 |
| `parser.rs` | `parser.rs` | 30% — 语法完全改变，仅 Location/New/Delete 内容解析逻辑保留 |
| `lexer.rs` | `lexer.rs` | 20% — 语法完全改变，仅块内容提取辅助函数可用 |

n_edit 的 297 个测试中，matcher、block、model、output 的单元测试可以直接迁移。
