# ScriptLang 语法规则（scriptlang-rs）

本文档记录 `scriptlang-rs` 当前支持的 ScriptLang XML 语法，用于编写 `examples/scripts-rhai` 下的脚本与 defs。

## 1. 文件类型与根节点

- 可执行脚本：`*.script.xml`，根节点为 `<script name="...">`
- 声明文件：`*.defs.xml`，根节点为 `<defs name="...">`
- 全局只读数据：`*.json`（通过 include 暴露为全局符号）

示例：

```xml
<!-- include: shared.defs.xml -->
<script name="main" args="int:hp">
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
- 路径相对当前文件（推荐写在文件头，编译器会扫描整份源文本中的 include 注释行）。
- 支持目标：`.script.xml`、`.defs.xml`、`.json`。
- include 缺失或循环依赖会在编译时报错。
- 脚本表达式/defs 函数代码若引用了 JSON 全局符号，但当前可见 include 闭包中没有该 `.json`，会在编译期报错（而不是等到运行期）。

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
- `<break>` / `<continue>`
- `<call>`
- `<return>`

说明：`<function>` 只能定义在 `<defs>` 中，不能作为 `<script>` 直接子节点。

## 4. `<defs>` 声明节点

- `<type name="TypeName">` + `<field name="..." type="..."/>`
- `<function name="Func" args="type:name,..." return="type:name">...</function>`
- `<defs name="shared">` 中声明的类型/函数具备命名空间前缀：`shared.TypeName`、`shared.func(...)`

示例：

```xml
<defs name="shared">
  <type name="Fighter">
    <field name="hp" type="int"/>
  </type>
  <function name="boost" args="int:base" return="int:out">
    out = base + 1;
  </function>
</defs>
```

## 5. 类型系统（摘要）

支持类型表达式：

- 基础类型：`int` / `float` / `string` / `boolean`
- 数组：`T[]`
- 映射：`#{T}`（key 固定为 string）
- 自定义对象类型：`ns.TypeName`（推荐）或在无歧义时使用短名 `TypeName`

## 6. 常用节点语义

- `<var name="x" type="int">1</var>`：声明变量，作用域在当前块内。
- `<var>` 不再支持 `value` 属性，只能使用节点内联文本作为初始表达式。
- `<text once="true">...</text>`：文本输出，支持 `${expr}` 插值；`once` 仅允许 `true/false`，表示同一脚本生命周期内只触发一次。
- `<code>...</code>`：执行 Rhai 代码，可读写可见变量。
- `<if when="...">...</if>`：条件分支，`when` 必须为布尔表达式；可选 `<else>` 分支。
- `<while when="...">...</while>`：循环；支持 `<break/>`、`<continue/>`。
- `<loop times="...">...</loop>`：循环语法糖（编译期展开）。
- `<choice text="...">` + `<option text="...">`：生成选项分支。
- `<option>` 支持 `when`（条件显示）、`once`（单次可见）与 `fall_over`（兜底项）。
- `<continue/>` 还可作为 `<option>` 的直接子节点，表示选中后立即回到当前 choice 边界。
- `<input var="name" text="Prompt"/>`：等待宿主输入并写入字符串变量；不支持 `default` 属性，也不能带子节点/内联文本。
- `<call script="other" args="..."/>`：调用其他脚本。
- `<return/>` 或 `<return script="next" args="..."/>`：返回/跳转返回；`return` 的 `args` 不支持 `ref`，且声明 `args` 时必须同时声明 `script`。
- defs 函数调用支持命名空间写法，例如：`shared.boost(hp)`。

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
- `once` 属性只允许出现在 `<text>` 和 `<option>`。
- `fall_over` 每个 `<choice>` 最多一个，且必须是最后一个 `<option>`，同时不能声明 `when`。
- `<continue/>` 只能出现在 `<while>` 内，或作为 `<option>` 的直接子节点。
- `<else>` 只能出现在 `<if>` 内。
- 已移除节点：`<vars> <step> <set> <push> <remove>`。
- `__` 前缀为保留命名（脚本名、类型名、字段名、函数名、变量名等均不建议使用）。
- XML 属性中 `<` 需要写成 `&lt;`。

## 9. 一个最小可运行示例

```xml
<!-- include: shared.defs.xml -->
<script name="main" args="string:name">
  <var name="hp" type="int">3</var>
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
- TypeScript 参考语法手册：[xumingthepoet/scriptlang/docs/product-specs/syntax-manual.md](https://github.com/xumingthepoet/scriptlang/blob/main/docs/product-specs/syntax-manual.md)
