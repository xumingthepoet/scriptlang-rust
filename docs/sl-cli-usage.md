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
- `--entry-script <name>`：入口脚本，默认 `main`
- `--state-out <path>`：状态输出文件（必填）

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
- `--entry-script <name>`：入口脚本，默认 `main`
- `--step <action>`：可重复，按出现顺序消费

`--step` 语法：
- `choose:<index>`（例：`choose:0`）
- `input:<text>`（例：`input:Rin`，`text` 可为空）

---

## 3. Agent 输出格式

### 3.1 `start/choose/input` 输出（机器可读）

- `RESULT:OK|ERROR`
- `EVENT:CHOICES|INPUT|END`
- `TEXT_JSON:...`
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
- `--entry-script <name>`：入口脚本，默认 `main`
- `--state-file <path>`：状态文件，默认 `.scriptlang/save.json`

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
