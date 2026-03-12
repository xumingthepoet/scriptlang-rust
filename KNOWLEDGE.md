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

### 2026-03-08 — 失败模式 — 可见性默认值变更后的入口回归
- 发现：当 `module` 默认可见性是 `private` 且 host entry 要求 `public` 时，旧样例/测试会在 entry 解析阶段集中失败。
- 细节：迁移时优先给“预期可对外运行”的 module 显式加 `default_access="public"`，并只把需要收敛的符号改成元素级 `access="private"`；否则会在 `compile_artifact/create_engine/start` 三层同时触发 private-entry 错误。
- 证据：本次新增访问控制后，未声明可见性的 `main.main` 在 artifact/api/runtime 各层都被拒绝，补齐 `default_access="public"` 后恢复。

### 2026-03-09 — 规范 — 文档与校验优先“白名单正向表述”
- 发现：面向新用户的文档若强调“历史废弃/旧格式报错”，会增加无关认知负担；同类实现里黑名单分支也容易长期残留无效概念。
- 细节：对输入/语法约束优先采用白名单模型（如“仅支持 `*.xml` + `<module>`”）；文档默认只写当前支持能力。只有当需求明确是迁移/兼容排障时，才补充历史格式说明。错误码设计也优先统一“非白名单即 unsupported”。
- 证据：`source_parse` 从 `legacy_*` 黑名单改为纯白名单后，测试与文档仍可完整表达行为且维护成本更低。

### 2026-03-09 — 架构决策 — runtime/compiler 性能护栏需长期保留
- 发现：性能相关实现约束若只写在 README，后续文档正交化时容易被“去重复”误删，导致排障时缺少基线。
- 细节：以下属于长期性能护栏，后续重构不可无意回退：
  - `sl-runtime` 复用单个内部 Rhai engine，不在每次 eval 重新构建。
  - runtime 对脚本级 prelude 生成做缓存，避免重复拼接同构 prelude 文本。
  - parser/compiler/runtime 的稳定 regex 采用静态惰性初始化，避免热路径重复编译。
  - `sl-compiler` 在项目编译阶段缓存 script 可达 import 闭包，避免重复 DFS。
- 失败模式：若以上护栏回退，常见表现是“冷启动或首次路径显著变慢，重复操作稍快”；容易被误判成业务脚本问题。
- 如何验证：性能异常优先做“首次执行 vs 再次执行”对比，并优先检查上述四类护栏是否仍在对应模块中生效，再做业务层排查。

### 2026-03-10 — 失败模式 — Rhai 函数重写在“全量匹配”时会放大首轮延迟
- 发现：`sl-runtime` 的 Rhai 执行路径若对“全部可见函数名”逐个做调用重写，会导致首轮（以及高频 code 步）出现数量级延迟，尤其在 function 数量大时。
- 细节：已在 `crates/sl-runtime/src/engine/eval.rs` 加入两层护栏：  
  1) 无 `(` 时跳过函数调用重写；  
  2) 有 `(` 时先提取源码中真实被调用的 token，只对命中函数名做重写。  
  同时在 prelude 构建中延迟 `invoke_body_symbol_map`，仅当 function body 含调用时才计算。
- 证据：本地探针中 `functions=120 + with_temp=true` 的首轮耗时由约 `17.5s` 降到约 `176ms`；`functions=10, steps=100` 由约 `1.56s` 降到约 `294ms`。
- 剩余瓶颈：当前 `execute_rhai_with_mode` 仍是“构造源码字符串 -> Rhai 解析执行”的模式，循环中大量唯一表达式时仍有显著解析成本。后续优先考虑 AST 缓存（按最终 source 缓存 AST，改用 `eval_ast_with_scope/run_ast_with_scope`）。

### 2026-03-12 — 失败模式 — 短函数引用归一化需覆盖 module function body 双路径
- 发现：`*short_fn` 若仅在 script/module-var/module-const 初始化时归一化，而遗漏 `<function>` 代码体，会在跨模块转发后以原始短名进入 runtime，触发 `ENGINE_INVOKE_TARGET_NOT_FOUND`。
- 细节：`crates/sl-compiler/src/module_resolver.rs` 的两个函数收集入口都必须执行函数字面量归一化：  
  1) `resolve_visible_module_symbols_with_aliases_and_module_scoped_type_aliases`（运行期可见符号路径）  
  2) `collect_functions_for_bundle_with_aliases`（artifact/bundle 路径）  
  同时先收集完整可见函数名集合，再做归一化校验，避免前向引用被误判为 not found。
- 证据：最小复现为 `<function> return event_system.set_condition(*can_phase_2_fn); </function>`，修复前 `invoke(stored_condition, [])` 报 `Invoke target not found.*can_phase_2_fn`。

### 2026-03-12 — 架构决策 — `invoke(fnVar, args)` 采用“引用能力”语义
- 发现：动态调用阶段若再次做“调用方可见性”校验，会破坏 `function` 作为一等值跨模块转发后的可调用性。
- 细节：`crates/sl-runtime/src/engine/eval.rs` 的 `build_module_prelude` 中，`invoke` 分发必须按 `invoke_all_functions` 生成，不再按 `invoke_public_functions` 过滤；`module.func(...)` 静态调用可见性仍由可见函数符号映射约束，不随 `invoke` 变更。
- 失败模式：若把 `invoke` 回退为“仅 public 可调”，会重新出现“合法获得 private 引用但 invoke 失败”的行为回归。
- 如何验证：回归场景使用三模块链路（source -> relay -> app），由 source 公共函数返回 private 引用，app 侧 `invoke(fnRef, [...])` 必须成功，同时 `app` 里直接 `source.hidden(...)` 仍应失败。
