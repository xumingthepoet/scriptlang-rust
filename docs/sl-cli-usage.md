# `sl-cli` 使用指南

本文档说明 `scriptlang-rs` 中 `sl-cli` 的主要功能、参数和典型使用流程。

## 1. 入口与模式

统一入口：

```bash
cargo run -p sl-cli -- <mode> ...
```

当前支持两个 mode：
- `agent`: 面向脚本化调用/自动化测试
- `tui`: 面向人工交互调试（全屏 TUI，必要时自动降级到行模式）

查看帮助：

```bash
cargo run -p sl-cli -- --help
cargo run -p sl-cli -- agent --help
cargo run -p sl-cli -- agent replay --help
cargo run -p sl-cli -- tui --help
```

---

## 2. Agent 模式

`agent` 提供四个子命令：
- `start`
- `choose`
- `input`
- `replay`

### 2.1 `agent start`

从脚本目录启动新会话，运行到第一个边界（`CHOICES/INPUT/END`），并在需要时保存状态。

```bash
cargo run -p sl-cli -- agent start \
  --scripts-dir crates/sl-test-example/examples/06-snapshot-flow \
  --state-out /tmp/sl-state.json
```

参数：
- `--scripts-dir <path>`：脚本目录（必填）
- `--entry-script <name>`：入口脚本，默认 `main.main`
- `--state-out <path>`：状态输出文件（必填）
- `--rand <csv>`：可选随机序列（例如 `12,3,1,4`）
- `--show-debug`：显示 `<debug>` 输出（默认隐藏）

### 2.2 `agent choose`

从已有状态恢复，提交一个 choice 索引，再继续运行到下一个边界。

```bash
cargo run -p sl-cli -- agent choose \
  --state-in /tmp/sl-state.json \
  --choice 0 \
  --state-out /tmp/sl-next.json
```

参数：
- `--state-in <path>`：输入状态文件（必填）
- `--choice <index>`：选择索引（必填）
- `--state-out <path>`：新状态输出文件（必填）
- `--rand <csv>`：可选随机序列覆盖（命令行优先于 state）
- `--show-debug`：显示 `<debug>` 输出（默认隐藏）

### 2.3 `agent input`

从已有状态恢复，提交一个输入文本，再继续运行到下一个边界。

```bash
cargo run -p sl-cli -- agent input \
  --state-in /tmp/sl-next.json \
  --text "Rin" \
  --state-out /tmp/sl-next2.json
```

参数：
- `--state-in <path>`：输入状态文件（必填）
- `--text <text>`：输入文本（必填）
- `--state-out <path>`：新状态输出文件（必填）
- `--rand <csv>`：可选随机序列覆盖（命令行优先于 state）
- `--show-debug`：显示 `<debug>` 输出（默认隐藏）

### 2.4 `agent replay`

从新引擎开始，按顺序消费动作队列（`--step`），自动输出完整事件流。  
当动作耗尽后，命令会继续运行到下一个边界（`CHOICES/INPUT/END`）再停止并返回成功。

```bash
cargo run -p sl-cli -- agent replay \
  --scripts-dir crates/sl-test-example/examples/16-input-name \
  --step input:Rin
```

```bash
cargo run -p sl-cli -- agent replay \
  --scripts-dir crates/sl-test-example/examples/06-snapshot-flow \
  --entry-script main \
  --step choose:0
```

```bash
cargo run -p sl-cli -- agent replay \
  --scripts-dir crates/sl-test-example/examples/07-battle-duel \
  --step choose:0 \
  --step choose:1 \
  --step input:Rin
```

参数：
- `--scripts-dir <path>`：脚本目录（必填）
- `--entry-script <name>`：入口脚本，默认 `main.main`
- `--step <action>`：可重复，按出现顺序消费
- `--rand <csv>`：可选随机序列（例如 `12,3,1,4`）
- `--show-debug`：显示 `<debug>` 输出（默认隐藏）

`--step` 语法：
- `choose:<index>`（例：`choose:0`）
- `input:<text>`（例：`input:Rin`，`text` 可为空）

`--rand` 语义：
- 传入后会覆盖脚本中的 `random(n)` 输出。
- 按序列依次返回 `value % n`。
- 序列耗尽后固定返回 `0`。

### 2.5 `agent compile`

编译脚本并输出 artifact JSON 文件。支持 `--dry-run` 模式用于排查编译错误。

```bash
# Dry-run 模式：只编译不写入，用于调试编译错误
cargo run -p sl-cli -- agent compile \
  --scripts-dir crates/sl-test-example/examples/01-text-code \
  --dry-run
```

```bash
# 正常编译：输出 artifact JSON 文件
cargo run -p sl-cli -- agent compile \
  --scripts-dir crates/sl-test-example/examples/01-text-code \
  -o /tmp/artifact.json
```

