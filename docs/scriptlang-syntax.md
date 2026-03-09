# ScriptLang 语法手册（scriptlang-rs）

本文档按“语法点”逐一说明当前 `scriptlang-rs` 支持的 XML 语法。每个语法点都附带至少一个示例，可直接用于 `examples` 风格工程。

文档职责：
- 本文档只定义语言语法与语义（source of truth for syntax）。
- API 与 CLI 细节由各自专题文档负责（通过 README 导航访问）。

边界约束：
- 本文档不定义 Rust API 函数签名、artifact/snapshot 字段契约。
- 本文档不定义 CLI 参数、输出行协议和回放流程。
- 若规则属于 API/CLI，改为链接到对应文档，不在此重复展开。

## 1. 文件类型

## 1.1 `*.xml`（模块源码）

XML 源文件统一使用普通 `name.xml` 文件名，且根节点必须是 `<module name="...">`。

```xml
<module name="battle" default_access="public">
  <type name="Combatant">
    <field name="hp" type="int"/>
  </type>
  <function name="boost" args="int:x" return="int:out">
    out = x + 1;
  </function>
  <var name="baseHp" type="int">100</var>
  <script name="main">
    <temp name="hero" type="Combatant">#{hp: baseHp}</temp>
    <call script="next"/>
  </script>
  <script name="next">
    <text>${boost(hero.hp)}</text>
  </script>
</module>
```

规则：
- 一个 `*.xml` 模块文件内可以有多个 `<script>`。
- `<module>` 下允许的直接子节点只有：`<type>`、`<function>`、`<var>`、`<const>`、`<script>`。
- module 名只取自 `<module name="...">`，不从文件名推导。
- `<module default_access="public|private">` 可设置 module 内默认可见性，默认是 `private`。
- `<type>/<function>/<var>/<const>/<script>` 可单独声明 `access="public|private"`；未声明时继承 `default_access`。
- 仅支持 `*.xml` 模块文件，且根节点必须是 `<module>`。
- module 内脚本对外注册名是 `moduleName.scriptName`，例如 `battle.main`。
- module 内 `type/function/var/const` 仍属于同一命名空间，例如 `battle.Combatant`、`battle.boost`、`battle.baseHp`。
- 同一个 module 内部，脚本可以直接用短名访问本 module 的 `type/function/var/const`，也可以用短名调用 sibling script。
- 跨 module 访问任何元素时，都必须使用限定名，例如 `shared.boost`、`shared.hp`、`shared.Hero`、`battle.main`。
- 跨 module import 只能访问对方 `public` 元素；`private` 仅在本 module 内可见。
- 宿主入口脚本必须是 `public`；`private` 脚本不能作为 entry。

## 2. import 语法

使用 XML 注释行：

```xml
<!-- import Shared from shared.xml -->
<!-- import { Battle, Shared } from modules/ -->
```

示例：

```xml
<!-- import { shared } from shared/ -->
<module name="main" default_access="public">
  <script name="main">
    <text>${shared.add(1, 2)}</text>
  </script>
</module>
```

规则：
- 允许在 `*.xml` 模块文件中声明 import。
- 路径相对当前文件。
- `<!-- import Shared from shared.xml -->` 只导入该文件声明的 `Shared` module。
- `<!-- import { Battle, Shared } from modules/ -->` 递归扫描目录，只导入显式列出的 module。
- 目录 import 要求目录树内 module 名唯一；重名直接报错。
- import 缺失、目录未匹配到任何 module、或循环依赖都会编译报错。

## 3. `<script>` 顶层属性

## 3.1 `name`（必填）

`name` 是脚本的局部名；对外编译名是 `moduleName.name`。

```xml
<module name="main" default_access="public">
  <script name="main">
    <text>Main</text>
  </script>
</module>
```

## 3.2 `args`（可选）

参数格式：`type:name` 或 `ref:type:name`，逗号分隔。

