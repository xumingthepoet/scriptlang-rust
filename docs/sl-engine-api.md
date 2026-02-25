# SL Engine API 使用文档（scriptlang-rs）

本文档面向宿主开发者，说明如何通过 `sl-api` / `sl-runtime` 在 Rust 中完成 ScriptLang 的编译、执行、存档和读档。

## 1. 分层与推荐入口

- 推荐入口：`crates/sl-api`
  - 负责 `xml -> compile -> engine start/resume` 的一站式流程。
- 底层入口：`crates/sl-runtime`
  - 直接操作 `ScriptLangEngine`，适合自定义集成。

建议优先使用 `sl-api`，只有在需要更细粒度控制时再直接依赖 `sl-runtime`。

## 2. 核心数据模型

### 2.1 输入源

- 所有 API 都以 `BTreeMap<String, String>` 输入脚本源：
  - `key`: 虚拟路径（如 `main.script.xml`、`shared.defs.xml`、`game.json`）
  - `value`: 文件文本内容

### 2.2 运行输出

- `EngineOutput`（来自 `sl-core`）：
  - `Text { text }`
  - `Choices { items, prompt_text }`
  - `Input { prompt_text, default_text }`
  - `End`

### 2.3 快照

- `SnapshotV3`（来自 `sl-core`）：
  - 包含运行帧、随机数状态、待处理边界（choice/input）和 once 状态。
  - `snapshot()` 仅允许在等待 choice/input 边界时调用。

### 2.4 错误

- 统一错误类型：`ScriptLangError`
  - 字段：`code`, `message`, `span`
  - 宿主侧建议以 `code` 做稳定分支处理。

## 3. `sl-api` 高层 API

## 3.1 `compile_scripts_from_xml_map`

仅编译并返回 `scripts`（不返回 entry/global_json）。

```rust
use std::collections::BTreeMap;
use sl_api::compile_scripts_from_xml_map;

let mut files = BTreeMap::new();
files.insert("main.script.xml".to_string(), r#"<script name="main"><text>Hello</text></script>"#.to_string());

let scripts = compile_scripts_from_xml_map(&files)?;
assert!(scripts.contains_key("main"));
# Ok::<(), sl_core::ScriptLangError>(())
```

## 3.2 `compile_project_from_xml_map`

编译完整工程，返回：
- `scripts`
- `entry_script`（显式指定或默认 `main`）
- `global_json`

```rust
use std::collections::BTreeMap;
use sl_api::compile_project_from_xml_map;

let files = BTreeMap::from([
    ("main.script.xml".to_string(), r#"<script name="main"><text>Hello</text></script>"#.to_string())
]);

let project = compile_project_from_xml_map(&files, None)?;
assert_eq!(project.entry_script, "main");
# Ok::<(), sl_core::ScriptLangError>(())
```

## 3.3 `create_engine_from_xml`

创建并自动 `start()` 引擎，常用于新会话。

参数：`CreateEngineFromXmlOptions`
- `scripts_xml`: 源文件映射
- `entry_script`: 可选；缺省自动解析
- `entry_args`: 入口脚本参数（`BTreeMap<String, SlValue>`）
- `host_functions`: 宿主函数注册表（当前 runtime 构建暂不支持真正调用）
- `random_seed`: 随机种子（决定 `random(n)` 序列）
- `compiler_version`: 快照版本兼容用

```rust
use std::collections::BTreeMap;
use sl_api::{create_engine_from_xml, CreateEngineFromXmlOptions};
use sl_core::EngineOutput;

let files = BTreeMap::from([
    ("main.script.xml".to_string(), r#"<script name="main"><text>Hello</text></script>"#.to_string())
]);

let mut engine = create_engine_from_xml(CreateEngineFromXmlOptions {
    scripts_xml: files,
    entry_script: None,
    entry_args: None,
    host_functions: None,
    random_seed: Some(1),
    compiler_version: Some("player.v1".to_string()),
})?;

assert!(matches!(engine.next_output()?, EngineOutput::Text { .. }));
# Ok::<(), sl_core::ScriptLangError>(())
```

## 3.4 `resume_engine_from_xml`

从快照恢复引擎，常用于存档读档。

