# NCS Bug 清单与修复记录

> **修复进度**: 第一阶段 5/5 ✅ | 第二阶段 6/6 ✅ | 第三阶段 4/8 ✅（桥接实现），核心命令迁移待完成 | 尚余 11 项

---

## 修复历史

### 第一阶段（阻断修复）— 全部完成

| Bug | 描述 | 修复方式 | 新增测试 |
|-----|------|----------|----------|
| BUG-01 | main.rs 未调用引擎 | `main.rs:106`: `engine.execute(commands)` 替代仅打印 | 1 |
| BUG-02 | Diff 行未输出到终端 | `main.rs:108-113`: `OutputFormatter::format_diff_lines()` | (合并) |
| BUG-04 | pop_exec_cmd truncate 误清 | `engine/mod.rs`: `truncate(pos)` → `remove(pos)` | 4 |
| BUG-07 | Open Dir stub 返回误导错误 | `error.rs`: 新增 `EngineError::NotImplemented { feature }` | 1 |
| BUG-08 | Location Path stub 返回误导错误 | `location.rs:37`: 改用 `NotImplemented` | (合并) |
| BUG-12 | 未实现命令静默跳过 | `engine/mod.rs:158-206`: 每个返回 `NotImplemented` 错误 | 1 |

### 第二阶段（严重修复）— 全部完成

| Bug | 描述 | 修复方式 | 新增测试 |
|-----|------|----------|----------|
| BUG-301 | @/ 块终止不校验命令名 | `lexer.rs`: `extract_block_content` 校验命令名，Open/Location/Off 始终终止 | 5 |
| BUG-201 | exec_cmds owner 未检查 | `engine/mod.rs`: `check_owner()` + `execute()` 接受 `&CommandRegistry` | 4 |
| BUG-202 | exec_cmds 退出仅移除单个 | `engine/mod.rs`: `pop_exec_cmd` 改为 `truncate(pos)` 范围移除 | 2 |
| BUG-401 | New(Start/End) 可在 block 内操作 | `commands/new.rs`: `execute_start/end` 移除 block_stack 分支 | 1 |
| BUG-402 | Delete:Block 不校验 Location:Block | `commands/delete.rs`: `execute_block` 检查 exec_cmds 中 Location 模式 | 3 |
| BUG-302 | Capture Token 被 Parser 丢弃 | `parser.rs`: 新增 `Command::Capture`，`engine/mod.rs`: 写入 pools | 1 |

### 第三阶段（变更追踪模型）— 桥接实现完成，核心迁移就绪

| Bug | 描述 | 修复方式 | 新增测试 |
|-----|------|----------|----------|
| BUG-501 | file_io.rs stub | `file_io.rs`: `read_file()`, `write_file()`, `path_exists()` 完整实现 | 6 |
| BUG-302 | Capture 存入 pools（增强） | `engine/mod.rs`: Capture 从 `last_result` 取值（替代空 CmdContent） | 1 (更新) |
| BUG-303 | Get 基本展开 | `engine/mod.rs`: `dispatch_command()` 中 Get 从 pools 读取 | 1 |
| BUG-104 | pools 写入 + Capture 管道 | `lexer.rs`: `Token::Close.capture` 字段 + `parse_close_with_capture()`；`engine/mod.rs`: `handle_close()` Capture 存入 pools | 1 (更新) |

### 第三阶段（变更追踪模型 — 本次完成）:::新建变更记录