```xml
<script name="battle" args="int:hp,ref:int:score">
  <text>HP=${hp}</text>
</script>
```

## 3.3 module 内 `<script>` 的命名

当 `<script>` 出现在 `<module name="battle" default_access="public">` 内：
- 局部名仍然写在 `name` 上，例如 `<script name="main">`
- 编译后的公开脚本名是 `battle.main`
- `entry_script`、`<call script="...">`、`<return script="...">` 对外都应使用 `battle.main` 这种限定名
- 但在同一个 module 内部，可以写 `<call script="next"/>`，它会解析到当前 module 下的 `battle.next`

示例：

```xml
<module name="battle" default_access="public">
  <script name="main">
    <call script="next"/>
  </script>
  <script name="next">
    <text>done</text>
  </script>
</module>
```

## 4. `<module>` 顶层属性

## 4.1 `name`（必填）

作为命名空间前缀（如 `shared.boost`）。

```xml
<module name="shared" default_access="public">
  <function name="boost" args="int:x" return="int:out">
    out = x + 1;
  </function>
</module>
```

对于 `<module name="battle" default_access="public">`，这个名字同时决定：
- `battle.Type`
- `battle.func`
- `battle.var`
- `battle.script`

## 4.2 `<module><var>`（全局可写变量）

`<module>` 下可以声明全局变量：

```xml
<module name="shared" default_access="public">
  <var name="hp" type="int">100</var>
</module>
```

语义规则：
- 变量在 `engine.start(...)` 时按声明顺序初始化。
- 可见性遵循 import 闭包：脚本可见才可读写。
- 读取/写入优先级：局部（含参数） > module 全局。
- 本 module 内声明的全局变量可直接用短名（如 `hp`）。
- 来自其他 module 的全局变量必须使用全名（如 `shared.hp`）。
- module 全局初始化表达式可以引用“前面已声明并已初始化”的 module `<var>`；前向引用会编译失败。

补充：
- `<module><var>` 使用统一的全局可写变量运行时模型。
- 它们都会参与 snapshot / resume。
- module 内脚本天然可以看到本 module 的这些全局变量。

## 4.3 `<module><const>`（全局只读常量）

`<module>` 下可声明只读常量：

```xml
<module name="shared" default_access="public">
  <const name="baseHp" type="int">40</const>
</module>
```

语义规则：
- 可见性/短名/限定名规则与 `<module><var>` 相同。
- 常量在 `engine.start(...)` 时初始化，运行时禁止写入（包括代码赋值、`input`、路径写入）。
- `<const>` 不参与 snapshot/save；`resume` 时会按声明重新构建。
- `<const>` 初始化表达式可引用已初始化的 const；若引用 `<var>` 会编译失败。

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

## 5.4 自定义类型（来自 module）

本 module 内可直接写短名；跨 module 必须写全名 `ns.Type`。

```xml
<var name="hero" type="shared.Hero">#{hp: 10}</var>
```

## 6. `<script>` 可执行节点语法点

## 6.1 `<temp>`

用途：声明变量。  
属性：`name`、`type`（必填）。  
初值：使用节点内联表达式；为空则用类型默认值。  

```xml
<temp name="hp" type="int">3</temp>
<temp name="title" type="string">"Knight"</temp>
```

## 6.2 `<text>`

用途：输出文本。支持 `${expr}` 插值。  
属性：`once`（可选，`true/false`）、`tag`（可选，宿主扩展标签，运行时透传）。  

```xml
<text once="true">Welcome, ${name}</text>
<text tag="sound">sfx/open-door.ogg</text>
```

## 6.2.1 `<debug>`

用途：输出调试文本。支持 `${expr}` 插值。  
属性：不支持任何属性（`once/tag` 都不支持）。  
说明：运行时会产出独立 `Debug` 事件，不并入普通 `Text` 事件。  