参数：`ResumeEngineFromXmlOptions`
- `scripts_xml`
- `snapshot`
- `host_functions`
- `compiler_version`

```rust
use std::collections::BTreeMap;
use sl_api::{
    create_engine_from_xml, resume_engine_from_xml,
    CreateEngineFromXmlOptions, ResumeEngineFromXmlOptions
};
use sl_core::EngineOutput;

let files = BTreeMap::from([
    ("main.script.xml".to_string(), r#"
<script name="main">
  <choice text="Pick">
    <option text="A"><text>A</text></option>
  </choice>
</script>
"#.to_string())
]);

let mut engine = create_engine_from_xml(CreateEngineFromXmlOptions {
    scripts_xml: files.clone(),
    entry_script: None,
    entry_args: None,
    host_functions: None,
    random_seed: Some(1),
    compiler_version: Some("player.v1".to_string()),
})?;

assert!(matches!(engine.next_output()?, EngineOutput::Choices { .. }));
let snapshot = engine.snapshot()?;

let mut resumed = resume_engine_from_xml(ResumeEngineFromXmlOptions {
    scripts_xml: files,
    snapshot,
    host_functions: None,
    compiler_version: Some("player.v1".to_string()),
})?;
resumed.choose(0)?;
assert!(matches!(resumed.next_output()?, EngineOutput::Text { .. }));
# Ok::<(), sl_core::ScriptLangError>(())
```

## 4. `sl-runtime` 直接 API（底层）

主要公开方法：
- `ScriptLangEngine::new(options)`
- `start(entry_script_name, entry_args)`
- `next_output()`
- `choose(index)`
- `submit_input(text)`
- `snapshot()`
- `resume(snapshot)`
- `waiting_choice()`
- `compiler_version()`

### 4.1 执行状态机协议（宿主循环）

```rust
loop {
    match engine.next_output()? {
        sl_core::EngineOutput::Text { text } => {
            println!("{}", text);
        }
        sl_core::EngineOutput::Choices { items, .. } => {
            let selected = items[0].index;
            engine.choose(selected)?;
        }
        sl_core::EngineOutput::Input { .. } => {
            engine.submit_input("player-input")?;
        }
        sl_core::EngineOutput::End => break,
    }
}
# Ok::<(), sl_core::ScriptLangError>(())
```

## 4.2 存档/读档规则

- `snapshot()` 只能在 `Choices` 或 `Input` 边界调用。
- `resume(snapshot)` 会校验：
  - `snapshot.schema_version`
  - `snapshot.compiler_version`
  - pending boundary 与当前脚本节点是否一致

建议流程：
1. `next_output()` 得到 `Choices/Input`
2. 立即 `snapshot()` 并持久化
3. 恢复时 `resume(snapshot)`
4. 再调用 `choose/submit_input`

## 5. API 行为要点（集成注意）

1. `create_engine_from_xml` 会自动 `start`。  
2. `compile_project_from_xml_map(..., None)` 默认入口脚本必须是 `main`。  
3. `choose(index)` / `submit_input(text)` 必须在对应 pending boundary 下调用。  
4. `random(n)` 要求 `n > 0`。  
5. JSON 全局变量在 runtime 只读。  
6. 传 `random_seed` 可保证可复现实验。  

## 6. 宿主函数现状

`HostFunctionRegistry` 接口已存在，但当前 runtime 构建遇到非空 host function 列表会返回：
- `ENGINE_HOST_FUNCTION_UNSUPPORTED`

因此当前版本应避免依赖 host function 真正执行。

## 7. 建议的错误处理模式

```rust
match engine.next_output() {
    Ok(output) => {
        // 正常处理输出
    }
    Err(err) => {
        eprintln!("code={} message={}", err.code, err.message);
        if let Some(span) = err.span {
            eprintln!(
                "span {}:{} - {}:{}",
                span.start.line, span.start.column, span.end.line, span.end.column
            );
        }
    }
}
```

## 8. 实战检查清单

- 入口脚本是否存在（默认 `main`）。
- 所有 include 路径是否可解析且无循环。
- 每次收到 `Choices/Input` 是否及时存档。
- `compiler_version` 是否在新旧进程间一致。
- 是否按 `EngineOutput` 协议驱动 `choose/submit_input`。
