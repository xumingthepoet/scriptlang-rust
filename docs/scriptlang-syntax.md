# ScriptLang 语法手册（scriptlang-rs）

本文档按“语法点”逐一说明当前 `scriptlang-rs` 支持的 XML 语法。每个语法点都附带至少一个示例，可直接用于 `examples/scripts-rhai` 风格工程。

## 1. 文件类型

## 1.1 `*.script.xml`（可执行脚本）

要求根节点是 `<script>`。

```xml
<script name="main">
  <text>Hello</text>
</script>
```

## 1.2 `*.defs.xml`（类型/函数/全局变量声明）

要求根节点是 `<defs>`。

```xml
<defs name="shared">
  <type name="Hero">
    <field name="hp" type="int"/>
  </type>
  <var name="baseHp" type="int">100</var>
</defs>
```

## 1.3 `*.json`（全局只读数据）

通过 include 进入可见闭包后，以“文件名（去扩展名）”作为符号使用。

```json
{ "bonus": 10 }
```

例如 `game.json` 在脚本里用 `game` 访问。

## 2. include 语法

使用 XML 注释行：

```xml
<!-- include: relative/path.ext -->
```

示例：

```xml
<!-- include: shared.defs.xml -->
<!-- include: game.json -->
<script name="main">
  <text>${shared.add(1, game.bonus)}</text>
</script>
```

规则：
- 允许在 `.script.xml` 和 `.defs.xml` 中声明 include。
- 路径相对当前文件。
- include 缺失或循环依赖会编译报错。

## 3. `<script>` 顶层属性

## 3.1 `name`（必填）

脚本名，全工程必须唯一。

```xml
<script name="main">
  <text>Main</text>
</script>
```

## 3.2 `args`（可选）

参数格式：`type:name` 或 `ref:type:name`，逗号分隔。

```xml
<script name="battle" args="int:hp,ref:int:score">
  <text>HP=${hp}</text>
</script>
```

## 4. `<defs>` 顶层属性

## 4.1 `name`（必填）

作为命名空间前缀（如 `shared.boost`）。

```xml
<defs name="shared">
  <function name="boost" args="int:x" return="int:out">
    out = x + 1;
  </function>
</defs>
```

## 4.2 `<defs><var>`（全局可写变量）

`<defs>` 下可以声明全局变量：

```xml
<defs name="shared">
  <var name="hp" type="int">100</var>
</defs>
```

语义规则：
- 变量在 `engine.start(...)` 时按声明顺序初始化。
- 可见性遵循 include 闭包：脚本可见才可读写。
- 读取/写入优先级：局部（含参数） > defs 全局 > JSON 全局（JSON 仍只读）。
- 访问方式：短名（如 `hp`）和全名（如 `shared.hp`）。
- 若短名冲突（多个 namespace 同名），短名不可用，只能用全名。
- defs 全局初始化表达式可以引用“前面已声明并已初始化”的 defs 全局；前向引用会编译失败。

## 5. 类型语法

## 5.1 基础类型

支持：`int` / `float` / `string` / `boolean`

```xml
<var name="hp" type="int">10</var>
```

## 5.2 数组类型 `T[]`

```xml
<var name="nums" type="int[]">[1, 2, 3]</var>
```

## 5.3 映射类型 `#{T}`

key 固定是 string。

```xml
<var name="dict" type="#{int}">#{a: 1, b: 2}</var>
```

## 5.4 自定义类型（来自 defs）

可用全名 `ns.Type`，或在无歧义场景下用短名。

```xml
<var name="hero" type="shared.Hero">#{hp: 10}</var>
```

## 6. `<script>` 可执行节点语法点

## 6.1 `<var>`

用途：声明变量。  
属性：`name`、`type`（必填）。  
初值：使用节点内联表达式；为空则用类型默认值。  

```xml
<var name="hp" type="int">3</var>
<var name="title" type="string">"Knight"</var>
```

## 6.2 `<text>`

用途：输出文本。支持 `${expr}` 插值。  
属性：`once`（可选，`true/false`）。  

```xml
<text once="true">Welcome, ${name}</text>
```

## 6.3 `<code>`

用途：执行 Rhai 代码。  

```xml
<code>hp = hp - 1;</code>
```

## 6.4 `<if>`

用途：条件分支。  
属性：`when`（必填，布尔表达式）。  

```xml
<if when="hp > 0">
  <text>alive</text>
</if>
```

## 6.5 `<else>`

用途：`<if>` 的否则分支，只能出现在 `<if>` 内。

```xml
<if when="hp > 0">
  <text>alive</text>
  <else>
    <text>dead</text>
  </else>
</if>
```

## 6.6 `<while>`

用途：循环执行。  
属性：`when`（必填，布尔表达式）。  

```xml
<while when="hp > 0">
  <text>HP=${hp}</text>
  <code>hp = hp - 1;</code>
</while>
```

## 6.7 `<loop>`

用途：循环语法糖（编译期展开为 `var + while`）。  
属性：`times`（必填，表达式，不能写 `${...}` 包裹）。  

```xml
<loop times="3">
  <text>tick</text>
</loop>
```

## 6.8 `<choice>`

用途：生成可选分支边界。  
属性：`text`（必填，提示文本）。  
子节点：仅允许 `<option>`。  

```xml
<choice text="Choose">
  <option text="A"><text>Alpha</text></option>
  <option text="B"><text>Beta</text></option>
</choice>
```

## 6.9 `<option>`

用途：`<choice>` 的选项。  
属性：
- `text`（必填）
- `when`（可选，显示条件）
- `once`（可选，单次可见）
- `fall_over`（可选，兜底选项）