| Bug | 描述 | 修复方式 | 新增测试 |
|-----|------|----------|----------|
| **BUG-204** | Delete 执行顺序：New 在前污染匹配 | `commands/delete.rs`: `execute_normal()` 在 `last_result.snapshot_lines` 上匹配（而非被 New 修改后的 `block.lines`）；`record_delete()` 记录变更 | 7 |
| **BUG-101** | CmdContent 变更记录层 | `commands/delete.rs`: `record_delete()`；`commands/new.rs`: `record_insert()`；变更数据写入 `last_result.content.changes` | 5 |
| **BUG-403** | Location 关闭时不触发终端输出 | `engine/mod.rs`: `print_location_result()` + `handle_close("LOCATION")` 中调用 + `--verbose` 控制 | 1 |
| **BUG-103** | 流/值输出未接入执行路径 | `engine/mod.rs`: `update_last_result()` 从 `CommandType.output` 读取输出类型，ValueOutput 清除 `last_result`；`registry.rs`: GET 注册为 StreamOutput | 1 |
| BUG-303 | Get 基本展开 | `engine/mod.rs`: `dispatch_command()` 中 Get 从 pools 读取 | 1 |
| BUG-104 | pools 写入 + Capture 管道 | `lexer.rs`: `Token::Close.capture` 字段 + `parse_close_with_capture()`；`engine/mod.rs`: `handle_close()` Capture 存入 pools | 1 (更新) |

**同时完成的架构基础**（不直接对应单个 BUG，为后续迁移铺路）：
- `ContentChange` 枚举（Insert/Delete + source_cmd）→ `cmd_content.rs`
- `ContentSource` 枚举（Block/File/CommandOutput）→ `cmd_content.rs`
- `CmdContent.{snapshot_lines, snapshot_raw, changes, source_info}` 字段
- `CmdContent::{record_insert, record_delete, apply_changes}` 方法
- `Command::{cmd_name, mode_name}` 方法 → `parser.rs`
- `Command::Close.capture` 字段 → `parser.rs`
- `Token::Close.capture` 字段 + `parse_close_with_capture()` → `lexer.rs`
- `Engine.last_result: Option<CommandResult>` 字段 + `update_last_result()` + `dispatch_command()`
- Engine `execute()` 三步流水线重写

### 第三阶段（待完成 — 桥接彻底化）

*当前桥接可行（cmd 层直接修改 Block + CmdContent 变更记录双写），以下为完全切换到延迟应用模型所需的剩余工作：*

| Bug | 当前状态 | 待完成工作 | 代码位置 |
|-----|:--------:|------------|----------|
| **BUG-101** | 🟡 桥接 | `commands/{new,delete,open,location}.rs` 的 `convert()`/`execute_core()`/`out()` 正式实现，由命令自身构建 `CommandResult` 返回 | `ncs/src/commands/*.rs` |
| **BUG-102** | 🟡 桥接 | 命令 `execute()` 签名改为 `-> Result<CommandResult, NcsError>`（当前返回 `Result<(), NcsError>`） | `ncs/src/commands/*.rs` |
| **BUG-103** | 🟡 部分 | `dispatch_command()` 或 `execute()` 中根据 `CommandType.output` 控制 `last_result` 的保留/丢弃（当前在 `update_last_result()` 中处理，缺少 Get/Capture 专有处理） | `ncs/src/engine/mod.rs` |
| **BUG-204** | 🟢 已修复 | `commands/delete.rs:execute_normal()` 在 `snapshot_lines` 上匹配；变更记录到 `CmdContent.changes`；不再受 New 先执行影响 | `ncs/src/commands/delete.rs` |
| **BUG-403** | 🟢 已修复 | `engine/mod.rs`: `print_location_result()` 实现，`handle_close("LOCATION")` 中调用，`--verbose` CLI 标志接入 | `ncs/src/engine/mod.rs` |
| **BUG-203** | ⏳ 未实现 | `is_independent` 字段的实际使用或移除 | `ncs/src/engine/mod.rs` |

### 迁移关键路径

