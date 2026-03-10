# `sl-lint` 使用指南

`sl-lint` 是独立于 `sl-cli` 的 ScriptLang 代码质量检查工具。

## 1. 运行方式

```bash
cargo run -p sl-lint -- \
  --scripts-dir crates/sl-test-example/examples/14-module-functions
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
