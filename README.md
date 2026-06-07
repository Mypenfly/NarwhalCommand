# N_Edit

基于注释的代码编辑工具，使用语义级语法通过 `.ned` 脚本精确修改源代码文件。

## 核心理念

- **语义级命令** — `Open`、`Location`、`New`、`Delete`、`Off`，降低 LLM 和人类的学习成本
- **精确匹配** — 保留格式、忽略空白差异的逐行逐字符匹配
- **安全操作** — 执行失败不写回文件，所有修改在内存中进行
- **友好报错** — 匹配歧义时给出候选上下文和建议

## 快速开始

```bash
cargo build --release
./target/release/n_edit script.ned
```

## .ned 脚本格式

脚本使用 `//!@` 注释前缀的命令，内容提取直到 `...` 或下一个命令。

```ned
//!@Open: path/to/file.rs
//!@Location:
fn main() {
    old_code();
...
//!@Delete:
    old_code();
...
//!@New:
    new_code();
...
//!@Off:Open
```

## 命令列表

| 命令 | 说明 |
|------|------|
| `Open:` | 打开目标文件 |
| `Location:` | 定位代码位置（支持嵌套） |
| `Location:Block` | 定位完整代码块 |
| `New:` | 在定位位置后插入内容 |
| `New:Start` | 在文件开头插入 |
| `New:End` | 在文件末尾追加 |
| `Delete:` | 删除匹配的连续行 |
| `Delete:Block` | 删除整个代码块 |
| `Off:Open` | 写回文件 |
| `Off:Location` | 退出当前定位作用域 |
| `Off:New` | 退出插入作用域 |

## CLI 选项

```
n_edit <script_path> [-v | --verbose] [-q | --quiet]
```

## 开发

```bash
# 构建
cargo build

# 测试
cargo test

# 格式化 / Lint
cargo fmt --check
cargo clippy -- -D warnings

# Nix 开发环境
nix develop
```

## 项目结构

```
src/       -- 核心模块（lexer → parser → engine 流水线）
tests/     -- 集成测试脚本和数据文件
docs/      -- 详细设计文档
```

详见 [AGENTS.md](AGENTS.md) 和 [docs/INSTRUCTION.md](docs/INSTRUCTION.md)。