```
✅ Step 1: BUG-204 — Delete 命令搬迁到 CmdContent 快照匹配（已完成）
        ├── 删除 commands/delete.rs 中 execute_normal() 对 block.lines 的依赖
        ├── 改为在 engine.last_result.content.snapshot_lines 上匹配
        ├── 匹配成功 → content.record_delete(start_idx, end_idx, "DELETE")
        └── engine.last_result 的 CmdContent 携带 ContentChange::Delete

✅ Step 2: BUG-101/102 — New 命令搬迁到 CmdContent 变更记录（已完成）
        ├── commands/new.rs execute_normal() 调用 content.record_insert()
        └── New 的变更记录累积在 CmdContent.changes 中

✅ Step 4: BUG-204 — apply_content_to_file 实现（已完成，桥接阶段备用）
        ├── apply_changes() → 转换 ContentChange → ContentBlock 行操作
        ├── reindex() → diff_lines
        └── write_back_to_parent()

✅ Step 5: BUG-103 — 流/值输出接入（已完成）
        ├── update_last_result() 中从 registry cmd_type.output 读取输出类型
        └── ValueOutput 命令的 last_result 清除

✅ Step 6: BUG-403 — Location 终端输出（已完成）
        ├── engine.print_location_result() 实现
        └── --verbose 标志接入

⏳ Step 3: BUG-101/102 — Open/Location 正式 convert/out（待完成）
        ├── Open.convert() / execute_core() / out() 正式实现
        ├── Location.convert() / execute_core() / out() 正式实现
        └── 移除 engine.update_last_result() 中的 OPEN/LOCATION 桥接分支
```

> **关键约束**：迁移过程中保持现有集成测试（49 n_edit + 51 NCS）全部通过。每一步完成后立即回归。

---

## 一、数据流架构缺失（CmdContent / CommandResult / pools）

### BUG-101: CmdContent convert()/out() 模式完全缺失

**状态**: 🟡 桥接实现（架构已就绪，命令层未搬迁）  
**严重程度**: 架构债务  
**对应文档**: ncs_dev.md §3.3、§6.4、INSTRUCTION.md §2.3  
**代码位置**: `ncs/src/cmd_content.rs`、`ncs/src/commands/*.rs`、`ncs/src/engine/mod.rs`

**已完成**:
- `ContentChange` 枚举、`ContentSource` 枚举、`CmdContent` 变更追踪字段全部就绪
- `CmdContent::{record_insert, record_delete, apply_changes}` 方法已实现
- `Command::{cmd_name, mode_name}` 方法已实现
- `Engine.last_result` + `update_last_result()` 在引擎层桥接追踪（后补模式）

**待完成**: 命令层面正式实现 `convert()`/`execute_core()`/`out()`，由命令自身直接操作 CmdContent 而非通过 engine 后补。详见下方"迁移关键路径"。

---

### BUG-102: CommandResult 未被任何命令返回

**状态**: 🟡 桥接实现（Engine 层通过 `update_last_result()` 后补）  
**严重程度**: 架构债务  
**对应文档**: ncs_dev.md §3.3、INSTRUCTION.md §2.3  
**代码位置**: `ncs/src/cmd_content.rs:108-133`、`ncs/src/commands/*.rs`、`ncs/src/engine/mod.rs`

**已完成**: `CommandResult` 通过 `Engine.last_result` 和 `update_last_result()` 在引擎层跟踪。  
**待完成**: 命令 `execute()` 签名改为 `-> Result<CommandResult, NcsError>`，由命令自身构建返回值。

---

### BUG-103: 流输出 / 值输出区分未接入执行路径

**状态**: 🟡 桥接实现（Engine 层 `is_stream` 硬编码，未读 `CommandType.output`）  
**严重程度**: 架构债务  
**对应文档**: ncs_dev.md §6.2、INSTRUCTION.md §1.2  
**代码位置**: `ncs/src/engine/mod.rs:309`（`update_last_result()` 中 `is_stream` 硬编码）  

**当前**: `update_last_result()` 中硬编码 `"OPEN"|"LOCATION"|"BASH"` 为 StreamOutput。  
**待完成**: 从 `registry.find_command(&cmd_name).cmd_type.output` 读取，ValueOutput 命令不保留 `last_result`。

---

### BUG-104: pools 字段存在但从未被写入

**状态**: 🟢 已修复  
**严重程度**: 功能缺口  
**对应文档**: ncs_dev.md §3.4、§6.1  
**代码位置**: `ncs/src/engine/mod.rs:171-177`

