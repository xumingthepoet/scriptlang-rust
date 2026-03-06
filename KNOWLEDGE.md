# Project Knowledge

## 规则
- 仅记录长期可复用知识：能影响未来实现/排障决策，且不属于提交流水账
- 优先写“为什么 + 约束 + 失败模式 + 如何验证”，不是只写“改了什么”
- 旧知识过时时允许直接修订；若需要保留历史，再补“更正”条目

## 格式
### YYYY-MM-DD — 类型 — 标题
- 发现：一句话说明学到了什么
- 细节：关键命令/文件路径/注意事项
- 证据：可选（报错信息/链接/输出片段），精简记录

## 示例
### 好示例（应记录）
### 2026-03-06 — 护栏 — 字段扩展后的验证顺序
- 发现：给公共输出模型新增字段时，先 `cargo check` 再 `make gate`，能更快暴露匹配分支遗漏。
- 细节：优先检查 runtime 测试辅助函数中的 `match` 分支是否覆盖新字段。
- 证据：常见报错为 “pattern does not mention field ...”。

### 反例（不应记录）
- “这次改了哪些文件/提交了什么 commit”
- “今天跑了什么命令且成功”
- “把 README 改了”

## Log（只在最下面追加）
### 2026-03-06 — 失败模式 — `CARGO_MANIFEST_DIR` 路径归一化
- 发现：在 workspace 维度执行 `cargo test --workspace --all-targets --all-features` 时，测试内直接用 `env!("CARGO_MANIFEST_DIR")` 拼路径可能不稳定。
- 细节：涉及跨目录断言（如 workspace 根路径、examples 目录）时，先把 manifest 目录归一化成绝对路径，再做 `join("..")` 等计算；否则会出现 `exists()/is_dir()` 偶发失败。
- 证据：`sl-test-example` 的 `workspace_root/examples_root` 断言在单包测试可通过，但在 workspace 全量测试下失败。

### 2026-03-06 — 失败模式 — Rhai 数值桥接需保留 `int` 语义
- 发现：把 `SlValue::Number` 无差别转成 Rhai `FLOAT` 会破坏数组下标等 `INT` 语义，导致运行时报 `Data type incorrect: f64 (expecting i64)`。
- 细节：向 Rhai scope 注入变量时应带类型上下文；声明为 `int` 的值（含对象/数组内对应字段）需转成 Rhai `INT`，不能仅靠运行后类型检查兜底。
- 证据：`ref:int` 跨脚本更新后用于 `arr[idx]` 的路径在修复前可稳定复现，修复后由回归测试覆盖。

### 2026-03-06 — 架构 — `defs` 视为无脚本的 `module`
- 发现：新增 `*.module.xml` 时，最稳妥的做法是把 `*.defs.xml` 视作“不能声明 `<script>` 的 module 兼容层”，而不是再引入一套平行声明模型。
- 细节：类型、函数、全局变量都继续走同一套命名空间与 include-closure 可见性逻辑；新增能力只落在 module 脚本注册为 `module.script`，以及同 module 内允许短名脚本跳转。
- 证据：这样可以直接复用 defs global 的 snapshot/resume 与短名冲突规则，避免 runtime 再维护第二套全局状态。