```xml
<choice text="Choose">
  <option text="Fight" when="hp > 0"><text>Battle</text></option>
  <option text="Leave" fall_over="true"><text>Escape</text></option>
</choice>
```

`fall_over` 规则：
- 每个 `<choice>` 最多一个 `fall_over="true"`。
- 必须是最后一个 `<option>`。
- `fall_over` 选项不能再声明 `when`。

## 6.10 `<input>`

用途：请求宿主输入字符串并写入变量。  
属性：`var`、`text`（必填）。  
限制：不支持 `default` 属性，不允许子节点/内联文本。  

```xml
<var name="heroName" type="string">"Traveler"</var>
<input var="heroName" text="请输入名字"/>
<text>Hello ${heroName}</text>
```

## 6.11 `<break/>`

用途：跳出最近的 `<while>`。  
限制：只能在 `<while>` 内使用。  

```xml
<while when="true">
  <break/>
</while>
```

## 6.12 `<continue/>`

用途：继续最近循环，或作为 `<option>` 直接子节点回到 choice。  
限制：
- 在循环语义下：必须在 `<while>` 内。
- 在 choice 语义下：必须是 `<option>` 的直接子节点。

```xml
<while when="hp > 0">
  <code>hp = hp - 1;</code>
  <continue/>
</while>
```

```xml
<choice text="Pick">
  <option text="Again">
    <continue/>
  </option>
</choice>
```

## 6.13 `<call>`

用途：调用其他脚本。  
属性：
- `script`（必填）
- `args`（可选，位置参数）

`args` 支持：
- 值参数：`expr`
- 引用参数：`ref:path`

```xml
<call script="battle" args="hp, ref:score"/>
```

## 6.14 `<return>`

用途：从当前脚本返回，或转移到新脚本。  
属性：
- `script`（可选）
- `args`（可选）

规则：
- `args` 不支持 `ref:`
- 若声明 `args`，必须同时声明 `script`

```xml
<return/>
```

```xml
<return script="nextScene" args="heroName, hp"/>
```

## 6.15 `<group>`

用途：语句分组容器，创建块级作用域。  
属性：无。  
语义：其子节点按出现顺序执行；在 `<group>` 中声明的 `<var>` 仅在该组内可见，可在其他 `<group>` 中重名声明。  

```xml
<group>
  <var name="title" type="string">"Knight"</var>
  <text>In group: ${title}</text>
</group>

<group>
  <var name="title" type="string">"Mage"</var>
  <text>In group: ${title}</text>
</group>
```

## 7. `<defs>` 声明语法点

## 7.1 `<type>`

用途：声明对象类型。  
属性：`name`（必填）。  
子节点：`<field>`。  

```xml
<defs name="shared">
  <type name="Hero">
    <field name="hp" type="int"/>
    <field name="name" type="string"/>
  </type>
</defs>
```

## 7.2 `<field>`

用途：定义类型字段。  
属性：`name`、`type`（必填）。  

```xml
<field name="hp" type="int"/>
```

## 7.3 `<function>`

用途：声明 defs 函数。  
属性：
- `name`（必填）
- `args`（可选，`type:name`）
- `return`（必填，`type:name`）

限制：
- defs 函数 `args` 不支持 `ref:`
- defs 函数 `return` 不支持 `ref:`
- 函数体只能是内联代码文本，不允许子元素

```xml
<defs name="shared">
  <function name="add" args="int:a,int:b" return="int:out">
    out = a + b;
  </function>
</defs>
```

## 8. JSON 全局可见性语法点

JSON 全局符号必须在 include 闭包内可见，否则编译失败。

```xml
<!-- include: game.json -->
<script name="main">
  <text>bonus=${game.bonus}</text>
</script>
```

## 9. 参数解析语法点

## 9.1 `<script args="...">`

```xml
<script name="main" args="int:hp,ref:int:score">
  <text>${hp}</text>
</script>
```

## 9.2 `<call args="...">`

```xml
<call script="battle" args="hp + 1, ref:score"/>
```

## 9.3 `<return args="...">`

```xml
<return script="next" args="hp, title"/>
```

## 10. 命名与约束语法点

## 10.1 保留前缀

`__` 前缀为保留命名，不可用于脚本名、类型名、函数名、变量名等。

```xml
<!-- 不建议/会被拒绝 -->
<script name="__internal">
  <text>x</text>
</script>
```

## 10.2 已移除节点

以下节点已移除：`<vars> <step> <set> <push> <remove>`。

```xml
<!-- 会编译报错 -->
<script name="main">
  <set path="x">1</set>
</script>
```

## 10.3 XML 转义

属性中出现 `<` 需写 `&lt;`。

```xml
<if when="hp &lt; 10">
  <text>danger</text>
</if>
```

## 11. 综合示例

```xml
<!-- include: shared.defs.xml -->
<!-- include: game.json -->
<script name="main" args="string:name">
  <var name="hp" type="int">3</var>
  <text once="true">你好，${name}</text>

  <loop times="2">
    <code>hp = hp + 1;</code>
  </loop>

  <while when="hp > 0">
    <choice text="动作">
      <option text="攻击" when="hp > 1">
        <code>hp = hp - 2;</code>
      </option>
      <option text="防御" once="true">
        <continue/>
      </option>
      <option text="结束" fall_over="true">
        <return script="result" args="name, hp"/>
      </option>
    </choice>
  </while>
</script>
```

## 12. 参考

- 运行示例：`examples/scripts-rhai/`
- Engine API：`docs/sl-engine-api.md`