```xml
<debug>hp=${hp}, round=${round}</debug>
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
子节点：允许 `<option>` 和 `<dynamic-options>`（可混排，按源码顺序展开）。  

```xml
<choice text="Choose">
  <option text="A"><text>Alpha</text></option>
  <dynamic-options array="arr" item="it" index="i">
    <option text="${it}:${i}"><text>Dyn</text></option>
  </dynamic-options>
  <option text="B"><text>Beta</text></option>
</choice>
```

## 6.9 `<option>`

用途：`<choice>` 的静态选项，或 `<dynamic-options>` 内的模板选项。  
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

当 `<option>` 用作 `<dynamic-options>` 模板时：
- 仅支持 `text`、`when`。
- 不支持 `once`。
- 不支持 `fall_over`。

## 6.10 `<dynamic-options>`

用途：从数组动态展开 choice 选项。  
属性：
- `array`（必填，表达式，运行时必须是数组）
- `item`（必填，数组元素绑定名）
- `index`（可选，元素索引绑定名）

子节点规则：
- 必须且只能有一个直接子节点 `<option>`，作为模板。

```xml
<choice text="Pick">
  <dynamic-options array="items" item="it" index="i">
    <option text="${it.name}" when="it.enabled">
      <text>picked ${it.name} at ${i}</text>
    </option>
  </dynamic-options>
</choice>
```

## 6.11 `<input>`

用途：请求宿主输入字符串并写入变量。  
属性：`var`、`text`（必填）。  
限制：不支持 `default` 属性，不允许子节点/内联文本。  

```xml
<temp name="heroName" type="string">"Traveler"</temp>
<input var="heroName" text="请输入名字"/>
<text>Hello ${heroName}</text>
```

## 6.12 `<break/>`

用途：跳出最近的 `<while>`。  
限制：只能在 `<while>` 内使用。  

```xml
<while when="true">
  <break/>
</while>
```

## 6.13 `<continue/>`

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

## 6.14 `<call>`

用途：调用其他脚本。  
属性：
- `script`（必填，支持 `${expr}` 模板插值）
- `args`（可选，位置参数）

`args` 支持：
- 值参数：`expr`
- 引用参数：`ref:path`

```xml
<call script="battle" args="hp, ref:score"/>
```

```xml
<call script="${nextScene}" args="hp"/>
```

module 相关规则：
- 对外调用 module 脚本时，推荐直接写限定名，例如 `<call script="battle.main"/>`
- 在同 module 内调用 sibling script 时，可写短名，例如 `<call script="next"/>`
- 若 `script` 使用 `${expr}` 动态插值且结果是短名，则只会解析到当前 module 的 sibling script

## 6.15 `<return>`

用途：从当前脚本返回，或转移到新脚本。  
属性：
- `script`（可选，支持 `${expr}` 模板插值）
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

```xml
<return script="scene-${chapter}"/>
```

module 相关规则与 `<call>` 相同：
- 对外跳转到 module 脚本时使用 `battle.next`
- 同 module 内可写短名 `next`
- 动态目标 `${expr}` 若产出短名，也只会解析到当前 module 的 sibling script

## 6.16 `<group>`

用途：语句分组容器，创建块级作用域。  
属性：无。  
语义：其子节点按出现顺序执行；在 `<group>` 中声明的 `<temp>` 仅在该组内可见，可在其他 `<group>` 中重名声明。  

```xml
<group>
  <temp name="title" type="string">"Knight"</temp>
  <text>In group: ${title}</text>
</group>

<group>
  <temp name="title" type="string">"Mage"</temp>
  <text>In group: ${title}</text>
</group>
```

## 7. `<module>` 声明语法点

## 7.1 `<type>`

用途：声明对象类型。  
属性：`name`（必填）。  
子节点：`<field>`。  
可出现位置：`<module>` 直接子节点。  

```xml
<module name="shared" default_access="public">
  <type name="Hero">
    <field name="hp" type="int"/>
    <field name="name" type="string"/>
  </type>
