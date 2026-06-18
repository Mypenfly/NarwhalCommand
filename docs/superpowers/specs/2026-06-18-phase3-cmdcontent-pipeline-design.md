# Phase 3: CmdContent 数据流 + 变更追踪 — 设计文档

> 对应 bugs.md 第三阶段修复清单：BUG-204 / 101 / 102 / 103 / 104 / 303 / 403 / 501

---

## 1. 核心设计理念

### 1.1 问题

当前实现中，New/Delete 命令通过 `engine.get_search_scope()` 获取 `&mut ContentBlock`，直接执行行插入/删除。这导致三个根本问题：

1. **BUG-204**：New 在 Delete 之前修改 ContentBlock → Delete 匹配失败
2. **强耦合**：命令直接操作 n_edit 内部类型（Line/ContentBlock），而非通过 CmdContent
3. **不可追踪**：修改直接生效，无变更记录，无法撤销

### 1.2 变更追踪模型

```
命令操作       →  记录 ContentChange   →  延迟应用到 ContentBlock/File
                  （Insert / Delete）       （Owner 命令关闭时）
```

**数据流向（块内）**：

```
Location ──out()──► CmdContent(snapshot=匹配的行)
                         │
              ┌──────────┘ (同一个 CmdContent 按顺序流转)
              ▼
New ──convert()──► 查看 snapshot, 追加 ContentChange::Insert ──out()──► CmdContent
                         │
              ┌──────────┘
              ▼
Delete ──convert()──► 在 snapshot 中匹配, 追加 ContentChange::Delete ──out()──► CmdContent
                         │
              ┌──────────┘
              ▼
@/Location ──► CmdContent.apply_changes() ──► 写入 ContentBlock.lines → write_back
```

**关键规则**：

1. 所有命令（除仅展开命令）的数据流动**统一为 CmdContent**
2. 命令从**前一条流输出命令**获取输入 CmdContent（后一个命令始终从前一个命令得到）
3. 块内（`!@X ... @/X` 之间）共享**同一个** CmdContent，命令串行查看/修改
4. 命令不直接修改文件行 — 仅向 CmdContent **追加 ContentChange 记录**
5. 变更在 Owner 命令退出时（`@/Cmd`）由 CmdContent 统一生效
6. Delete 匹配始终使用 `snapshot_lines`（Location 创建时的原始数据）
7. 修改可追踪（changes 列表记录每次变更的来源命令）

---

## 2. 数据结构

### 2.1 ContentChange

```rust
/// 内容变更记录 — 命令对 CmdContent 的修改追踪
pub enum ContentChange {
    /// 插入变更（来自 New）
    Insert {
        /// 在 snapshot 中的插入位置（行索引，插入到该行之后）
        after_line: usize,
        /// 插入的行内容
        lines: Vec<CmdLine>,
        /// 变更来源命令（"NEW" / "GET"）
        source_cmd: String,
    },
    /// 删除变更（来自 Delete）
    Delete {
        /// 在 snapshot 中的删除起始行索引
        start_line: usize,
        /// 在 snapshot 中的删除结束行索引（含）
        end_line: usize,
        /// 变更来源命令
        source_cmd: String,
    },
}
```

### 2.2 CmdContent 扩展

```rust
pub struct CmdContent {
    // === 现有字段 ===
    pub raw_content: String,
    pub lines: Vec<CmdLine>,
    pub is_print: bool,
    pub result: Vec<CmdLine>,

    // === Phase 3 新增 ===
    /// Location 创建时的原始数据快照（不可变，仅追加不会修改）
    pub snapshot_lines: Vec<CmdLine>,
    pub snapshot_raw: String,
    /// 变更记录列表（按命令执行顺序追加）
    pub changes: Vec<ContentChange>,
    /// 数据来源信息（用于定位写回目标）
    pub source_info: Option<ContentSource>,
}

/// CmdContent 的来源（决定写回目标）
pub enum ContentSource {
    /// 来源为 ContentBlock（Location 匹配产生）
    Block {
        /// block_stack 中的索引位置
        block_index: usize,
    },
    /// 来源为整个文件（Open 产生）
    File {
        file_path: String,
    },
    /// 来源为命令输出（Bash / Get 产生）
    CommandOutput,
}
```

