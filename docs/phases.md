# N_Edit 实现阶段拆分

## Phase 0: 项目骨架搭建 ✅ 已完成

**目标**：可编译、可测试的最小 Rust 项目。

### 任务清单

- [x] `cargo init` 初始化项目，Cargo.toml 配置
- [x] 引入依赖：`clap`（CLI）、`colored`（终端颜色）
- [x] 入口 `main.rs`：接收文件路径参数，读取脚本内容并打印
- [x] 搭目录骨架：`model.rs`、`error.rs`、`lexer.rs`、`parser.rs`、`engine.rs`、`matcher.rs`、`block.rs`、`file_io.rs`、`output.rs`
- [x] `error.rs` 中建立根错误类型 `NEditError`（枚举，含 Display + Error trait）

### 当前产出物

```
src/
  lib.rs       # 库入口，导出所有模块供集成测试
  main.rs      # clap 参数解析，调用 lexer → parser → engine
  model.rs     # 核心数据结构（Line, ContentBlock, FileContent 等）
  error.rs     # 所有错误类型集中定义
  lexer.rs     # 词法分析器（含 Separator token）
  parser.rs    # 语法分析器（含歧义检测）
  engine.rs    # 执行引擎（状态机 + New/Delete + diff 追踪）
  matcher.rs   # 核心匹配算法（含空 Location 处理）
  block.rs     # Block 解析器（Phase 3 实现）
  file_io.rs   # 文件读写（已集成到 model.rs）
  output.rs    # 彩色终端输出（含 DiffLine 格式化）
tests/
  data/        # 测试用真实 Rust 源码（services.rs, config.rs, sample.rs）
  scripts/     # .ned 测试脚本
  integration_test.rs  # 21 个端到端集成测试
```

### 验证

```bash
cargo build
cargo run -- example.ned
```

---

## Phase 1: Open / Location 匹配（纯文本）/ Off ✅ 已完成

**目标**：最基础的读取-定位-写回流程。验证核心匹配算法。

### 1.1 数据结构

- [x] `model.rs`：定义 `Line`、`ContentBlock`、`FileContent`、`LocationLine`、`LocationContent`、`FirstMatchContent`、`MatchLine`
- [x] `Line` 实现 `stripped_content()` — 预计算并缓存去空白版本
- [x] `FileContent` 实现 `from_path(path)` — 读文件、逐行解析（计算 taps）、构建首行哈希索引
- [x] `FileContent` 实现 `write_back(path)` — 按行写回文件

### 1.2 词法分析

- [x] `lexer.rs`：识别 `//!@` 标识符前缀
- [x] 识别命令头：`Open:`、`Location:`、`Off:`
- [x] 提取命令内容块（直到 `...` 分隔符或下一个 `//!@`）
- [x] 输出 Token 流：`Vec<Token>`
- [x] Token 含行号信息，供错误定位
- [x] 新增 `Separator` token 用于追踪 `...` 分隔符

```rust
enum Token {
    Open { file_path: String, line: usize },
    Location { lines: Vec<String>, line: usize },
    Off { target: String, line: usize },
    Separator { line: usize },   // Phase 2 新增
}
```

### 1.3 语法分析

- [x] `parser.rs`：Token 流 → AST（`Vec<Command>`）
- [x] 解析 `LocationContent`：从 Location Token 的 lines 构建 `Vec<LocationLine>`（计算 diff_taps）
- [x] 语法校验：Off 命令的 target 是否合法（Open / Location / New）
- [x] 语法校验：New:Normal 和 Delete 前必须有 Location（不能隔 `...`）

### 1.4 核心匹配算法

- [x] `matcher.rs`：实现 `find_unique_block(file, location) -> Result<ContentBlock, MatchError>`
  - 首行去空白匹配 → 使用 first_line_index HashMap O(1) 查找候选集
  - 逐行比对（去空白 + diff_taps）→ 筛选
  - 结果唯一性校验