</module>
```

## 7.2 `<field>`

用途：定义类型字段。  
属性：`name`、`type`（必填）。  

```xml
<field name="hp" type="int"/>
```

## 7.3 `<function>`

用途：声明命名空间函数。  
属性：
- `name`（必填）
- `args`（可选，`type:name`）
- `return`（必填，`type:name`）

限制：
- module 函数 `args` 不支持 `ref:`
- module 函数 `return` 不支持 `ref:`
- 函数体只能是内联代码文本，不允许子元素

```xml
<module name="shared" default_access="public">
  <function name="add" args="int:a,int:b" return="int:out">
    out = a + b;
  </function>
</module>
```

## 7.4 `<module><script>`

用途：在 module 内声明可执行脚本。  
可出现位置：仅 `<module>` 直接子节点。  

```xml
<module name="camp" default_access="public">
  <script name="main">
    <text>Camp</text>
  </script>
  <script name="rest">
    <text>Rest</text>
  </script>
</module>
```

规则：
- `name` 必填，且在同一 module 下不能重复。
- 同名局部脚本可以存在于不同 module 中，例如 `a.main` 和 `b.main` 可以共存。
- module 内 `<script>` 的正文语法，与本文其他 `<script>` 节点语法完全一致。
- 只要某个 `*.xml` 模块文件被 import，该模块内脚本天然看到同文件同 module 的 `type/function/var`。

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

ScriptLang 表达式层不再推荐 XML 转义写法，而是使用自己的保留字：

- `<` 写成 `LT`
- `<=` 写成 `LTE`
- `&&` 写成 `AND`
- XML 属性表达式里的字符串写单引号：`'Rin'`
- `<code>` / `<function>` / `<var>...</var>` / `<temp>...</temp>` 里的字符串写双引号：`"Rin"`

旧写法会在送入 Rhai 前报错：

- `&lt;`
- `&lt;=`
- `&amp;&amp;`
- 属性表达式里的 `&quot;...&quot;`

普通文本节点里如果只是展示 XML 字符，仍然按 XML 规则写 `&lt;` / `&amp;`。

示例 1：`<if when="...">` 中的比较和逻辑表达式

```xml
<if when="hp LT 10">
  <text>danger</text>
</if>
```

```xml
<if when="hp LTE 10 AND name == 'Rin'">
  <text>danger</text>
</if>
```

示例 1b：`<code>` / `<function>` 中的字符串

```xml
<code>name = "Rin";</code>
```

示例 2：文本节点里显示尖括号/`&`

```xml
<text>使用 &lt;tag&gt; 语法，并用 A &amp; B 连接。</text>
```

## 10.4 内建动态函数调用 `invoke(name, [args])`

用途：按字符串函数名动态调用 `<function>`。  
约束：
- 仅支持限定名：`module.func`。
- 第二个参数必须是数组字面量/数组表达式。
- 可调用任意 module 的 `public` 函数，即使当前脚本未 import 对方 module。
- 不能调用 `private` 函数。
- 参数个数必须与目标函数声明一致。

```xml
<module name="main" default_access="public">
  <script name="main">
    <temp name="v" type="int">invoke("shared.add", [3, 4])</temp>
    <text>${v}</text>
  </script>
</module>
```

常见错误：
- `invoke("add", [1])`：`name` 不是 `module.func`。
- `invoke("shared.add", 1)`：`args` 不是数组。
- `invoke("shared.hidden", [1])`：目标函数是 `private`。

## 11. 综合示例

```xml
<!-- import shared from shared.xml -->
<module name="main" default_access="public">
  <script name="main" args="string:name">
    <temp name="hp" type="int">3</temp>
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
        <return script="main.result" args="name, hp"/>
      </option>
    </choice>
  </while>
  </script>
</module>
```

## 12. 参考

- 运行示例：`crates/sl-test-example/examples/`
- Engine API：由 API 专题文档负责（见 README 导航）