**修复细节**:
```rust
// engine/mod.rs — dispatch_command()
Command::Capture { pool_name } => {
    let content = self.last_result.take()
        .map(|r| r.content)
        .unwrap_or_default();
    self.pools.insert(pool_name.clone(), content);
    Ok(())
}
// handle_close() — Close 管道捕获
if let Some(pool_name) = capture {
    if let Some(result) = self.last_result.take() {
        self.pools.insert(pool_name.clone(), result.content);
    }
}
```
- Capture 命令从 `last_result` 取 CmdContent（含 snapshot + changes）
- `@/Cmd | Capture pool_name` 语法通过 `Token::Close.capture` 解析

---

## 二、exec_cmds 生命周期缺陷

### BUG-201: exec_cmds owner 检查从未执行 ✅ 已修复

**严重程度**: 严重  
**对应文档**: ncs_dev.md §6.3 第 2 条、INSTRUCTION.md §1.3 "状态约束"  
**代码位置**: `ncs/src/engine/mod.rs:112-170`、`ncs/src/registry.rs:269-393`

**文档要求**:
> 执行前检查：解析到新命令时，检查其 owner 是否存在于 exec_cmds 中

**修复细节**:

1. `engine.execute()` 签名变更：新增 `registry: &CommandRegistry` 参数（`mod.rs:113`）
2. 新增 `check_owner()` 方法（`mod.rs:224-265`）：
   - 从 `Command` AST 抽取 `(cmd_name, current_mode)` 
   - 查找 `CommandEntry.owners`，按 `allowed_modes` 过滤（匹配当前命令模式）
   - 遍历 exec_cmds，检查任一 owner 是否存在
   - 无匹配 owner → `RegistryError::OwnerNotExecuted`
3. Registry 修正（`registry.rs:354-359`）：New Start/End 增加 `("Open", vec!["Start"/"End"])` 作为 owner
4. 所有调用者更新：`main.rs:106,162`、`integration_test.rs:96` 传入 `&registry`

**新增测试** (4):
- `test_location_without_open_returns_owner_not_executed`
- `test_new_without_location_returns_owner_not_executed`
- `test_delete_without_location_returns_owner_not_executed`
- `test_location_then_new_passes_owner_check`

---

### BUG-202: exec_cmds 退出逻辑 ✅ 已修复

**严重程度**: 严重  
**对应文档**: ncs_dev.md §6.3 第 3 条  
**代码位置**: `ncs/src/engine/mod.rs:341-346`

**文档要求**:
> 退出（@/Cmd）：从末尾向前查找匹配的 Cmd → 清除非独立命令（从该位置到末尾的所有非独立命令一并移除）

**修复细节**:
```rust
// engine/mod.rs:341-346 — 修改后
fn pop_exec_cmd(&mut self, name: &str) {
    let upper = name.to_uppercase();
    if let Some(pos) = self.exec_cmds.iter().rposition(|ec| ec.cmd_name == upper) {
        self.exec_cmds.truncate(pos);  // 从 pos 截断，移除 pos 及之后所有条目
    }
}
```
- 第一阶段：`truncate(pos)` → `remove(pos)`（修复多余删除）
- 第二阶段：`remove(pos)` → `truncate(pos)`（补全范围移除语义）
- 示例：`[OPEN, LOCATION, NEW]` + `@/Location` → `[OPEN]`（NEW 一并清除）

**新增测试** (2):
- `test_pop_exec_cmd_removes_range_on_close`
- `test_pop_exec_cmd_on_last_removes_only_single`

---

### BUG-203: `is_independent` 字段定义但从未被读取

**状态**: ⏳ 未实现  
**严重程度**: 一般  
**对应文档**: ncs_dev.md §3.2（ExecutedCommand 定义）  
**代码位置**: `ncs/src/engine/mod.rs:44`

**实际情况**: `ExecutedCommand.is_independent` 在结构体中定义，全部设为 `false`，无任何读取代码。

---

### BUG-204: Location → Delete → New 顺序执行时 Delete 误读到 New 修改后的 Block