- [x] 空 Location 返回整个搜索范围作为 ContentBlock（matched_line_count=0）
- [x] Block 边界：暂以"从匹配首行到文件末尾"作为范围（Phase 3 再做精确 Block 解析）

### 1.5 执行引擎

- [x] `engine.rs`：状态机
  - `Open` → 读文件 → 构建 `FileContent`
  - `Location` → 调用 matcher → 得到 ContentBlock → push 到 `block_stack`
  - `Off:Location` → pop block_stack → 写回上层 block
  - `Off:Open` → pop 所有 → write_back_to_file
  - 脚本末尾无 Off → 隐式 `Off:Open`
- [x] `apply_block_to_file` 使用 truncate+extend 方式，支持 block 行数增减

### 1.6 性能优化

- [x] 预计算 stripped_content：FileContent 构建时对每行预存去空白版本
- [x] 首行哈希索引：`HashMap<String, Vec<usize>>` 避免全量扫描（INSTRUCTION.md 4.2）

---

## Phase 2: New / Delete（在 ContentBlock 内操作） ✅ 已完成

**目标**：基于 Phase 1 的 ContentBlock，实现内容修改。

### 2.1 数据结构扩展

- [x] `model.rs`：定义 `NewContent`、`NewLine`、`DeleteContent`、`DeleteLine`
- [x] `Command` 枚举新增：`New { position, content }`、`Delete { block, content }`
- [x] `NewPosition` 枚举：`Normal`、`Start`、`End`

### 2.2 词法/语法扩展

- [x] lexer 识别：`New:`、`New:Start`、`New:End`、`Delete:`
- [x] 提取 New/Delete 内容块（到 `...` 或下一命令）
- [x] parser：构建 `NewContent` / `DeleteContent` 结构
- [x] **歧义保护**：parser 追踪 Location→Separator→New/Delete 状态，若 New:Normal/Delete 前出现 `...`，报 `ParseError::MissingLocation`
- [x] **修饰符处理**：`Location:Block` 和 `Delete:Block` 的 `Block` 不作为内容提取

### 2.3 New 插入逻辑

- [x] `engine.rs`：实现 `execute_new()`
  - Normal：在 Location 最后一行之后插入（matched_line_count=0 时在 block 开头插入）
  - 计算缩进（diff_taps 为绝对缩进量），逐行插入
  - 重算受影响行的 `line_num`（调用 `block.reindex()`）
- [x] `New:Start`：插入到文件/block 开头（不要求 Location）
- [x] `New:End`：插入到文件/block 末尾（不要求 Location）

### 2.4 Delete 删除逻辑

- [x] `engine.rs`：实现 `execute_delete()`
  - 检查 `block_stack` 非空
  - 在 block 内逐行匹配 del_content（去空白匹配）
  - 要求连续匹配，不可跳行
  - **邻接检查**：Delete 首行必须紧邻 Location 最后一行（matched_line_count>0 时），否则报 `DeleteNotAdjacent`
  - 删除匹配区间 → `block.reindex()`

### 2.5 输出

- [x] `output.rs`：`DiffLineKind`（Added/Deleted/Unchanged）+ `OutputFormatter`
  - 新增行前加绿色 `+`，含行号
  - 删除行前加红色 `-`，含行号
  - 检测终端能力（`is_terminal`），管道/重定向时自动关闭颜色
  - 引擎执行后自动输出 diff 结果

### 2.6 错误处理扩展

- [x] `ParseError::MissingLocation` — 歧义的 `...` 分隔
- [x] `MatchError::DeleteMatchFailed` — 删除内容未找到
- [x] `MatchError::DeleteNotAdjacent` — 删除位置与 Location 不紧邻
- [x] 所有错误包含中文描述和修复建议

### 验证状态

```bash
cargo test                          # 133 个测试全部通过
cargo clippy                        # 零 warning
cargo run -- test_new_delete.ned   # 执行成功，输出带 + / - 的 diff
```

---

## Phase 3: Location:Block + Delete:Block