### 2.3 CmdContent 变更操作方法

```rust
impl CmdContent {
    /// 记录一个 Insert 变更
    pub fn record_insert(&mut self, after_line: usize, lines: Vec<CmdLine>, source_cmd: &str) {
        self.changes.push(ContentChange::Insert {
            after_line,
            lines,
            source_cmd: source_cmd.to_string(),
        });
    }

    /// 记录一个 Delete 变更
    pub fn record_delete(&mut self, start_line: usize, end_line: usize, source_cmd: &str) {
        self.changes.push(ContentChange::Delete {
            start_line,
            end_line,
            source_cmd: source_cmd.to_string(),
        });
    }

    /// 将所有变更应用到 snapshot，生成最终 lines
    /// 在 owner 命令退出时调用
    pub fn apply_changes(&mut self) {
        let mut result = self.snapshot_lines.clone();

        for change in &self.changes {
            match change {
                ContentChange::Insert { after_line, lines, .. } => {
                    let insert_pos = after_line + 1; // 插入到行之后
                    let pos = insert_pos.min(result.len());
                    // 将插入行 splice 进去
                    for (i, line) in lines.iter().enumerate() {
                        result.insert(pos + i, line.clone());
                    }
                }
                ContentChange::Delete { start_line, end_line, .. } => {
                    if *start_line <= *end_line && *end_line < result.len() {
                        result.drain(*start_line..=*end_line);
                    }
                }
            }
        }

        self.lines = result;
        self.raw_content = self.lines.iter()
            .map(|l| l.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
    }

    /// 计算变更后的行数（不实际应用变更）
    pub fn changed_line_count(&self) -> usize {
        let mut count = self.snapshot_lines.len() as isize;
        for change in &self.changes {
            match change {
                ContentChange::Insert { lines, .. } => count += lines.len() as isize,
                ContentChange::Delete { start_line, end_line, .. } => {
                    count -= (end_line + 1 - start_line) as isize;
                }
            }
        }
        count.max(0) as usize
    }
}
```

---

## 3. Engine 改动

### 3.1 新增字段

```rust
pub struct Engine {
    // === 现有字段 ===
    pub file_path: Option<String>,
    pub file: Option<FileContent>,
    pub block_stack: Vec<ContentBlock>,
    pub diff_lines: Vec<DiffLine>,
    pub last_diff_block_key: Option<(usize, usize)>,
    pub exec_cmds: Vec<ExecutedCommand>,
    pub pools: HashMap<String, CmdContent>,
    pub verbose: bool,

    // === Phase 3 新增 ===
    /// 上一条命令的 CommandResult（供 Capture 和下一个命令输入使用）
    pub last_result: Option<CommandResult>,
}
```

### 3.2 execute() 重写

