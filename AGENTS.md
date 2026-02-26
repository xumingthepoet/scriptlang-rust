# AGENTS

`scriptlang-rs` 是一个 Rust workspace，目标是把 ScriptLang 的解析、编译、运行和宿主接口解耦实现。

## 项目结构（重点）

### 顶层目录
- `crates/`: Rust workspace 主体代码（核心实现都在这里）。
- `examples/`: 可运行的脚本样例与 smoke 场景。
- TypeScript 参考实现仓库：`https://github.com/xumingthepoet/scriptlang`（用于对齐行为，不直接参与 Rust 构建）。
- `Cargo.toml`: workspace 成员与共享依赖声明。
- `Makefile`: 统一质量门禁入口（`make gate`）。

## 主要文档
- `README.md`: 当前主文档，包含项目简介、crate 说明、常用命令与 CLI 示例。

### Workspace Crates
- `crates/sl-core`: 通用类型、值模型、错误、快照数据结构。
- `crates/sl-parser`: XML 解析和 include 信息提取。
- `crates/sl-compiler`: include 图校验、defs/json/script 编译到中间表示（IR）。
- `crates/sl-runtime`: 执行引擎（`next`/`choose`/`submit_input`/`snapshot`/`resume`）。
- `crates/sl-api`: 面向宿主的高层 API（create/compile/resume 等）。
- `crates/sl-cli`: 命令行入口（`agent start/choose/input`）。

### 依赖方向（必须保持）
1. `sl-core` 为最底层，不依赖其他业务 crate。
2. `sl-parser`、`sl-compiler`、`sl-runtime` 只依赖 `sl-core` 和必要三方库。
3. `sl-api` 负责组合 parser/compiler/runtime，不反向渗透实现细节。
4. `sl-cli` 只作为宿主层调用 `sl-api`，不内联核心业务逻辑。

## 开发流程
1. 先确认修改落在哪一层（parser/compiler/runtime/api/cli），避免跨层耦合。
2. 优先复用 `examples` 补充或回归场景。
3. 提交前运行 `make gate`。
4. 只要 `make gate` 通过，可直接提交，无需再次询问。
5. 单元测试必须参考子项目结构组织，按源代码文件一对一建立测试防守。
6. 同一文件内，函数测试顺序必须与源代码中的函数定义顺序一致。
7. 每次代码改动后都要根据变更影响同步更新相关文档（如 `README.md`、设计说明、接口说明等）。

## 质量门禁
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`
- `cargo llvm-cov --workspace --exclude sl-cli --all-features --all-targets --summary-only --fail-under-lines 100`
- `make gate`

## 完成定义（DoD）
- 变更位于正确分层，未破坏 crate 边界。
- 相关示例/测试已覆盖新增或修复行为，且满足文件级一对一防守。
- 覆盖率门禁：除 `sl-cli` 外其余 crate 需达到 `100%`（由 `make coverage` 强制）。
- `make gate` 通过。