**目标**：代码块精确识别（花括号 / 缩进/类似lsp的精确识别），支持整块增删。

> **当前状态**：Lexer 已处理 `Location:Block` / `Delete:Block` 修饰符（不泄漏为内容），
> 但 BlockParser 和对应的执行逻辑尚未实现。

### 3.1 BlockParser

- [ ] `block.rs`：实现 `BlockParser`
  - `parse_brace_block(file, start_line)` — 花括号语言，逐字符扫描建树
  - `parse_indent_block(file, start_line)` — 缩进语言，缩进层级判断
  - `detect_language(first_match)` — 语言类型检测
- [ ] 花括号扫描器：
  - 维护 `depth`、`in_string`、`in_comment` 状态
  - 处理 `\"`, `\\` 转义
  - 处理 `//` 行注释、`/* */` 块注释

### 3.2 数据结构扩展

- [ ] `Command` 枚举更新：
  - `Location` 增加 `block: bool` 标志
  - `Delete` 增加 `block: bool` 标志（字段已存在，需在 parser 中设置）

### 3.3 词法/语法扩展

- [x] lexer 识别：`Location:Block`、`Delete:Block`（Lexer 已支持，修饰符不泄漏）
- [ ] parser：正确设置 `block` 标志（从 Token 无法传递，需扩展 Token 或 parser 逻辑）
- [ ] 语法校验：`Delete:Block` 要求前一个 Location 也使用 Block

### 3.4 执行引擎扩展

- [ ] Location:Block → 调用 BlockParser → 精确 ContentBlock（非"到文件末尾"）
- [ ] Block 不可解析时拒绝 Block 指令 → 报错
- [ ] Delete:Block → 删除整个 ContentBlock（仅保留首行行号以避免空行）

### 3.5 匹配器增强

- [ ] matcher 引入 diff_taps 校验（当前已实现，Phase 1 已含）
- [ ] 跳过空行比对（当前已实现）

### 实现建议

1. **Token 扩展**：当前 `Token::Location { lines, line }` 缺少 `block` 字段，建议新增 `Token::Location { block: bool, lines, line }`，同样扩展 `Token::Delete`。
2. **Block 边界替换**：当前 `build_content_block` 取 `file.lines[start_index..]` 到末尾，Phase 3 需替换为 BlockParser 输出的精确范围。
3. **Language 检测时机**：在 `expect_single_match` 拿到唯一匹配后，立即对 matched 行做语言检测，而非事后推断。
4. **花括号扫描器**：建议先实现 Rust/C/JS 的花括号扫描（覆盖主流语言 80%+ 场景），再实现缩进语言（Python/YAML）。

### 验证

```bash
cargo test block::tests::test_brace_block
cargo test block::tests::test_indent_block
cargo test engine::tests::test_delete_block
```

---

## Phase 4: 嵌套 Location

**目标**：Location 在 ContentBlock 内再次定位，递归缩小范围。

> **当前状态**：空 Location 已支持（返回整个搜索范围），但 SearchScope 未抽象，
> `get_search_scope()` 目前始终返回 `&FileContent`。

### 4.1 匹配器改造

- [ ] `matcher.rs`：`find_unique_block` 增加 `search_scope` 参数
  - 顶层 Location → 搜索范围 = FileContent
  - 嵌套 Location → 搜索范围 = 当前 ContentBlock
- [ ] 抽象 `SearchScope` trait 或使用 enum 统一 FileContent 和 ContentBlock 的访问接口

### 4.2 执行引擎扩展

- [ ] `engine.rs`：Location 嵌套处理
  - 栈顶已有 ContentBlock → 搜索范围为栈顶 block
  - 匹配结果 push 到 block_stack（缩小范围）
  - Off:Location 时 pop 并写回上一层（当前 `write_back_to_parent` 已支持）
  - 嵌套时 `matched_line_count` 计算相对于子 block

### 4.3 嵌套示例测试