```rust
pub fn execute(&mut self, commands: &[Command], registry: &CommandRegistry) -> Result<(), NcsError> {
    for command in commands {
        match command {
            Command::Close { name, capture } => {
                // 0. 在关闭前，如果当前有待生效的 CmdContent，先应用变更
                if let Some(ref mut content) = self.last_result.as_mut().map(|r| &mut r.content) {
                    if !content.changes.is_empty() {
                        self.apply_content_to_file(content)?;
                    }
                }

                // 1. Capture: 将 last_result 存入 pools
                if let Some(pool_name) = capture {
                    if let Some(result) = self.last_result.take() {
                        self.pools.insert(pool_name.clone(), result.content);
                    }
                }

                // 2. 执行关闭逻辑
                self.handle_close(name, registry)?;
            }
            other => {
                // 0. 权限检查
                self.check_owner(command, registry)?;

                // 1. convert: 从前一条命令获取输入
                let input = self.last_result.as_ref()
                    .map(|r| r.content.clone())
                    .unwrap_or_else(CmdContent::empty);
                let internal = command.convert(input, self)?;

                // 2. execute_core: 核心逻辑（可能记录变更到 internal）
                let raw_result = command.execute_core(self, internal)?;

                // 3. out: 序列化输出
                let output = command.out(raw_result, self);

                // 4. 构建 CommandResult
                let output_type = self.get_command_output_type(command, registry);
                let is_stream = matches!(output_type, Some(OutputType::StreamOutput));
                let cmd_result = CommandResult { content: output, is_stream };

                // 5. 存储结果（供下一个命令使用）
                self.last_result = Some(cmd_result);

                // 6. 加入 exec_cmds
                self.push_exec_cmd(command);
            }
        }
    }

    // 隐式关闭：如有待生效变更，先应用
    if let Some(ref mut content) = self.last_result.as_mut().map(|r| &mut r.content) {
        if !content.changes.is_empty() {
            self.apply_content_to_file(content)?;
        }
    }
    self.handle_implicit_close(registry)?;
    Ok(())
}
```

### 3.3 handle_close 重写

```rust
fn handle_close(&mut self, name: &str, registry: &CommandRegistry) -> Result<(), NcsError> {
    let upper = name.to_uppercase();

    // Location 关闭：触发终端输出（BUG-403）
    if upper == "LOCATION" && self.verbose {
        self.print_location_result()?;
    }

    // 关闭逻辑：block_stack / file / exec_cmds 清理
    match upper.as_str() {
        "LOCATION" | "NEW" => self.handle_block_close()?,
        "OPEN" => self.handle_open_close()?,
        _ => {
            self.pop_exec_cmd(&upper);
        }
    }

    self.last_result = None;
    Ok(())
}
```

### 3.4 apply_content_to_file — 变更生效入口

```rust
/// 将 CmdContent 中记录的变更写入对应的 ContentBlock 或 FileContent
fn apply_content_to_file(&mut self, content: &CmdContent) -> Result<(), NcsError> {
    match &content.source_info {
        Some(ContentSource::Block { block_index }) => {
            // 变更作用于 block_stack 中的某个 ContentBlock
            if let Some(block) = self.block_stack.get_mut(*block_index) {
                self.apply_changes_to_block(block, content)?;
            }
        }
        Some(ContentSource::File { .. }) => {
            // 变更作用于整个文件（Open → New Start/End）
            if let Some(ref mut file) = self.file {
                self.apply_changes_to_file(file, content)?;
            }
        }
        Some(ContentSource::CommandOutput) | None => {
            // 命令输出，无需写入文件
        }
    }
    Ok(())
}

/// 将 CmdContent 的变更应用到 ContentBlock
fn apply_changes_to_block(&mut self, block: &mut ContentBlock, content: &CmdContent) -> Result<(), NcsError> {
    // 1. 收集 diff 数据（从 content.changes 构建 diff_lines）
    self.record_diff_from_changes(block, content)?;

    // 2. 将 ContentChange 转换为 ContentBlock 的实际行操作
    let mut lines = content.snapshot_lines.clone();
    for change in &content.changes {
        match change {
            ContentChange::Insert { after_line, lines: insert_lines, .. } => {
                // 转换 CmdLine → Line
                let n_edit_lines: Vec<Line> = insert_lines.iter().map(|cl| {
                    Line {
                        line_num: LineNumber::new(1), // 待 reindex
                        taps: 0,
                        diff_taps: 0,
                        content: cl.content.clone(),
                        stripped_content: strip_whitespace(&cl.content),
                    }
                }).collect();
                // ... 插入到 block.lines 的正确位置 ...
            }
            ContentChange::Delete { start_line, end_line, .. } => {
                // 从 block.lines 中 drain
                block.lines.drain(*start_line..=*end_line);
            }
        }
    }

    block.reindex();
    Ok(())
}
```

---

## 4. 各命令 convert/execute_core/out 设计

### 4.1 Command 枚举新增方法