**状态**: 🟢 已修复（2026-06-18，Phase 3 变更追踪模型）  
**严重程度**: 致命  
**对应文档**: ncs_dev.md §5.4（Delete 执行流："在 snapshot_lines 中逐行去空白匹配删除内容"）  
**代码位置**: `ncs/src/commands/delete.rs:91-155`、`ncs/src/engine/executor.rs:162-245`、`ncs/src/engine/mod.rs:295-302`

**根因**: `commands/delete.rs:execute_normal()` 在 `ContentBlock.lines`（可能已被 New 修改）上匹配，而非在 Location 时的原始快照上匹配。

**修复方式**:
1. `executor.rs`: 新增 `find_delete_match_in_snapshot()`、`check_delete_adjacency_in_snapshot()`、`map_snapshot_index_to_block_index()` — 基于 `CmdLine` 快照的匹配函数（与现有 ContentBlock 版本对偶）
2. `cmd_content.rs`: `CmdLine::stripped_content()` — 与 `Line::stripped_content()` 一致的模糊匹配
3. `delete.rs:execute_normal()`: 优先从 `engine.last_result.content.snapshot_lines` 匹配；通过 stripped_content 映射到 `block.lines` 实际位置；调用 `record_delete()` 记录变更
4. `engine/mod.rs:update_last_result()`: DELETE 移除重复匹配（由 delete.rs 直接记录）

**验证**:
- `test_delete_matches_on_snapshot_not_affected_by_prior_new` — New 在前 Delete 在后，Delete 在快照上成功匹配
- `test_delete_matches_on_snapshot_with_multiple_new_before` — 多重 New 不干扰 Delete
- `test_snapshot_not_modified_by_new_before_delete` — 快照不受 New 插入污染（旧版此测试预期失败，现已通过）
- 51 项集成测试全部通过，包括 `rust_edge_cases`、`scenario09_delete_replace`、`multi_op_refactor` 等复杂场景

---

## 三、词法/语法分析缺陷

### BUG-301: `@/` 块终止不校验命令名匹配 ✅ 已修复

**严重程度**: 严重  
**对应文档**: ncs_dev.md §2.3（块执行终止条件第 2 条："对应的 @/Cmd 关闭符号"）  
**代码位置**: `ncs/src/lexer.rs:239-258`

**修复细节**:

1. `extract_block_content()` 新增 `cmd_name: &str` 参数（`lexer.rs:233`）
2. 调用处提取命令名传入（`lexer.rs:103-106`）:
   ```rust
   let cmd_name = match &token {
       Token::Command { name, .. } => name.clone(),
       _ => unreachable!(),
   };
   ```
3. 块终止逻辑改为三级判断（`lexer.rs:247-257`）:
   - `@/Open` / `@/Off` → 始终终止（根关闭符）
   - `@/Location` → 始终终止（块上下文关闭符）
   - 其他 `@/Cmd` → 仅命令名匹配时终止
   - 不匹配的 `@/` → 作为内容行继续提取
4. 三方约定保持向后兼容：现有脚本中 `@/Location` 和 `@/Open` 仍正确关闭所有块

**新增测试** (5):
- `test_block_does_not_terminate_on_non_matching_close`（`@/New` 不终止 `!@Location`）
- `test_block_terminates_on_matching_close`（`@/New` 正确终止 `!@New`）
- `test_block_terminates_on_open_close_even_in_nested_block`（`@/Open` 始终终止）
- `test_location_close_terminates_any_block`（`@/Location` 始终终止）
- `test_delete_close_does_not_terminate_location_block`（`@/Delete` 不终止 `!@Location`）

---

### BUG-302: Capture Token 在 Parser 中被丢弃 ✅ 已修复

**严重程度**: 严重  
**对应文档**: ncs_dev.md §6.1（Capture 指令）、INSTRUCTION.md §6  
**代码位置**: `ncs/src/parser.rs:209-212`、`ncs/src/engine/mod.rs:165-168`

**修复细节**:

1. **Parser** — 新增 `Command::Capture` 变体（`parser.rs:115-118`）:
   ```rust
   Capture {
       pool_name: String,
   },
   ```
2. **Parser** — Token 转换（`parser.rs:209-211`）:
   ```rust
   Token::Capture { pool_name, .. } => {
       commands.push(Command::Capture { pool_name });
   }
   ```
3. **Engine** — 处理 Capture（`engine/mod.rs:165-168`）:
   ```rust
   Command::Capture { pool_name } => {
       let content = CmdContent::default();
       self.pools.insert(pool_name.clone(), content);
   }
   ```

**新增测试** (1):
- `test_capture_command_stores_into_pools`

---

### BUG-303: `!@Get` 行内前缀展开未实现

**状态**: 🟡 基本实现（pools 读取完成，块内展开 + like 未实现）  
**严重程度**: 一般  
**对应文档**: ncs_dev.md §5.12、§2.5  
**代码位置**: `ncs/src/engine/mod.rs:178-193`

**已完成**: `dispatch_command()` 中 Get 从 pools 读取 CmdContent 并设入 `last_result`。  
**待完成**: 块内展开（`!@Get` 在 New/Delete 块中展开为 raw_content）、`like=[!@Cmd]` 伪装模式。

---

## 四、命令实现缺陷

### BUG-401: New(Start) / New(End) 可在 block_stack 非空时错误操作 ✅ 已修复

**严重程度**: 严重  
**对应文档**: ncs_dev.md §5.3（New Start/End owner 为 Open，文件级操作）  
**代码位置**: `ncs/src/commands/new.rs:35-63`

**修复细节**:

1. **`execute_start()`** — 移除 `block_stack.last_mut()` 分支（`new.rs:35-56`）:
   ```rust
   fn execute_start(engine: &mut Engine, content: NewContent) -> Result<(), NcsError> {
       let file = engine.file.as_mut().ok_or(...)?;  // 始终操作 file
       let insert_pos = 0;
       // ... 在文件开头插入 ...
   }
   ```
2. **`execute_end()`** — 移除 `block_stack.last_mut()` 分支（`new.rs:65-84`）:
   ```rust
   fn execute_end(engine: &mut Engine, content: NewContent) -> Result<(), NcsError> {
       let file = engine.file.as_mut().ok_or(...)?;  // 始终操作 file
       let insert_start = file.lines.len();
       // ... 在文件末尾追加 ...
   }
   ```
3. 配合 BUG-201 owner 检查：New(Start/End) owner 为 Open，允许在 Location pending 时执行

**新增测试** (1):
- `test_new_start_operates_at_file_level_even_with_pending_location`

---

### BUG-402: Delete:Block 未校验前一个 Location 使用了 Block 模式 ✅ 已修复

**严重程度**: 严重  
**对应文档**: ncs_dev.md §5.4（"Delete:Block — 删除整个 ContentBlock（要求 Location 也用 Block）"）  
**代码位置**: `ncs/src/commands/delete.rs:43-58`

**修复细节**:

1. **location_is_block 检查**（`delete.rs:43-49`）:
   ```rust
   let location_is_block = engine
       .exec_cmds
       .iter()
       .rev()
       .find(|ec| ec.cmd_name == "LOCATION")
       .is_some_and(|ec| ec.mode_name == "Block");
   ```
2. **错误返回**（`delete.rs:51-55`）: `EngineError::BlockRequiredForDelete`（此前已定义但从未构造）
3. 现有测试更新：`test_delete_block_clears_block` 预置 exec_cmds 中的 Location Block 条目

**新增测试** (2):
- `test_delete_block_with_location_normal_errors`
- `test_delete_block_with_location_block_succeeds`

---

### BUG-403: Location 关闭时不触发终端输出

**状态**: 🔴 未实现  
**严重程度**: 功能缺口  
**对应文档**: ncs_dev.md §5.2（"遇到 @/Location 时触发终端打印"）  
**代码位置**: `ncs/src/engine/mod.rs`（`handle_close()` 中 `LOCATION` 分支）