```rust
// //!@Open: ./sample.rs
// //!@Location:
// fn outer() {
//     let a = 1;
//     // 嵌套 Location 定位 inner
//     //!@Location:
//     fn inner() {
//         ...
//     //!@New:
//     let x = 2;
//     //!@Off:Location
//     //!@Off:Location
// //!@Off:Open
```

### 实现建议

1. **SearchScope 抽象**：建议定义 `enum SearchScope<'a> { File(&'a FileContent), Block(&'a ContentBlock) }`，实现统一的 `lines()` 和 `first_line_index()` 访问。
2. **LineNumber 映射**：嵌套 Location 匹配到的是子 ContentBlock 内的相对行号，写回时需要映射到父 ContentBlock 的绝对行号。建议在 `ContentBlock` 中记录 `parent_offset`。
3. **Off 链处理**：当前 `Off:Location` 只 pop 一次，多层嵌套时需要逐层写回。当前 `write_back_to_file` 已用 `while let Some(block) = self.block_stack.pop()` 处理，可复用。
4. **先实现 Phase 3 再实现 Phase 4**：嵌套 Location 的正确性依赖精确的 Block 边界解析，否则子 ContentBlock 可能包含不相关的内容。

### 验证

```bash
cargo test engine::tests::test_nested_location
cargo test engine::tests::test_nested_location_new
```

---

## Phase 5: 行号 Location（`@66,120`）+ Raw 命令

**目标**：按行号直接定位，跳过匹配流程；支持 `...` 字面量写入。

### 5.1 行号 Location

- [ ] parser：识别 `//!@Location:@66,120` 格式，解析行号范围
  - 支持单行：`@66`
  - 支持范围：`@66,120`
  - 验证 start > 0, end >= start
- [ ] `model.rs`：`LineRange { start: usize, end: usize }`
- [ ] engine：行号定位直接按索引截取，不经过 matcher
- [ ] 行号搜索范围：顶层 = FileContent，嵌套 = 当前 ContentBlock（行号是相对于搜索范围的）

### 5.2 Raw 命令

- [ ] lexer：识别 `//!@Raw: ...`
- [ ] `Command` 枚举新增：`Raw { content: String }`
- [ ] parser：Raw Token 出现在 New/Delete 内容块中时融入对应行
- [ ] `NewLine` 和 `DeleteLine` 增加 `is_raw: bool` 字段（字段已存在，值始终为 false）
- [ ] New 插入时：is_raw 行直接写入 content，不计算缩进
- [ ] Delete 匹配时：is_raw 行按字面量匹配

### 实现建议

1. **行号定位优先级**：当 Location 同时包含 `@行号` 和定位内容时，优先使用行号定位（跳过 matcher）。
2. **Raw 与 Separator 冲突**：当前 `...` 在 Lexer 中是全局分隔符，Raw 需要在此之上提供字面量转义。建议在 parser 阶段处理：遇到 Raw token 时，将其后的 Separator token 跳过并融入内容。
3. **行号映射**：嵌套 Location 中的行号定位，行号应相对于当前 ContentBlock 而非整个文件。

### 验证

```bash
cargo test engine::tests::test_line_number_location
cargo test engine::tests::test_raw_ellipsis
```

---

## Phase 6: 错误美化 / 彩色输出 ✅ 基本完成

**目标**：用户体验完好的错误提示和终端输出。

> **当前状态**：核心功能已实现，后续可做细节打磨。

### 6.1 错误信息增强 ✅

- [x] `error.rs`：所有错误类型实现详细的 `Display`
  - 包含上下文代码块
  - 包含行号引用
  - 包含修复建议（中文提示）
- [x] 错误输出格式统一（参考 INSTRUCTION.md 第 5.1 节）

### 6.2 彩色终端输出 ✅

- [x] `output.rs`：封装 `colored` 库
  - 绿色 `+` 标注新增行
  - 红色 `-` 标注删除行
- [x] 检测终端能力：`is_terminal` 为 false 时关闭颜色（管道/重定向）