```rust
impl Command {
    fn convert(&self, input: CmdContent, engine: &Engine) -> Result<CmdContent, NcsError>;
    fn execute_core(&self, engine: &mut Engine, internal: CmdContent) -> Result<CmdContent, NcsError>;
    fn out(&self, result: CmdContent, engine: &Engine) -> CmdContent;
    fn cmd_name(&self) -> String;
    fn mode_name(&self) -> String;
}
```

### 4.2 Open

```
convert(): 空输入 → 空 CmdContent（无上游）
execute_core(): 读文件 → engine.file
out(): FileContent → CmdContent(snapshot=文件行, source=File)
```

```rust
fn open_out(file: &FileContent, file_path: &str) -> CmdContent {
    CmdContent {
        snapshot_lines: file_to_cmd_lines(file),
        snapshot_raw: file.lines.iter().map(|l| &l.content).join("\n"),
        lines: file_to_cmd_lines(file),
        raw_content: file.lines.iter().map(|l| &l.content).join("\n"),
        changes: vec![],
        source_info: Some(ContentSource::File { file_path: file_path.to_string() }),
        is_print: false,
        result: vec![],
    }
}
```

### 4.3 Location

```
convert(): 合并 Open 输出的 CmdContent 与自身的 LocationContent
execute_core(): 在 Engine.file 中匹配定位 → 创建 ContentBlock 推入 block_stack
out(): 将 ContentBlock 内容转为 CmdContent(snapshot=匹配行, source=Block)
```

**关键**：Location 的 out() 创建的 CmdContent 是块内所有后续命令的操作对象。

```rust
fn location_out(block: &ContentBlock, block_index: usize, file_path: &str) -> CmdContent {
    let cmd_lines = block_to_cmd_lines(block);
    CmdContent {
        snapshot_lines: cmd_lines.clone(),  // 锁定的原始快照
        snapshot_raw: cmd_lines.iter().map(|l| &l.content).join("\n"),
        lines: cmd_lines,
        raw_content: "".to_string(),
        changes: vec![],                     // 初始无变更
        source_info: Some(ContentSource::Block { block_index }),
        is_print: true,
        result: format_location_result(block, file_path),
    }
}
```

### 4.4 New

```
convert(): 接收上一条命令的 CmdContent + 自身的 NewContent → 返回同样的 CmdContent
           内部将 NewContent 转换为待插入的 CmdLine 列表（存入临时字段或 attachment）
execute_core(): 从 CmdContent.snapshot_lines 计算插入位置（基于 match_info 或 args）
               调用 content.record_insert(...) 追加 ContentChange::Insert
out(): 返回 CmdContent（现在包含新的 Insert 变更记录）
```

```rust
fn new_execute_core(engine: &Engine, mut content: CmdContent) -> Result<CmdContent, NcsError> {
    // 计算插入位置（基于 Location 的匹配结果）
    let insert_pos = if let Some(Source::Block { block_index }) = &content.source_info {
        if let Some(block) = engine.block_stack.get(*block_index) {
            match &block.match_info {
                MatchInfo::Location { matched_line_count } => *matched_line_count,
                MatchInfo::DeleteAt { position } => *position,
                MatchInfo::Empty => content.snapshot_lines.len(),
            }
        } else {
            content.snapshot_lines.len()
        }
    } else {
        content.snapshot_lines.len()
    };

    // 记录 Insert 变更
    let new_lines = extract_new_lines_from_content(&content); // 从自身 NewContent 来
    content.record_insert(insert_pos, new_lines, "NEW");

    Ok(content)
}
```

### 4.5 Delete

```
convert(): 接收上一条命令的 CmdContent + 自身的 DeleteContent → 返回同样的 CmdContent
           内部将 DeleteContent 转换为待匹配的行
execute_core(): 在 CmdContent.snapshot_lines 中匹配删除内容（使用现有的去空白匹配逻辑）
               匹配成功后调用 content.record_delete(...) 追加 ContentChange::Delete
out(): 返回 CmdContent（现在包含新的 Delete 变更记录）
```