参数：
- `--scripts-dir <path>`：脚本目录（必填）
- `--entry-script <name>`：入口脚本，默认 `main.main`
- `-o, --output <path>`：输出文件路径（非 dry-run 必填）
- `--dry-run`：仅在内存中编译，不写入文件
- `--rand <csv>`：可选随机序列（compile 命令中未使用，为保持一致性）

---

## 3. Agent 输出格式

### 3.1 `start/choose/input` 输出（机器可读）

- `RESULT:OK|ERROR`
- `EVENT:CHOICES|INPUT|END`
- `TEXT_JSON:...`
- `TEXT_TAG_JSON:...`（可选；仅当对应 `TEXT_JSON` 来自 `<text tag="...">` 时输出）
- `DEBUG_JSON:...`（可选；仅 `--show-debug` 时输出）
- `PROMPT_JSON:...`
- `CHOICE:<index>|<json_text>`
- `INPUT_DEFAULT_JSON:...`
- `STATE_OUT:<path|NONE>`
- `ERROR_CODE:...`（仅 `RESULT:ERROR`）
- `ERROR_MSG_JSON:...`（仅 `RESULT:ERROR`）

### 3.2 `replay` 输出（人类可读）

- `RESULT:OK`
- `MODE:REPLAY`
- `TEXT: ...`
- `DEBUG: ...`（仅 `--show-debug`）
- `CHOICES: ...` 后跟 `- [index] text`
- `INPUT: ...`
- `DEFAULT: ...`
- `APPLY: choose:...` / `APPLY: input:...`
- `END`
- `ACTIONS_USED: ...`
- `ACTIONS_TOTAL: ...`
- `STOP_AT: CHOICES|INPUT|END`

错误时仍沿用统一错误输出：
- `RESULT:ERROR`
- `ERROR_CODE:...`
- `ERROR_MSG_JSON:...`

---

## 4. TUI 模式

启动方式：

```bash
cargo run -p sl-cli -- tui --scripts-dir crates/sl-test-example/examples/06-snapshot-flow
```

参数：
- `--scripts-dir <path>`：脚本目录（必填）
- `--entry-script <name>`：入口脚本，默认 `main.main`
- `--state-file <path>`：状态文件，默认 `.scriptlang/save.json`
- `--rand <csv>`：可选随机序列（例如 `12,3,1,4`）
- `--show-debug`：显示 `<debug>` 输出（默认隐藏）

全屏模式快捷键：
- `Up/Down`：选择选项
- 输入文字 + `Backspace`：编辑输入框
- `Enter`：提交 choice/input
- `s`：保存
- `l`：加载
- `r`：重开
- `h`：帮助
- `q` / `Esc`：退出

当 stdin/stdout 不是 TTY，或在测试环境下，会自动降级到行模式。  
行模式命令：
- `:help`
- `:save`
- `:load`
- `:restart`
- `:quit`

---

## 5. 常见工作流

### 5.1 自动化脚本流程（状态驱动）
1. `agent start` 获取首个边界并落盘状态
2. 根据边界调用 `agent choose` 或 `agent input`
3. 重复直到 `EVENT:END`

### 5.2 快速回放流程（队列驱动）
1. 写一串 `--step`
2. 执行 `agent replay`
3. 直接查看完整事件流和 `STOP_AT`

### 5.3 人工调试流程
1. 直接进入 `tui`
2. 通过快捷键交互
3. 用 `save/load` 做中断恢复测试

### 5.4 推荐验证闭环（避免“仅编译通过”）
1. 先执行 `agent compile --dry-run`，尽早发现 include/类型/XML 语法问题
2. 再执行 `agent replay --rand "<固定序列>" --step ...`，覆盖真实运行路径
3. 将同一组 `--rand` 与 `--step` 固化到 CI/回归脚本，避免随机路径漏测

---

## 6. 高频报错与修复建议

1. `XML_PARSE_ERROR ... invalid name token`
- 常见原因：属性值里直接写了 `<` 或 `&&`
- 修复：改用 ScriptLang 保留字，写成 `LT` / `LTE` / `AND`

2. `TYPE_UNKNOWN: Unknown custom type "game.WorldState"`
- 常见原因：脚本所在模块没有 import 对应的 `*.xml` 定义文件
- 修复：在每个使用该类型的模块文件里显式 `<!-- import ... from ... -->`

3. `when` 字符串比较表达式异常
- 常见原因：属性表达式里继续使用 XML 转义或双引号字符串
- 修复：属性里写单引号字符串，例如 `name == 'Rin'`

4. `<code>` / `<function>` 里的字符串异常
- 常见原因：代码块里写了单引号字符串，触发 ScriptLang 的 char/单引号禁用规则
- 修复：代码块里改成双引号，例如 `<code>name = "Rin";</code>`

4. `Data type incorrect: f64 (expecting i64)`
- 归类：运行时类型稳定性 bug（不是脚本作者的预期行为）
- 触发链路（典型）：`ref:int` 跨脚本传递并在 `<code>` 更新后，再用于数组下标
- 建议：优先升级到包含修复的版本；如仍复现，请附最小复现脚本反馈给开发者
