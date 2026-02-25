# ScriptLang 语法规则（scriptlang-rs）

本文档记录 `scriptlang-rs` 当前支持的 ScriptLang XML 语法，用于编写 `examples/scripts-rhai` 下的脚本与 defs。

## 1. 文件类型与根节点

- 可执行脚本：`*.script.xml`，根节点为 `<script name="...">`
- 声明文件：`*.defs.xml`，根节点为 `<defs name="...">`
- 全局只读数据：`*.json`（通过 include 暴露为全局符号）

示例：

```xml
<!-- include: shared.defs.xml -->
<script name="main" args="number:hp">
  <text>HP=${hp}</text>
</script>
```

## 2. include 头注释

在 `.script.xml` / `.defs.xml` 文件头可使用：

```xml
<!-- include: relative/path.ext -->
```

规则：

- 一行一个 include。
- 路径相对当前文件。
- 支持目标：`.script.xml`、`.defs.xml`、`.json`。
- include 缺失或循环依赖会在编译时报错。

## 3. `<script>` 顶层可执行节点

`<script>` 的直接子节点支持：

- `<var>`
- `<text>`
- `<code>`
- `<if>` / `<else>`
- `<while>`
- `<loop>`
- `<choice>` / `<option>`
- `<input>`
- `<call>`
- `<return>`

说明：`<function>` 只能定义在 `<defs>` 中，不能作为 `<script>` 直接子节点。

## 4. `<defs>` 声明节点

- `<type name="TypeName">` + `<field name="..." type="..."/>`
- `<function name="Func" args="type:name,..." return="type:name">...</function>`

示例：

```xml
<defs name="shared">
  <type name="Fighter">
    <field name="hp" type="number"/>
  </type>
  <function name="boost" args="number:base" return="number:out">
    out = base + 1;
  </function>
</defs>
```

## 5. 类型系统（摘要）

支持类型表达式：

- 基础类型：`number` / `string` / `boolean`
- 数组：`T[]`
- 映射：`Map<string, T>`
- 自定义对象类型：`TypeName`（来自可见 defs）

## 6. 常用节点语义

- `<var name="x" type="number" value="1"/>`：声明变量，作用域在当前块内。
- `<text>...</text>`：输出文本，支持 `${expr}` 插值。
- `<code>...</code>`：执行 Rhai 代码，可读写可见变量。
- `<if when="...">...</if>`：条件分支，`when` 必须为布尔表达式。
- `<while when="...">...</while>`：循环；支持 `<break/>`、`<continue/>`。
- `<loop times="...">...</loop>`：循环语法糖（编译期展开）。
- `<choice text="...">` + `<option text="...">`：生成选项分支。
- `<input var="name" text="Prompt"/>`：等待宿主输入并写入字符串变量。
- `<call script="other" args="..."/>`：调用其他脚本。
- `<return/>` 或 `<return script="next" args="..."/>`：返回/跳转返回。

## 7. 参数语法

- `<script args="...">` 使用逗号分隔参数：
  - 值传递：`type:name`
  - 引用传递：`ref:type:name`
- `<call args="...">` 使用位置参数：
  - 值参数：`expr`
  - 引用参数：`ref:varName`

## 8. 命名与约束（重点）

- `script name` 必须唯一。
- `choice/option/input/call/if/while` 等必填属性缺失会编译报错。
- `__` 前缀为保留命名（脚本名、类型名、字段名、函数名、变量名等均不建议使用）。
- XML 属性中 `<` 需要写成 `&lt;`。

## 9. 一个最小可运行示例

```xml
<!-- include: shared.defs.xml -->
<script name="main" args="string:name">
  <var name="hp" type="number" value="3"/>
  <text>你好，${name}</text>
  <while when="hp > 0">
    <text>HP=${hp}</text>
    <code>hp = hp - 1;</code>
  </while>
  <choice text="下一步？">
    <option text="结束">
      <return/>
    </option>
  </choice>
</script>
```

## 10. 参考

- Rust workspace 示例：`examples/scripts-rhai/`
- TypeScript 参考语法手册：`script-lang/docs/product-specs/syntax-manual.md`