```rust
fn delete_execute_core(engine: &Engine, mut content: CmdContent) -> Result<CmdContent, NcsError> {
    // 在 snapshot_lines 中匹配（不受其他命令的 Insert/Delete 影响）
    // 原因：BUFFER-204 — snapshot_lines 是 Location 时的原始数据
    let del_lines = extract_delete_lines_from_content(&content);
    let (start_idx, end_idx) = find_delete_match_in_snapshot(&content.snapshot_lines, &del_lines)?;

    // 检查邻接性（Delete 首行紧邻 Location 最后一行）
    check_delete_adjacency_in_snapshot(&content.snapshot_lines, start_idx)?;

    // 记录 Delete 变更
    content.record_delete(start_idx, end_idx, "DELETE");

    Ok(content)
}
```

**BUG-204 修复原理**：Delete 始终在 `snapshot_lines` 中匹配。无论 New 是否在 Delete 之前执行（都只追加 ContentChange::Insert），snapshot_lines 不受影响。因此 Delete 的匹配永远正确。

### 4.6 Bash / Exec / Read / Write

Phase 3 仅实现骨架（沿用 `NotImplemented` 错误）。其中 Bash 的 convert/execute_core/out 预留接口：

```
convert(): input.raw_content 或自身命令字符串
execute_core(): std::process::Command::new("bash") ...
out(): stdout → CmdContent(snapshot=stdout行, source=CommandOutput)
```

### 4.7 Get

```
convert(): pool_name 对应的 CmdContent（从 pools 获取）
execute_core(): 透传
out(): 透传（source_info 保持为原始来源）
```

---

## 5. 变更生效（Owner 命令退出时）

### 5.1 时机

在 `execute()` 主循环中，遇到 `Command::Close` 时，**首先**将 `last_result` 中的 CmdContent 变更应用到文件/Block，**然后**执行关闭清理。

```rust
Command::Close { name, capture } => {
    // 先应用变更
    if let Some(ref mut content) = self.last_result.as_mut().map(|r| &mut r.content) {
        if !content.changes.is_empty() {
            self.apply_content_to_file(content)?;
            content.changes.clear(); // 清空已应用的变更
        }
    }
    // 再 Capture
    if let Some(pool_name) = capture { ... }
    // 最后关闭清理
    self.handle_close(name, registry)?;
}
```

### 5.2 生效流程

```
CmdContent ──► 遍历 changes ──► 找到 source_info ──► 锁定 ContentBlock ──► drain / insert
     │                                                                           │
     │                                    ┌──────────────────────────────────────┘
     │                                    ▼
     │                              block.reindex()
     │                              engine.record_diff()
     │
     └──► 变更已生效，source_info 不变（供 Capture 保存）
```

### 5.3 diff 记录

从 ContentChange 列表构建 diff_lines（替代现有的 `record_diff_with_context()` 直接操作 block）：

```rust
fn record_diff_from_changes(&mut self, block: &ContentBlock, content: &CmdContent) -> Result<(), NcsError> {
    for change in &content.changes {
        match change {
            ContentChange::Insert { after_line, lines, .. } => {
                for line in lines {
                    self.diff_lines.push(DiffLine {
                        kind: DiffLineKind::Added,
                        line_number: LineNumber::from_index(*after_line + 1),
                        content: line.content.clone(),
                    });
                }
            }
            ContentChange::Delete { start_line, end_line, .. } => {
                for i in *start_line..=*end_line {
                    if let Some(line) = content.snapshot_lines.get(i) {
                        self.diff_lines.push(DiffLine {
                            kind: DiffLineKind::Deleted,
                            line_number: LineNumber::from_index(i + 1),
                            content: line.content.clone(),
                        });
                    }
                }
            }
        }
    }
    Ok(())
}
```

---

## 6. Pools / Capture / Get (BUG-104 / 303)

### 6.1 Capture

Lexer / Parser 扩展（同原方案）：

