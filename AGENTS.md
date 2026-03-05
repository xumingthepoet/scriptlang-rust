# AGENTS

`scriptlang-rs` 是一个 Rust workspace，目标是把 ScriptLang 的解析、编译、运行和宿主接口解耦实现。

## 项目结构（重点）

### 顶层目录
- `crates/`: Rust workspace 主体代码（核心实现都在这里）。
- `crates/sl-test-example/examples/`: 可运行的脚本样例与 smoke 场景。
- `Cargo.toml`: workspace 成员与共享依赖声明。
- `Makefile`: 统一质量门禁入口（`make gate`）。

## 主要文档
- `README.md`: 当前主文档，包含项目简介、crate 说明、常用命令与 CLI 示例。
- `KNOWLEDGE.md`: 帮助后续开发的长期知识记忆。

### Workspace Crates
- `crates/sl-core`: 通用类型、值模型、错误、快照数据结构。
- `crates/sl-parser`: XML 解析和 include 信息提取。
- `crates/sl-compiler`: include 图校验、defs/json/script 编译到中间表示（IR）。
- `crates/sl-runtime`: 执行引擎（`next`/`choose`/`submit_input`/`snapshot`/`resume`）。
- `crates/sl-api`: 面向宿主的高层 API（create/compile/resume 等）。
- `crates/sl-cli`: 命令行入口（`agent start/choose/input`）。

### 依赖方向（必须保持）
1. `sl-core` 为最底层，不依赖其他业务 crate。
2. `sl-parser` 依赖 `sl-core` 和必要三方库。
3. `sl-compiler` 可依赖 `sl-parser`、`sl-core` 和必要三方库。
4. `sl-runtime` 只依赖 `sl-core` 和必要三方库。
5. `sl-api` 负责组合 compiler/runtime，不反向渗透实现细节。
6. `sl-cli` 只作为宿主层调用 `sl-api`，不内联核心业务逻辑。

### 对外公开面（必须保持）
- 对宿主/用户推荐且稳定的入口只有：
  - `sl-api`（库）
  - `sl-cli`（命令行）
- 其余 crate（`sl-core/sl-parser/sl-compiler/sl-runtime/sl-test-example`）属于内部实现细节，不作为直接集成入口。

## 开发流程
1. 先确认修改落在哪一层（parser/compiler/runtime/api/cli），避免跨层耦合。
2. 优先复用 `crates/sl-test-example/examples` 补充或回归场景。
3. 提交前运行 `make gate`。
4. 只要 `make gate` 通过，可直接提交，无需再次询问。
5. 单元测试必须与被测源文件写在同一个文件内，不允许拆到独立测试文件。
6. 同一文件内，函数测试顺序必须与源代码中的函数定义顺序一致。
7. 每次代码改动后都要根据变更影响同步更新相关文档（如 `README.md`、设计说明、接口说明等）。
8. 仅当发现“可复用、可执行、可避免未来重复踩坑”的长期知识时，再更新 `KNOWLEDGE.md`（不要把每次改动流水账写进去）。

## 完成定义（DoD）
- 变更位于正确分层，未破坏 crate 边界。
- 相关示例/测试已覆盖新增或修复行为，且满足文件级一对一防守。
- `make gate` 通过（覆盖率必须达到 `100%`）。

## Knowledge logging (Ralph-style)

- 本项目长期知识记忆在：`KNOWLEDGE.md`。
- 原则参考：<https://ghuntley.com/ralph/>（优先沉淀可复用决策知识，而非提交日志）。
- 记录标准（全部满足才记录）：
  - 跨任务仍会复用（不是一次性上下文）。
  - 能改变后续实现/排障决策（有明确行动价值）。
  - 包含“为什么/约束/失败模式/验证方式”之一，不只是“改了什么”。
- 不记录：
  - 单次提交摘要、显而易见代码事实、可从 `git diff` 直接读出的内容。
  - 临时调试痕迹、无复用价值的命令输出。
- 允许修订旧知识：当旧记录不再准确时，优先直接更新对应条目；若需保留历史，再补“更正”。
