# AGENTS

`scriptlang-rs` 是一个 Rust workspace，目标是把 ScriptLang 的解析、编译、运行和宿主接口解耦实现。

## 项目结构（重点）

### 顶层目录
- `crates/`: Rust workspace 主体代码（核心实现都在这里）。
- `crates/sl-test-example/examples/`: 可运行的脚本样例与 smoke 场景。
- `Cargo.toml`: workspace 成员与共享依赖声明。
- `Makefile`: 统一质量门禁入口（`make gate`）。

## 主要文档
- [README.md](README.md): 当前主文档，包含项目简介、crate 说明、常用命令与 CLI 示例。
- [KNOWLEDGE.md](KNOWLEDGE.md): 帮助后续开发的长期知识记忆。

### 文档正交原则（必须保持）
- `README.md` 只做导航与分工说明，不承载语法/API/CLI 细节规则。
- 用户不会阅读 `AGENTS.md` / `KNOWLEDGE.md`；所有指导 agent 协作、流程、约束的内容只能写在这两处，不能暴露到 `README.md` 或 `docs/*`。
- 语法细节只放在 `docs/scriptlang-syntax.md`。
- Rust API / artifact / snapshot 契约只放在 `docs/sl-engine-api.md`。
- CLI 参数与输出协议按工具拆分到各自主文档：
  - `sl-cli` 只放在 `docs/sl-cli-usage.md`
  - `sl-lint` 只放在 `docs/sl-lint-usage.md`
- 同一规则只能有一个“主文档”；其他文档只允许链接，不做重复定义。
- 若行为规则发生变更，先更新对应“主文档”；非主文档只保留链接，不复制规则正文。
- 写文档时优先使用白名单表述（明确“支持什么”），避免面向历史格式的黑名单叙述。
- 文档链接结构必须是有向无环图（DAG），默认主干为：
  - `AGENTS.md -> README.md -> docs/*`
  - 子文档禁止回链到上游文档（不回指 `README.md`/`AGENTS.md`）。

### Workspace Crates
- `crates/sl-core`: 通用类型、值模型、错误、快照数据结构。
- `crates/sl-parser`: XML 解析和 import 信息提取。
- `crates/sl-compiler`: import 图校验、module/json/script 编译到中间表示（IR）。
- `crates/sl-runtime`: 执行引擎（`next`/`choose`/`submit_input`/`snapshot`/`resume`）。
- `crates/sl-api`: 面向宿主的高层 API（create/compile/resume 等）。
- `crates/sl-cli`: 命令行入口（`agent start/choose/input/replay/compile`）。
- `crates/sl-lint`: 独立 lint 工具（静态质量检查）。

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
  - `sl-lint`（命令行）
- 其余 crate（`sl-core/sl-parser/sl-compiler/sl-runtime/sl-test-example`）属于内部实现细节，不作为直接集成入口。

## 版本兼容策略（强约束）
- 当前阶段默认**不考虑旧版本兼容**，包括实现代码、重构方案与 Plan/设计文档。
- 新需求若与旧行为冲突，直接以新行为为准，并**完整删除旧实现**（包括旧分支、旧入口、旧测试、旧文档中的旧规则描述），不保留“兼容桥接层”或“过渡开关”。
- 针对语法/属性替换，不需要刻意保留“旧属性报错”的实现代码或测试；直接移除旧属性支持与相关断言，统一以当前语法为唯一规则。
- 禁止在代码、注释、提交说明、Plan 文档中使用 `V1/V2`、`新老并存`、`双轨兼容` 这类过渡性表述；直接描述“当前唯一生效方案”。
- 若用户明确要求保留兼容层，才可例外；未明确要求时一律按“删除旧实现”执行。