```rust
// Token::Close 增加 capture 字段
Token::Close {
    name: String,
    capture: Option<String>,
}

// Command::Close 增加 capture 字段
Command::Close {
    name: String,
    capture: Option<String>,
}
```

执行时机：变更已生效 → last_result 中的 CmdContent（已含完整数据 + changes 历史）→ 存入 pools。

### 6.2 Get

```rust
// Get 的 execute_core: 从 pools 提取
fn get_execute_core(engine: &Engine, _internal: CmdContent) -> Result<CmdContent, NcsError> {
    let pool_name = /* 从自身 AST 获取 */;
    let content = engine.pools.get(&pool_name)
        .cloned()
        .ok_or_else(|| EngineError::PoolNotFound { pool_name })?;
    Ok(content) // 透传
}
```

`!@Get with like=...` 属于 Phase 5，Phase 3 仅实现基本展开。

---

## 7. Location 终端输出 (BUG-403)

```rust
fn print_location_result(&self) -> Result<(), NcsError> {
    if let Some(content) = &self.last_result {
        if content.is_print && !content.result.is_empty() {
            for line in &content.result {
                // 灰色输出
                println!("{}", line.content);
            }
        }
    }
    Ok(())
}
```

---

## 8. 流输出 / 值输出 (BUG-103)

接入 `CommandType.output`（同原方案）。关键区别：StreamOutput 命令的 `last_result` 保留给后续命令；ValueOutput 仅打印后丢弃。

---

## 9. file_io.rs 补全 (BUG-501)

同原方案：`read_file()` / `write_file()` / `path_exists()` 三个工具函数。

---

## 10. Error 扩展

```rust
enum EngineError {
    // ... 现有变体 ...
    PoolNotFound { pool_name: String },
    ChangeApplicationFailed { reason: String },
}
```

---

## 11. 实现顺序

```
阶段 A：数据结构基础
├── Step 1: ContentChange 枚举 + CmdContent 扩展（snapshot/changes/source_info）
├── Step 2: CmdContent::record_insert/record_delete/apply_changes
├── Step 3: file_io.rs 补全 (BUG-501)

阶段 B：Command 三步法骨架（先 stub，跑通编译）
├── Step 4: Command::cmd_name() / mode_name() 方法
├── Step 5: Command::convert() / execute_core() / out() 方法定义
├── Step 6: 每个 Command 变体的 stub 实现（返回空 CmdContent）

阶段 C：Engine 管道重写
├── Step 7: Engine.execute() 重写（三步法调用 + Close 变更生效 + Capture）
├── Step 8: Engine.apply_content_to_file / apply_changes_to_block
├── Step 9: engine.handle_close() 整合 Location 终端输出

阶段 D：命令实现填充
├── Step 10: Open convert/out — 文件 → CmdContent
├── Step 11: Location convert/out — 匹配 → CmdContent(snapshot+source)
├── Step 12: New execute_core — record_insert (BUG-204 自动解决)
├── Step 13: Delete execute_core — 在 snapshot 匹配, record_delete (BUG-204 自动解决)
├── Step 14: 旧 commands/{new,delete}.rs 中的直接行操作代码移除
├── Step 15: diff 记录从 ContentChange 构建

阶段 E：Pools & Get (BUG-104/303)
├── Step 16: Lexer 管道解析 (| Capture pool_name)
├── Step 17: Capture 存入 pools (在 execute 主循环中)
├── Step 18: Get execute_core 实现（从 pools 读取）
├── Step 19: Get 在 Parser 块提取中展开（仅展开，不触发终止）

阶段 F：验证
├── Step 20: 全量测试回归 + clippy + fmt
├── Step 21: BUG-204 场景严格断言 (scenario03)
├── Step 22: 新增测试（见 §12）
├── Step 23: 更新 ncs_dev.md（变更追踪模型写入 §3.3 / §5.3 / §5.4）
```

---

## 12. 测试策略

### 12.1 单元测试

