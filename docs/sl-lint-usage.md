# `sl-lint` 使用指南

`sl-lint` 是独立于 `sl-cli` 的 ScriptLang 代码质量检查工具。

## 1. 运行方式

```bash
cargo run -p sl-lint -- \
  --scripts-dir crates/sl-test-example/examples/14-module-functions
```

针对 `example-project`：

```bash
cargo run -p sl-lint -- \
  --scripts-dir example-project \
  --entry-script app.main
```

可选参数：
- `--entry-script <name>`：入口脚本名，默认 `main.main`

## 2. 输出格式

每条诊断：
- `[warning] <code> <file>:<line>:<column> <message>`
- 可选 `help: <suggestion>`

末尾汇总：
- `<errors> errors, <warnings> warnings`

退出码：
- 存在任意 warning/error 返回 `1`
- 无诊断返回 `0`

## 3. 当前规则

- `unused-script`
- `unused-module`
- `unused-function`
- `unused-module-var`
- `unused-module-const`
- `unused-local-var`
- `unused-param`
- `prefer-short-name`
- `unused-import`
- `unreachable-node`

## 4. 引用识别范围

- `unused-import` 会把以下用法视为“已使用”：
  - 脚本/表达式中的跨模块符号引用（函数、模块变量、模块常量、脚本字面量）。
  - 注释 alias 指令（如 `<!-- alias ids.LocationId -->`）里的目标模块。
- `unused-module` 会把 import/alias 的实际使用记录视为模块已使用，不会对这类模块重复报未使用。
- `unused-script` 除了静态 `call/goto` 可达链，还会识别表达式和函数体里的 `@module.script` / `@short` 字面量引用。
- `unused-script` / `unused-function` 不会因为 `export` 自动豁免；未被可达链或表达式调用使用时仍会告警。
- `unused-module-var` / `unused-module-const` 不会因为 `export` 自动豁免；只要未被读取就会告警。
- 函数体（`<function>...</function>`）中的 ScriptLang 表达式也会参与引用分析，不再只分析 `<script>` 节点。
- `prefer-short-name` 会在“同一 root module 可直接访问短名”时提示冗余全限定前缀，例如：
  - `root.child.fn()` -> 建议 `child.fn()`
  - `m.values` -> 建议 `values`