### 6.3 格式化输出 ✅

- [x] ContentBlock 输出带行号前缀：`L12: content`（`format_diff_lines` 支持）
- [x] 匹配诊断信息：最多展示 3 个候选，超过时显示 `(n more)`（`expect_single_match` 已实现）

### 6.4 日志/详细模式 ✅

- [x] `--verbose` 标志：打印执行过程（词法分析 token 数、语法分析命令数）
- [x] `--quiet` 标志：只输出错误，不输出成功操作的 ContentBlock

### 后续可改进

1. **错误信息带颜色**：当前 `eprintln!` 未着色，可对错误类型关键词加黄色高亮。
2. **错误位置标线**：显示源码片段并在出错行下方加 `^~~~` 指示。
3. **JSON 错误输出**：`--error-format json` 方便 LLM 解析错误。

---

## Phase 7: 扩展命令

**目标**：产品化扩展功能。

### 7.1 Include 命令

- [ ] `//!@Include: ./partial.ned` — 引入另一个脚本文件
- [ ] 支持递归 Include（设置最大深度限制防止循环引用）
- [ ] Include 文件的内容展开后继续解析（相当于内联）

### 7.2 后续扩展（低优先级）

- [ ] Async 命令 + Off:Async：并行处理多个文件
- [ ] 修改可逆转：备份原文件，支持回滚
- [ ] TUI 预览模式：修改前预览 diff

### 实现建议

1. **Include 实现**：建议在 Lexer 层面做展开（读取 Include 文件内容并内联到 Token 流中），这样后续阶段无感知。
2. **循环引用检测**：维护 `HashSet<PathBuf>` 已包含的文件路径，遇到重复直接报错。
3. **Async 实现**：需要将 Engine 重构为可克隆/可并行的状态机，每个 Async 块有独立的 `block_stack` 和 `file`。

---

## 阶段依赖关系

```
Phase 0 ──▶ Phase 1 ──▶ Phase 2 ──▶ Phase 3 ──▶ Phase 4
                   │         │             │
                   │         ▼             │
                   │    Phase 6 ✅         │
                   │         │             │
                   │         ▼             │
                   │    Phase 5 (可并行 Phase 3/4)
                   │                       │
                   └───────────────────────┘
                                      │
                                      ▼
                                 Phase 7
```

- Phase 0 → Phase 1：项目骨架是基础 ✅
- Phase 1 → Phase 2：New/Delete 依赖 ContentBlock ✅
- Phase 2 → Phase 3：Block 指令依赖基础 New/Delete
- Phase 3 → Phase 4：嵌套依赖精确 Block 解析
- Phase 5 可与 Phase 3/4 并行（行号定位不依赖嵌套）
- Phase 6 已基本完成（可在 Phase 2 后持续增强）
- Phase 7 在所有核心功能稳定后开始

---

## 当前测试状态

```
cargo test  → 133 passed, 0 failed
cargo clippy → 0 warnings

测试分布：
  src/lib.rs 单元测试       108 个
  src/main.rs 集成测试       4 个
  tests/integration_test.rs  21 个
```

### 测试覆盖场景

| 场景 | 测试数 | 说明 |
|------|--------|------|
| Lexer 词法分析 | 22 | Token 识别、内容提取、Separator、行号、修饰符 |
| Parser 语法分析 | 22 | Command 构建、diff_taps 计算、歧义检测 |
| Engine 执行引擎 | 27 | Open/Location/New/Delete/Off 全流程、diff_lines |
| Matcher 匹配算法 | 8 | 精确匹配、歧义报错、空行跳过、空 Location |
| Model 数据结构 | 22 | Line/ContentBlock 操作、reindex、文件读写 |
| Error 错误类型 | 8 | Display 格式化、错误信息完整性 |
| Output 输出格式化 | 4 | 彩色/无色、行号、ContentBlock |
| 集成测试 | 21 | 真实文件 New/Delete/Replace、空 Location、歧义检查 |