| Test | 对应 Bug | 说明 |
|------|---------|------|
| `test_content_change_insert_recorded` | BUG-101 | record_insert 后 changes 列表增长 |
| `test_content_change_delete_recorded` | BUG-101 | record_delete 后 changes 列表增长 |
| `test_apply_changes_insert_only` | BUG-101 | 仅有 Insert 变更时 apply_changes 正确 |
| `test_apply_changes_delete_only` | BUG-101 | 仅有 Delete 变更时 apply_changes 正确 |
| `test_apply_changes_insert_then_delete` | BUG-204 | Insert+Delete 组合生效正确 |
| `test_delete_matches_snapshot_not_current` | BUG-204 | 多命令变更后 Delete 匹配仍用 snapshot |
| `test_location_delete_new_ordering` | BUG-204 | 三个命令正确执行 |
| `test_open_convert_out` | BUG-101 | Open → CmdContent 格式正确 |
| `test_location_convert_out_has_snapshot` | BUG-101 | Location out 包含 snapshot |
| `test_command_result_stream_flag` | BUG-102/103 | is_stream 正确反映 OutputType |
| `test_capture_stores_result_in_pools` | BUG-104 | @/Open \| Capture x → pools["x"] 有值 |
| `test_get_reads_from_pools` | BUG-303 | !@Get x → 返回正确 CmdContent |
| `test_location_close_prints_result` | BUG-403 | @/Location 触发终端输出 |
| `test_snapshot_never_modified_by_changes` | BUG-204 | record_insert/delete 不改变 snapshot_lines |

### 12.2 集成测试

| 变更 | 说明 |
|------|------|
| `scenario03_replace_func.ncs` | lenient → 严格断言 |
| 新增 `change_tracking_basic.ncs` | Open → Location → New → Delete → 验证 diff |
| 新增 `change_tracking_reverse.ncs` | Location → Delete → New 顺序验证 |
| 新增 `capture_basic.ncs` | Open → Loc → @/Open \| Capture → !@Get |
| 新增 `location_verbose.ncs` | 验证 Location 终端输出 |

---

## 13. 破坏性影响评估

| 模块 | 变更程度 | 说明 |
|------|---------|------|
| `engine/mod.rs` | **重写** | execute() / handle_close() / apply_content_to_file |
| `commands/new.rs` | **大幅修改** | 移除直接行插入，改为 record_insert |
| `commands/delete.rs` | **大幅修改** | 移除直接行删除，改为 snapshot 匹配 + record_delete |
| `commands/location.rs` | 小幅修改 | out() 创建带 snapshot 的 CmdContent |
| `commands/open.rs` | 小幅修改 | out() 创建带 snapshot 的 CmdContent |
| `cmd_content.rs` | **大幅扩展** | ContentChange / snapshot / changes / source_info / apply |
| `model.rs` | 不变 | ContentBlock 不需要 snapshot_lines 字段 |
| `matcher.rs` | 不变 | Location 匹配逻辑完全不变 |
| `lexer.rs` | Token::Close 加字段 | Capture 管道解析 |
| `parser.rs` | Command::Close 加字段 + Get 展开 | — |
| `error.rs` | 新增 2 个变体 | PoolNotFound / ChangeApplicationFailed |
| `n_edit` | **不变** | 完全不受影响 |
| `ncs_dev.md` | **更新** | §3.3 / §5.3 / §5.4 写明变更追踪模型 |

---

## 14. ncs_dev.md 需更新的段落

1. **§3.3 CmdContent** — 新增 snapshot_lines / changes / source_info 字段说明及变更追踪模型
2. **§5.3 New** — "直接插入行" → "向 CmdContent 追加 ContentChange::Insert，由 Owner 关闭时生效"
3. **§5.4 Delete** — "在 ContentBlock 内逐行匹配并删除" → "在 CmdContent.snapshot_lines 匹配 + ContentChange::Delete，延迟生效"
4. **§6.4 数据传递路径** — 更新数据流图体现 shared CmdContent + changes 累积
