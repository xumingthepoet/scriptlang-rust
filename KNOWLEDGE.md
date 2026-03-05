# Project Knowledge (append-only)

## 规则
- 发现任何有用知识就 **立刻追加** 到下面的 Log 底部
- **不要改旧记录**；需要纠正就写一条新的记录

## 格式
### YYYY-MM-DD — 类型 — 标题
- 发现：一句话说明学到了什么
- 细节：关键命令/文件路径/注意事项
- 证据：可选（报错信息/链接/输出片段），精简记录

## Log（只在最下面追加）
### 2026-03-06 — 协议/模型 — `<text tag>` 透传链路约定
- 发现：`<text>` 的 `tag` 适合定义为可选元数据，从 compiler IR 一路透传到 runtime/api/cli，而不是在引擎内置行为。
- 细节：`sl-core::ScriptNode::Text` 与 `sl-core::EngineOutput::Text` 都新增 `tag: Option<String>`；`sl-cli` 机器输出在 `TEXT_JSON` 后按需追加 `TEXT_TAG_JSON`，保持旧解析器兼容。
- 证据：实现点在 `crates/sl-compiler/src/script_compile.rs`、`crates/sl-runtime/src/engine/step.rs`、`crates/sl-cli/src/boundary_runner.rs`。