## 开发流程
1. 先确认修改落在哪一层（parser/compiler/runtime/api/cli），避免跨层耦合。
2. 每次修复 bug，都必须在 `crates/sl-test-example/examples` 增加或更新可运行的回归案例；回归验证以该案例为准。
3. 提交前运行 `make gate`。
4. 只要 `make gate` 通过，可直接提交，无需再次询问。
5. 修复 bug 时不要求新增单元测试（无需在被测 crate 的单元测试中补用例）。
6. 若任务明确要求补单元测试，单元测试必须与被测源文件写在同一个文件内，且同一文件内测试顺序需与源代码函数定义顺序一致。
7. 每次代码改动后都要根据变更影响同步更新相关文档（如 `README.md`、设计说明、接口说明等）；`scripts/*` 归类为工具脚本，不在 `AGENTS.md` 之外的文档中谈及。
8. 仅当发现“可复用、可执行、可避免未来重复踩坑”的长期知识时，再更新 `KNOWLEDGE.md`（不要把每次改动流水账写进去）。
9. 新增或改造示例时，优先做“组合覆盖”示例（一次体现多个已支持能力），并鼓励使用贴近真实流程的相对复杂案例，不只给最小片段。
10. 需求涉及语义替换时，直接替换并清理旧实现，不做兼容保留；相关计划与文档只写最终方案，不写分代迁移话术。

## 完成定义（DoD）
- 变更位于正确分层，未破坏 crate 边界。
- 修复 bug 时，`crates/sl-test-example/examples` 中已有对应可运行回归案例覆盖该行为。
- 若任务明确要求补单元测试，则需满足“同文件、同顺序”约束。
- `make gate` 通过（覆盖率必须达到 `99.50%`）。

## Knowledge logging (Ralph-style)

- 本项目长期知识记忆在：`KNOWLEDGE.md`。
- 原则参考：<https://ghuntley.com/ralph/>（优先沉淀可复用决策知识，而非提交日志）。
- 记录标准（全部满足才记录）：
  - 跨任务仍会复用（不是一次性上下文）。
  - 能改变后续实现/排障决策（有明确行动价值）。
  - 包含“为什么/约束/失败模式/验证方式”之一，不只是“改了什么”。
  - 必须能回答“未来 agent 具体在哪个文件/哪类改动上会用到这条知识”。
- 不记录：
  - 单次提交摘要、显而易见代码事实、可从 `git diff` 直接读出的内容。
  - 临时调试痕迹、无复用价值的命令输出。
  - 某个功能“这次是怎么开发的”“这次怎么使用”的说明（应写入 `README.md` 或 `docs/`）。
- 允许修订旧知识：当旧记录不再准确时，优先直接更新对应条目；若需保留历史，再补“更正”。

### `KNOWLEDGE.md` 内容边界（强约束）
- 应该写：
  - 某类文件/模块的长期规范（例如“改某文件前后必须满足什么约束”）。
  - 容易重复踩坑的失败模式，以及可执行护栏。
  - 会影响后续实现路径的架构决策。
- 不应该写：
  - 某次需求功能的实现过程、上线过程、使用教程。
  - “本次提交做了什么”的流水账。

### 记录示例（该写）
- 失败模式 + 护栏：
  - 例：给 parser/compiler/runtime 加字段后，先跑 `cargo check` 再跑 `make gate`，否则会在测试辅助匹配分支上遗漏（如 `EngineOutput` 的匹配臂）。
- 架构决策 + 触发条件：
  - 例：类似 `<text tag>` 的扩展需求，默认在核心层“元数据透传”，具体行为留给宿主层；除非需求明确要求内置行为。
- 可执行约束：
  - 例：涉及分层改动时，`sl-cli` 只编排 `sl-api`，不要把 runtime/compiler 业务逻辑下沉进 CLI。

### 记录示例（不该写）
- “这次改了 20 个文件，新增了 tag 字段。”
- “今天跑了 make gate，全绿。”
- “提交信息是 xxx。”
- “README/docs 已更新。”
- “新增了某功能，开发步骤是 A→B→C，使用方式是 D。”
