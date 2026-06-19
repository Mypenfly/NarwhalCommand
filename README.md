# Narwhal Command Script

**实验性命令脚本系统。** 将 `"系统一切皆是文本"` 推向极致——用统一的字符级语义语法操控整个系统，从文件编辑到 Shell 命令，从数据管道到外部工具。

> **实验性声明**: 本项目是一个激进的系统交互范式实验。语法设计、命令模型、数据流动方式均为探索性质，随时可能发生破坏性变更。被实验者是：一个 LLM 能否通过归一化的文本命令语法，以最低学习成本操控任意系统资源？

## 核心理念

```
传统:  Shell脚本 + sed/awk + Python胶水 + Makefile = 碎片化工具链
NCS:   !@Open → !@Location → !@Delete → !@New → @/Open = 统一语义流
```

**一条语法，所有操作。** 不是取代 Bash，而是把 Bash、编辑器、包管理器、部署工具全部纳入同一个语义化命令框架中——脚本即是文档，文档即是执行。

## 仓库结构

```
NarwhalCommand/
├── n_edit/          # 精确代码编辑引擎（稳定）
│   ├── src/         # 基于 //!@ 注解的语义级文件修改工具
│   └── tests/       # 297 个测试
├── ncs/             # Narwhal Command Script
│   ├── src/         # 通用命令脚本系统（!@Cmd 语法）
│   │   ├── commands/  # 12 个命令实现
│   │   └── engine/    # 引擎核心 + command_pipeline + executor
│   └── tests/       # 322 单元测试 + 70 集成测试
└── docs/            # 设计文档和开发指南
```

## n_edit — 精确代码编辑

使用 `//!@` 前缀的 `.ned` 脚本，通过**去空白内容 + 缩进差异**双重匹配实现格式感知的源码修改。297 个测试，全部通过。

```bash
cargo run -p n_edit -- script.ned
```

## ncs — 命令脚本系统

从 n_edit 扩展为**全系统可操作的命令脚本**。关键变化：

| 方面 | n_edit | ncs |
|------|--------|-----|
| 命令前缀 | `//!@Cmd:` | `!@Cmd` |
| 关闭符号 | `//!@Off:Cmd` | `@/Cmd` |
| 执行模型 | 单文件状态机 | 命令注册表 + exec_cmds + pools |
| 数据传递 | 隐式栈 | 显式 CmdContent convert/execute_core/out |
| 扩展能力 | 无 | Include 动态注册外部命令 |

```bash
cargo run -p ncs -- script.ncs
```

**语法快速参考**：见 [docs/ncs_syntax.md](docs/ncs_syntax.md)

## 设计哲学

### 字符级精度
一切操作基于字符——不依赖 AST、不用正则魔法。你要什么，就写什么。

### 语义即命令
`!@Open` 读文件，`!@Location` 定位，`!@Delete` 删除，`!@New` 写入。命令名就是英语单词，语法就是自然语言的排列组合。

### 管道即数据
命令间通过 `CmdContent` 传递数据。`Capture` 存入全局池，`Get` 取出复用。

### 错误即指南
每个错误附带 `title`（标题）、`detail`（详情）、`hints`（修复建议）。不是报错，是指路。

## 命令概览（ncs）

| 类型 | 命令 | 状态 | 说明 |
|------|------|:----:|------|
| **文件编辑** | `Open` | ✅ | 打开文件/目录。Dir 模式序列化为树形文本，支持 Location/New/Delete 操作目录结构 |
| | `Location` | ✅ | 去空白 + diff_taps 语义匹配定位（Normal/Block） |
| | `New` | ✅ | Normal/Start/End 模式插入 |
| | `Delete` | ✅ | Normal/Block 模式删除 |
| | `Raw` | ✅ | 仅展开，字面量融入父命令 |
| **系统操作** | `Bash` | ✅ | 执行 bash 命令（安全审查：sudo/rm -rf/chmod 777/mkfs/dd/forkbomb 拦截） |
| | `Exec` | ✅ | `script -c` 直连终端执行，支持彩色/交互输出 |
| | `Read` | ✅ | 读取文件（start/end）+ syntect 高亮（base16-ocean.dark）；Dir 模式树形结构 |
| | `Write` | ✅ | Normal 模式写文件（自动创建父目录）；Raw 模式全量原样写入至 EOF |
| **元命令** | `Include` | ✅ | 动态导入外部命令（ExecMethod: Default/Bash/Script） |
| | `WorkPath` | ✅ | 设置工作路径（基于脚本父目录），影响后续 `./`、`../` 展开 |
| | `Get` | ✅ | 从 pools 提取数据；`like` 伪装模式和块内展开待 Phase 5 |

## 实现阶段

```
Phase 0-2: 基础设施 + 核心命令 (Open/Location/New/Delete/Raw)  ✅
Phase 3:   CmdContent 数据流 + 变更追踪                          ✅
Phase 4:   独立命令 (Bash/Exec/Read/Write/Include/WorkPath/Get)  ✅
           + Open Dir 树形文本模式                                ✅
Phase 5:   Get 高级特性 (like 伪装 + 块内展开)                    ⚠️
Phase 6:   错误处理打磨 + 终端输出完善                            ⚠️
```

## 开发

```bash
cargo build                     # 构建
cargo test --workspace          # 全量测试 (693 tests)
cargo test -p ncs               # ncs 测试 (392 tests)
cargo clippy --workspace -- -D warnings
cargo fmt --check
cargo run -p ncs -- script.ncs --verbose
```

## 文档索引

| 文档 | 内容 |
|------|------|
| [docs/ncs_syntax.md](docs/ncs_syntax.md) | NCS 脚本语法参考手册 |
| [docs/INSTRUCTION.md](docs/INSTRUCTION.md) | 总体设计路径、数据结构、算法流程、代码规范 |
| [docs/ncs_dev.md](docs/ncs_dev.md) | NCS 命令定义、执行流、错误体系 |
| [docs/phases.md](docs/phases.md) | 实现阶段拆分和进度追踪 |
| [docs/n_edit_dev.md](docs/n_edit_dev.md) | n_edit 语法设计和命令机制 |