**待实现**: `engine.print_location_result()` — 在 `handle_close("LOCATION")` 中调用，输出 CmdContent.result（带文件路径和行号的匹配块，灰色）。需配合 CLI `--verbose` 标志。

---

### BUG-404: Location 内容块边界在非标准文件中识别不准确

**状态**: ⏳ 未修复  
**严重程度**: 一般  
**对应文档**: ncs_dev.md §5.2（Location 匹配算法）、INSTRUCTION.md §3.1  
**代码位置**: `ncs/src/matcher.rs`（`find_unique_block`）、`ncs/src/block.rs`（BlockParser）

**发现场景**: `line_range_delete.ncs` 尝试删除 `rust_complex.rs` 中的 `impl Default for AppConfig` 块。该文件中 `impl Default for AppConfig` 缺少闭合 `}};`（与后续 `impl AppConfig` 无缝衔接），导致 Location 匹配的块边界不准确，Delete 无法精确删除目标块。

**根本原因**: `rust_complex.rs` 是故意构造的边缘 case 文件（用于测试非标 Rust 代码），其中的 `impl Default` 不包含完整的大括号闭合。`matcher.rs` 的 `find_unique_block()` 通过前后扩展寻找块边界，但当文件结构异常时（如缺少闭合括号），块边界扩展可能跨入相邻的语法结构，导致 Delete 的作用域不精确。

**影响范围**: 仅影响非标准的、语法不完整的文件。标准格式的 Rust/Go/Python/YAML/JSON/TOML 文件不受影响。

**临时绕过**: 对于非标准文件，使用更小的 Location 范围（单行定位）+ `!@Delete` + `!@New` 逐行替换，而非一次性删除整个块。

---

## 五、Phase 2 范围内的 stub / 未完成项

### BUG-501: `file_io.rs` 仍为 stub

**状态**: 🟢 已修复  
**对应文档**: phases.md §2.2  
**代码位置**: `ncs/src/file_io.rs`（~95 行，含 `read_file()`, `write_file()`, `path_exists()` + 6 tests）

---

### BUG-502: `Open Dir` 模式 stub

**状态**: ⏳ stub（已修正错误类型）  
**对应文档**: phases.md §2.3  
**代码位置**: `ncs/src/commands/open.rs:78-82`

**已修复**: 返回 `EngineError::NotImplemented`（第一阶段 BUG-07）替代误导的 `FileError::NotFound`。

---

### BUG-503: `Location Path` 模式 stub

**状态**: ⏳ stub（已修正错误类型）  
**对应文档**: phases.md §2.4  
**代码位置**: `ncs/src/commands/location.rs:37-39`

**已修复**: 返回 `EngineError::NotImplemented`（第一阶段 BUG-08）替代误导的 `MatchError::NoMatch`。

---

## 六、数据结构与注册表偏差

### BUG-601: `PermissionType` 与文档不一致（Raw / Get）

**状态**: ⏳ 未修复  
| 命令 | 文档权限 | 代码权限 |
|------|---------|---------|
| Raw | ProgramExec | `PermissionType::None` |
| Get | ProgramExec | `PermissionType::None` |

---

### BUG-602: `Open` 命令权限缺少 FileWrite

**状态**: ⏳ 未修复  
**代码位置**: `ncs/src/registry.rs:257-259`

Open 仅注册为 `PermissionType::FileRead`，缺少 FileWrite。

---

### BUG-603: `Command::Open` AST 与文档字段偏差

**状态**: ✅ 非 Bug（代码实现更合理）  
**代码位置**: `ncs/src/parser.rs:27-34`

| 字段 | 文档 | 代码 |
|------|------|------|
| content_lines | ✅ 有 | ❌ 无（正确，Open 是 LineExec） |
| args | ❌ 无 | ✅ 有（正确，start/end 需要 args） |

---

## 七、与 ncs_dev.md 完整对比矩阵

| ncs_dev.md 设计要求 | 当前状态 | 对应 Bug |
|---------------------|:--------:|----------|
| §3.3 CmdContent convert()/out() | 🟡 桥接实现 | BUG-101 |
| §3.3 CommandResult 返回 | 🟡 桥接实现 | BUG-102 |
| §6.2 流输出 / 值输出区分 | 🟢 已修复 | BUG-103 |
| §3.4 / §6.1 pools 数据存储 | 🟢 已修复 | BUG-104 |
| §6.3 exec_cmds owner 检查 | ✅ 已修复 | BUG-201 |
| §6.3 exec_cmds 退出逻辑 | ✅ 已修复 | BUG-202 |
| §3.2 is_independent 字段 | ❌ 未读取 | BUG-203 |
| §5.4 Delete 执行顺序（变更追踪） | 🟢 已修复 | BUG-204 |
| §2.3 @/ 块终止匹配命令名 | ✅ 已修复 | BUG-301 |
| §6.1 Capture 指令 | 🟢 已修复 | BUG-302 |
| §5.12 Get 行内展开 | 🟡 部分实现 | BUG-303 |
| §5.3 New Start/End 文件级 | ✅ 已修复 | BUG-401 |
| §5.4 Delete:Block 校验 | ✅ 已修复 | BUG-402 |
| §2.6 Location Result 打印 | 🟢 已修复 | BUG-403 |
| §5.2 块边界在非标准文件中 | ⚠️ 不精确 | BUG-404 |
| §2.2 file_io.rs | 🟢 已修复 | BUG-501 |
| §5.1 Open Dir 模式 | ⏳ stub | BUG-502 |
| §5.2 Location Path 模式 | ⏳ stub | BUG-503 |
| §5.5 Raw 权限标注 | ⚠️ 不一致 | BUG-601 |
| §8 Open 权限标注 | ⚠️ 不一致 | BUG-602 |
| §4.2 Open AST content_lines | ✅ 非 Bug | BUG-603 |

---

## 修复优先级（更新后）

### 已完成 ✅

| 阶段 | Bug | 内容 |
|------|-----|------|
| 第一阶段 | BUG-01/02/04/07/08/12 | main.rs 引擎接入、diff 输出、pop_exec_cmd、NotImplemented 错误 |
| 第二阶段 | BUG-301/201/202/401/402/302 | @/ 终止、owner 检查、exec_cmds 退出、New 文件级、Delete:Block 校验、Capture |
| 第三阶段（桥接） | BUG-501/302↑/303/104 | file_io.rs、Capture 增强、Get 基本展开、pools 完整写入 |
| 第三阶段（变更追踪） | BUG-204/101/403/103 | ✅ Delete 快照匹配、New 变更记录、Location 终端输出、流输出控制 |

### 第四阶段（待完成 — 桥接彻底化）

| Bug | 内容 |
|-----|------|
| BUG-101/102 | Open/Location 正式 convert/out；命令签名改为返回 CommandResult |
| BUG-204 | handle_close 中的 apply_content_to_file 激活（移除 cmd 层直接 block 操作） |
| BUG-203 | is_independent 实际使用或移除 |
| BUG-303 | Get 块内展开 + like 伪装模式 |
| BUG-404 | 非标准文件块边界识别优化 |
| BUG-601/602 | 权限标注对齐文档 |

---

## 非 Bug 说明

以下为 Phase 3+ 的开发任务：

| 项目 | 所属阶段 |
|------|----------|
| Bash / Exec / Read / Write / Include / WorkPath 命令实现 | Phase 3–4 |
| `!@Write Raw` 特殊行为（收集到 EOF） | Phase 3 |
| Bash 安全审查 | Phase 3 |
| Include 动态注册外部命令 | Phase 4 |
| WorkPath 设置工作目录 | Phase 4 |
| Get 作为独立命令（like 伪装） | Phase 5 |
| 行号 Delete（@start,end） | Phase 5 |
| Open Dir 完整实现 | Phase 3 |
| Location Path 完整实现 | Phase 3 |
