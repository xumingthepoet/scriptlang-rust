# ScriptLang 语法手册（scriptlang-rs）

本文档按“语法点”逐一说明当前 `scriptlang-rs` 支持的 XML 语法。每个语法点都附带至少一个示例，可直接用于 `examples` 风格工程。

## 1. 文件类型

## 1.1 `*.xml`（模块源码）

XML 源文件统一使用普通 `name.xml` 文件名，且根节点必须是 `<module name="...">`。

```xml
<module name="battle" export="type:Combatant;enum:State;function:boost;var:baseHp;script:main,next">
  <type name="Combatant">
    <field name="hp" type="int"/>
  </type>
  <enum name="State">
    <member name="Idle"/>
    <member name="Run"/>
  </enum>
  <function name="boost" args="int:x" return_type="int">
    return x + 1;
  </function>
  <var name="baseHp" type="int">100</var>
  <script name="main">
    <temp name="hero" type="Combatant">#{hp: baseHp}</temp>
    <call script="@next"/>
  </script>
  <script name="next">
    <text>${boost(hero.hp)}</text>
  </script>
</module>
```

规则：
- 一个 `*.xml` 模块文件内可以有多个 `<script>`。
- `<module>` 下允许的直接子节点只有：`<type>`、`<enum>`、`<function>`、`<var>`、`<const>`、`<script>`。
- `<module>` 允许嵌套 `<module>`；嵌套模块会在编译期展平为点分命名空间（例如 `a.b.c`）。
- module 名只取自 `<module name="...">`，不从文件名推导。
- 嵌套 `<module>` 的 `name` 必须是单段标识符（不允许 `.`）；最终命名空间由父子链路拼接。
- `<module export="...">` 控制 module 对外可见符号；未声明 `export` 时所有声明默认 `private`。
- `export` 语法：`kind:name1,name2;kind2:name3`，支持 `script/function/var/const/type/enum` 六类。
- `<type>/<enum>/<function>/<var>/<const>/<script>` 不支持 `access` 属性。
- 仅支持 `*.xml` 模块文件，且根节点必须是 `<module>`。
- module 内脚本对外注册名是 `moduleName.scriptName`，例如 `battle.main`。
- module 内 `type/enum/function/var/const` 仍属于同一命名空间，例如 `battle.Combatant`、`battle.State`、`battle.boost`、`battle.baseHp`。
- 同一个 module 内部，脚本可以直接用短名访问本 module 的 `type/function/var/const`，也可以用短名调用 sibling script。
- 跨 module 访问任何元素时，都必须使用限定名，例如 `shared.boost`、`shared.hp`、`shared.Hero`、`battle.main`。
- 跨 module import 只能访问对方在 `export` 中声明的元素；未导出的元素仅在本 module 内可见。
- 同一根 module 树（同文件内 `a.*`）内部，子模块之间可见对方 `export` 的符号；未 `export` 的符号仍不可见。
- 同根内部访问时，第一层子模块（如 `c.*`）不要求根模块 `a` 导出 `module:c`；但访问更深层（如 `c.d.*`）要求父模块逐层导出 `module`（例如 `c` 需 `export=\"module:d\"`）。
- 文件外访问时，根模块需要 `export="module:child"` 显式暴露子模块入口；未暴露子模块不可通过 `a.child.*` 访问。
- 宿主入口脚本必须在 `export` 中声明；未导出脚本不能作为 entry。
- 声明名（`script/type/enum/field/member/function/args/return/var/const/temp/dynamic-options item/index`）会在编译期做 Rhai 关键字冲突检查，命中时报 `NAME_RHAI_KEYWORD_RESERVED`（大小写敏感）。
- 声明名必须是单段标识符（`[A-Za-z_][A-Za-z0-9_]*`），不允许在声明位使用限定名（例如 `<enum name="main.State">` 非法）。
- 当前约束：`module name` 仅做 `__` 前缀保留检查，不参与 Rhai 关键字冲突拦截。

## 2. import 语法

使用 XML 注释行：

```xml
<!-- import Shared from shared.xml -->
<!-- import { Battle, Shared } from modules/ -->
<!-- alias Shared.hp -->
<!-- alias Shared.BASE as base -->
```

示例：

```xml
<!-- import { shared } from shared/ -->
<module name="main" export="script:main">
  <script name="main">
    <text>${shared.add(1, 2)}</text>
  </script>
</module>
```

规则：
- 允许在 `*.xml` 模块文件中声明 import。
- 路径相对当前文件。
- `<!-- import Shared from shared.xml -->` 只导入该文件声明的 `Shared` module。
- `import` 仅导入目标文件的根模块；不支持直接导入子模块。
- `<!-- import { Battle, Shared } from modules/ -->` 递归扫描目录，只导入显式列出的 module。
- 目录 import 要求目录树内 module 名唯一；重名直接报错。
- 同一文件内，重复导入到同一个目标文件会编译报错（不再静默去重）。
- import 缺失、目录未匹配到任何 module、或循环依赖都会编译报错。

## 2.1 alias 语法

使用 XML 注释行：

```xml
<!-- alias Shared.hp -->
<!-- alias Shared.Hero as HeroType -->
```

规则：
- `alias` 目标只支持两段限定名 `Module.Entity`。
- `<!-- alias Shared.hp -->` 等价于 `<!-- alias Shared.hp as hp -->`。
- `alias` 只影响当前文件脚本编译时的可见符号，不会新增 import 边。
- `alias` 目标必须在当前 import + export 可见闭包内。
- `alias` 仅支持 `type` / module `var` / module `const`，不支持 function。
- 同一文件内，重复声明完全相同的 alias 会编译报错（不再静默跳过）。
- 同名 alias 指向不同目标会编译报错；alias 名与同类已可见名字冲突也会编译报错。

## 3. `<script>` 顶层属性

## 3.1 `name`（必填）

`name` 是脚本的局部名；对外编译名是 `moduleName.name`。

```xml
<module name="main" export="script:main">
  <script name="main">
    <text>Main</text>
  </script>
</module>
```

## 3.2 `args`（可选）

参数格式：`type:name` 或 `ref:type:name`，逗号分隔。

约束：
- `kind="goto"`（默认）脚本的 `args` 不支持 `ref:`
- `kind="call"` 脚本的 `args` 支持 `ref:`

```xml
<script name="battle" args="int:hp,ref:int:score">
  <text>HP=${hp}</text>
</script>
```

## 3.3 `kind`（可选，默认 `goto`）

可选值：
- `goto`：主流程脚本，允许 `<call>` / `<goto>` / `<end/>`，不允许 `<return/>`
- `call`：子过程脚本，允许 `<call>` / `<return/>`，不允许 `<goto>` / `<end/>`

```xml
<script name="helper" kind="call" args="ref:int:hp">
  <code>hp = hp + 1;</code>
  <return/>
</script>
```

## 3.4 module 内 `<script>` 的命名

当 `<script>` 出现在 `<module name="battle" export="script:main">` 内：
- 局部名仍然写在 `name` 上，例如 `<script name="main">`
- 编译后的公开脚本名是 `battle.main`
- `entry_script` 仍使用 `battle.main` 这种限定名（宿主入口规则不变）
- `<call script="...">`、`<goto script="...">` 只允许两种形式：`@module.script` / `@short`，或 `script` 类型变量名
- 在同一个 module 内部可以写 `@next`，编译期会补全为当前 module 下的 `battle.next`

示例：

```xml
<module name="battle" export="script:main,next">
  <script name="main">
    <call script="@next"/>
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
<module name="shared" export="function:boost">
  <function name="boost" args="int:x" return_type="int">
    return x + 1;
  </function>
</module>
```

对于 `<module name="battle">`，这个名字同时决定：
- `battle.Type`
- `battle.func`
- `battle.var`
- `battle.script`

## 4.2 `export`（可选）

用于声明 module 对外可见符号集合；未声明即“全部 private”。

```xml
<module name="shared" export="type:Hero;enum:State;function:boost;var:hp;const:base;script:main,next">
  <type name="Hero">
    <field name="hp" type="int"/>
  </type>
  <enum name="State">
    <member name="Idle"/>
  </enum>
  <function name="boost" args="int:x" return_type="int">
    return x + 1;
  </function>
  <var name="hp" type="int">100</var>
  <const name="base" type="int">7</const>
  <script name="main"><text>ok</text></script>
  <script name="next"><text>next</text></script>
</module>
```

规则：
- `export` 以分号分组：`kind:name1,name2`。
- `kind` 仅支持：`script/function/var/const/type/enum`。
- `kind` 支持：`module/script/function/var/const/type/enum`。
- `module:child1,child2` 用于导出根模块下可被外部访问的子模块入口。
- `name` 必须指向当前 module 内真实声明；缺失目标会编译失败。
- 重复导出同一 `kind:name` 会编译失败。

## 4.3 `<module><var>`（全局可写变量）

`<module>` 下可以声明全局变量：

```xml
<module name="shared" export="var:hp">
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
- 支持 `format` 属性：
  - 缺省/`format="inline"`：节点内联表达式（默认行为）
  - `format="xml"`：结构化初始化（对象 `<field>`、数组 `<item>`、map `<tuple key>`）

补充：
- `<module><var>` 使用统一的全局可写变量运行时模型。
- 它们都会参与 snapshot / resume。
- module 内脚本天然可以看到本 module 的这些全局变量。

`format="xml"` 示例：

```xml
<var name="hero" type="Hero" format="xml">
  <field name="hp">10</field>
  <field name="name">"Rin"</field>
</var>
<var name="nums" type="int[]" format="xml">
  <item>1</item>
  <item>2</item>
</var>
<var name="dict" type="#{string=>int}" format="xml">
  <tuple key="hp">10</tuple>
</var>
<var name="phaseScore" type="#{Stage=>int}" format="xml">
  <tuple key="Stage.Begin">1</tuple>
</var>
```

## 4.4 `<module><const>`（全局只读常量）

`<module>` 下可声明只读常量：

```xml
<module name="shared" export="const:baseHp">
  <const name="baseHp" type="int">40</const>
</module>
```

语义规则：
- 可见性/短名/限定名规则与 `<module><var>` 相同。
- 常量在 `engine.start(...)` 时初始化，运行时禁止写入（包括代码赋值、`input`、路径写入）。
- `<const>` 不参与 snapshot/save；`resume` 时会按声明重新构建。
- `<const>` 初始化表达式可引用已初始化的 const；若引用 `<var>` 会编译失败。
- `format` 规则与 `<module><var>` 相同，支持 `inline/xml`。

## 5. 类型语法

## 5.1 基础类型

支持：`int` / `float` / `string` / `boolean` / `script` / `function`

```xml
<var name="hp" type="int">10</var>
<var name="nextScene" type="script">@battle.main</var>
<var name="fnRef" type="function">*battle.add</var>
```

## 5.2 数组类型 `T[]`

```xml
<var name="nums" type="int[]">[1, 2, 3]</var>
```

## 5.3 映射类型 `#{K=>V}` / `#{V}`

- `#{K=>V}`：显式 key/value 类型。
- `#{V}`：简写，等价于 `#{string=>V}`。
- `K` 当前仅支持：`string` 或 `enum` 类型。
- 运行时底层 key 仍是 string；若 `K` 是 enum，则 key 必须命中 member 名。

```xml
<var name="dict" type="#{string=>int}">#{a: 1, b: 2}</var>
<var name="dict2" type="#{int}">#{a: 1, b: 2}</var>
<var name="stateScore" type="#{State=>int}">#{Idle: 0, Run: 10}</var>
```

## 5.4 自定义类型（来自 module）

本 module 内可直接写短名；跨 module 必须写全名 `ns.Type`。

```xml
<var name="hero" type="shared.Hero">#{hp: 10}</var>
```

## 6. `<script>` 可执行节点语法点

### 6.0 宿主可见同级事件节点

对宿主/玩家流程来说，以下节点是同级事件来源：
- `<text>` -> `Text`
- `<choice>` -> `Choices`
- `<input>` -> `Input`
- `<end/>` -> `End`（游戏终结，运行立即停止）

其中 `<end/>` 用于显式声明“到此结束”。一旦命中，不再继续执行后续节点，也不会再进入 `choice/input` 边界。

## 6.1 `<temp>`

用途：声明变量。  
属性：`name`、`type`（必填），`format`（可选，`inline/xml`）。  
初值：  
- 缺省/`format="inline"`：使用节点内联表达式；非 enum 为空时使用类型默认值，enum 必须显式写 `Type.Member`。  
- `format="xml"`：结构化初始化（对象 `<field>`、数组 `<item>`、map `<tuple key>`）。

```xml
<temp name="hp" type="int">3</temp>
<temp name="title" type="string">"Knight"</temp>
<temp name="state" type="State">State.Run</temp>
<temp name="hero" type="Hero" format="xml">
  <field name="hp">3</field>
  <field name="name">"Rin"</field>
</temp>
```

`format="xml"` 规则：
- 对象类型仅允许 `<field name="...">expr</field>`，字段必须完整且不重复。
- 数组类型仅允许 `<item>expr</item>`。
- `#{string=>T}` 仅允许 `<tuple key="...">expr</tuple>`，`key` 按原样字符串解释。
- `#{Enum=>T}` 的 `tuple key` 必须是 `Type.Member`。
- `field/item/tuple` 默认使用内联表达式文本；也可在这些节点上写 `format="xml"` 做递归结构化初始化。
- 任意节点标注 `format="xml"` 后，不接受非空内联文本；必须使用结构子节点（允许空白文本）。
- `format="xml"` 时禁止和非空内联文本混用（空白文本可忽略）。

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

`<code>` 顶层 `let` 声明会写回当前脚本变量作用域，可被后续节点读取：

```xml
<code>let ok = true;</code>
<text>${ok}</text>
```

`script` 类型也可在 `<code>` 中赋值（值必须是 `@...` 脚本字面量）：

```xml
<temp name="nextScene" type="script">@main.start</temp>
<code>nextScene = @main.result;</code>
```

在 module 脚本上下文里可写 `@short`（如 `@next`），编译期会补全为 `@module.script`。

`function` 类型也可在 `<code>` 中赋值（值必须是 `*...` 函数引用字面量）：

```xml
<temp name="fnRef" type="function">*shared.add</temp>
<code>fnRef = *shared.mul;</code>
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

## 6.7 `<for>`

用途：循环语法糖（编译期展开为 `group + temp + while`）。  
属性：`temps`、`condition`（必填），`iteration`（可选）。  
`temps` 格式：`name:type:init;name2:type2:init2;`（每项必须是 `name:type:init`，允许尾分号）。  
语义：`iteration` 在每轮开始时执行，但第一次进入循环不执行；省略时等价 no-op。  
在 `<for>` 体内命中 `<continue/>` 后，下一轮开始仍会先执行 `iteration`。  
说明：`<loop>` 已移除。  

```xml
<for
  temps="i:int:0;limit:int:3;"
  condition="i LT limit"
  iteration="i = i + 1;"
>
  <text>tick-${i}</text>
</for>
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
属性：`var`、`text`（必填），`max_length`（可选，非负整数）。  
限制：不支持 `default` 属性，不允许子节点/内联文本。  
补充：
- `max_length` 按 Unicode 字符数量计数（实现口径：`chars().count()`）。
- 宿主提交输入后，若长度超过 `max_length`，运行时返回 `ENGINE_INPUT_TOO_LONG`。
- 用户输入空白时会回退到 `default_text`；回退后的值同样参与 `max_length` 校验。
- 常见“先定义 string temp，再 input”可用 `<temp-input>` 宏简写。

```xml
<temp name="heroName" type="string">"Traveler"</temp>
<input var="heroName" text="请输入名字" max_length="16"/>
<text>Hello ${heroName}</text>
```

### 6.11.1 `<temp-input>`

用途：声明一个 `string` 类型临时变量，并立刻对它发起输入边界。  
属性：`name`、`type`、`text`（必填），`max_length`（可选，非负整数）。  
限制：
- `type` 目前只能写 `string`。
- 不允许子元素；内联文本作为默认值表达式（为空时默认空串）。

编译期等价展开：
- `<temp name=\"...\" type=\"string\">...</temp>`
- `<input var=\"...\" text=\"...\" max_length=\"...\"/>`

```xml
<temp-input name="heroName" type="string" text="请输入名字" max_length="16">"Traveler"</temp-input>
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
- `script`（必填，仅支持 `@literal` 或 `script` 变量名）
- `args`（可选，位置参数）

`args` 支持：
- 值参数：`expr`
- 引用参数：`ref:path`
- `<call>` 目标脚本必须是 `kind="call"`

```xml
<call script="@battle.main" args="hp, ref:score"/>
```

```xml
<temp name="nextScene" type="script">@battle.main</temp>
<call script="nextScene" args="hp"/>
```

module 相关规则：
- 对外调用 module 脚本时使用 `@battle.main`
- 在同 module 内调用 sibling script 时使用 `@next`
- `@short` 会在编译期补全为当前 module 的限定名
- `@short` 仅可用于 module 脚本上下文；在非 module 脚本中使用会编译失败
- 若 `script` 来自变量，变量值需是 `@module.script` 形态（module 内写入 `@short` 时会在编译期补全）
- 可见性语义：
  - 静态字面量目标（`@module.script` / `@short`）仍遵循 import + export 可见性；
  - 动态变量目标（`script` 类型变量）按引用能力执行：变量里已持有合法脚本引用即可调用（可到 private 脚本）。
- 变量目标只支持“变量名”本身，不支持路径表达式（如 `a.b`）
- `script="battle.main"`、`script="${...}"` 会编译失败
- `script="next"` 只有在 `next` 是可见且类型为 `script` 的变量时才合法；否则会编译失败

## 6.15 `<goto>`

用途：跳转到新脚本（不新增调用栈）。  
属性：
- `script`（必填，仅支持 `@literal` 或 `script` 变量名）
- `args`（可选，位置参数，不支持 `ref:`）

规则：
- `<goto>` 目标脚本必须是 `kind="goto"`

```xml
<temp name="nextScene" type="script">@battle.next</temp>
<goto script="nextScene" args="heroName, hp"/>
```

```xml
<goto script="@battle.next"/>
```

module 相关规则与 `<call>` 相同：
- 对外跳转到 module 脚本时使用 `@battle.next`
- 同 module 内可写短名 `@next`
- `@short` 仅可用于 module 脚本上下文；在非 module 脚本中使用会编译失败
- 若 `script` 来自变量，变量值需是 `@module.script` 形态（module 内写入 `@short` 时会在编译期补全）
- 可见性语义：
  - 静态字面量目标（`@module.script` / `@short`）仍遵循 import + export 可见性；
  - 动态变量目标（`script` 类型变量）按引用能力执行：变量里已持有合法脚本引用即可跳转（可到 private 脚本）。
- 变量目标只支持“变量名”本身，不支持路径表达式（如 `a.b`）
- `script="${...}"` 已移除，动态目标用 `script` 类型变量承载

## 6.16 `<return/>`

用途：从当前 `kind="call"` 脚本返回到调用方。  
规则：
- 仅 `kind="call"` 脚本可使用
- 不支持 `script` / `args` 属性

```xml
<return/>
```

## 6.17 `<end/>`

用途：显式声明游戏终结；立即结束当前运行。  
规则：
- 仅 `kind="goto"` 脚本可使用
- 不支持属性和内容
- 命中后直接产出终止事件（`End`），宿主应停止继续驱动
- 脚本会在编译期做“终止结构校验”（非路径推理）：
  - `kind="goto"` 末尾结构必须能递归证明为 `<goto/>` 或 `<end/>`
  - `kind="call"` 末尾结构必须能递归证明为 `<return/>`
  - 末尾可递归节点：`<group>`、`<if>`（then/else 均需通过）、`<choice>`（每个 option 模板均需通过）
  - 末尾 `<while>` 一律不通过；`<if>` 无 `<else>` 视为空分支，不通过
  - 违规时报：`XML_SCRIPT_TERMINATOR_REQUIRED`

```xml
<end/>
```

## 6.18 `<group>`

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
<module name="shared" export="type:Hero">
  <type name="Hero">
    <field name="hp" type="int"/>
    <field name="name" type="string"/>
  </type>
</module>
```

## 7.2 `<enum>`

用途：声明枚举类型。  
属性：`name`（必填）。  
子节点：`<member name="..."/>`（至少 1 个，且名称唯一）。  
可出现位置：`<module>` 直接子节点。  

```xml
<module name="shared" export="enum:State">
  <enum name="State">
    <member name="Idle"/>
    <member name="Run"/>
  </enum>
</module>
```

规则：
- enum 的值语义为 member 名字符串：`State.Run` 在运行时等价于 `"Run"`。
- enum 结构化声明位点（如 `<temp>/<var>/<const>` 初值）必须写 `Type.Member`，不能直接写字符串字面量。
- `enum` 与 `type` 共享命名空间，重名会编译失败。

## 7.3 `<field>`

用途：定义类型字段。  
属性：`name`、`type`（必填）。  

```xml
<field name="hp" type="int"/>
```

## 7.4 `<function>`

用途：声明命名空间函数。  
属性：
- `name`（必填）
- `args`（可选，`type:name`）
- `return_type`（必填，类型表达式）

限制：
- module 函数 `args` 不支持 `ref:`
- module 函数 `return_type` 不支持 `ref:`
- 函数体只能是内联代码文本，不允许子元素

```xml
<module name="shared" export="function:add">
  <function name="add" args="int:a,int:b" return_type="int">
    return a + b;
  </function>
</module>
```

`script` 类型也可用于 `<function>` 参数和返回值：

```xml
<module name="router" export="function:pick">
  <function name="pick" args="script:current" return_type="script">
    if current == @router.main {
      return @router.alt;
    }
    return @router.fallback;
  </function>
</module>
```

`function` 类型也可用于 `<function>` 参数和返回值：

```xml
<module name="router" export="function:pick">
  <function name="pick" args="function:current" return_type="function">
    if current == *router.main {
      return *router.alt;
    }
    return *router.fallback;
  </function>
</module>
```

## 7.5 `<module><script>`

用途：在 module 内声明可执行脚本。  
可出现位置：仅 `<module>` 直接子节点。  

```xml
<module name="camp" export="script:main,rest">
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
<call script="@battle.main" args="hp + 1, ref:score"/>
```

规则：
- 对 `script="@module.name"` 这类编译期可静态定位目标脚本的调用，参数个数必须与目标脚本声明完全一致，否则编译报错。
- 对动态目标（`script` 类型变量）保持运行时检查。

## 9.3 `<goto args="...">`

```xml
<goto script="@next" args="hp, title"/>
```

规则：
- 对 `goto script="@module.name"` 这类静态目标转移，参数个数必须与目标脚本声明完全一致，否则编译报错。
- 对动态目标（`script` 类型变量）保持运行时检查。

## 10. 命名与约束语法点

## 10.1 保留前缀

`__` 前缀为保留命名，不可用于脚本名、类型名、函数名、变量名等。

```xml
<!-- 不建议/会被拒绝 -->
<script name="__internal">
  <text>x</text>
</script>
```

## 10.2 编译期上下文宏

`__script__` 为编译期内置变量宏，表示“当前脚本限定名”（例如 `main.main`、`battle.next`）。

规则：
- 仅在表达式上下文生效（如 `when`、`args`、`<code>`、`<temp>` 初始化、`${...}`）。
- 编译期直接替换为字符串字面量。
- 仅替换独立标识符；字符串字面量里的 `__script__` 不会替换。

```xml
<script name="main">
  <temp name="sid" type="string">__script__</temp>
  <text>${__script__}</text>
  <if when="__script__ == 'main.main'">
    <text>is main</text>
  </if>
</script>
```

## 10.3 已移除节点

以下节点已移除：`<loop> <vars> <step> <set> <push> <remove>`。

```xml
<!-- 会编译报错 -->
<script name="main">
  <set path="x">1</set>
</script>
```

## 10.4 XML 转义

ScriptLang 表达式层不再推荐 XML 转义写法，而是使用自己的保留字：

- `<` 写成 `LT`
- `<=` 写成 `LTE`
- `&&` 写成 `AND`
- XML 属性表达式里的字符串写单引号：`'Rin'`
  例如 `when`、`args` 等属性表达式。
- `<text>` 的 `${...}` 插值表达式里字符串写双引号：`"Rin"`。
- `<code>` / `<function>` / `<var>...</var>` / `<temp>...</temp>` 里的字符串写双引号：`"Rin"`

旧写法会在编译期报错（`XML_RHAI_*`）：

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

### 10.4.1 Rhai 规则校验阶段

Rhai 相关检查分为两层：

- 编译期（错误码前缀 `XML_RHAI_`）：
  - 预处理规则：`LT/LTE/AND`、单双引号规则、`@script` / `*function` 字面量规则。
  - Rhai 语法检查：表达式/代码块的语法错误会在编译时报错。
  - 覆盖上下文：
    - 脚本内表达式：`when`、`call/goto args`、`choice/dynamic-options` 表达式、`${...}`。
    - 脚本内代码块：`<code>`、`<temp>/<var>` 初始化。
    - module 级代码：`<function>` body、`<var>/<const>` initializer。
- 运行期（错误码前缀 `ENGINE_`）：
  - 仅保留动态依赖错误（例如运行态类型不匹配、只读写入、目标不存在等）。

示例 2：文本节点里显示尖括号/`&`

```xml
<text>使用 &lt;tag&gt; 语法，并用 A &amp; B 连接。</text>
```

## 10.5 函数引用与动态调用

用途：使用 `function` 类型承载函数引用，并通过 `invoke(fnVar, [args])` 动态调用。  
约束：
- `function` 字面量写法：`*module.func` 或 `*short`（仅 module 脚本上下文）。
- `*...` 只能作为“引用值”使用，不能直接调用。
- 静态调用写法只支持：`method(...)`、`module.method(...)`。
- `invoke` 第一参数必须是 `function` 类型变量名（`fnVar`），不支持字面量或字符串。
- 第二参数必须是数组字面量/数组表达式。
- `invoke` 按“引用能力”执行：当 `fnVar` 已持有合法函数引用时，可执行对应目标（包括 private 目标）。
- private 函数引用仍必须通过合法路径获得（例如 module 内部函数返回、已持有引用的变量转发）；非法字面量获取规则不变。
- 静态调用 `method(...)` / `module.method(...)` 仍遵循 import + export 可见性。
- 参数个数必须与目标函数声明一致。

```xml
<!-- import shared from shared.xml -->
<module name="main" export="script:main">
  <script name="main">
    <temp name="fnRef" type="function">*shared.add</temp>
    <temp name="v" type="int">invoke(fnRef, [3, 4])</temp>
    <text>${v}</text>
  </script>
</module>
```

常见错误：
- `invoke(*shared.add, [1])`：`invoke` 首参不是变量名。
- `invoke("shared.add", [1])`：`invoke` 首参不是 `function` 变量。
- `*shared.add(1)`：`*...` 不能直接调用。
- `invoke(fnRef, 1)`：`args` 不是数组。

## 10.6 脚本 kind 判断内建函数

用途：在表达式里判断某个脚本引用是否是 `call/goto` 类型。  
支持函数：
- `is_call_kind_script(scriptRef)`
- `is_goto_kind_script(scriptRef)`

约束：
- `scriptRef` 可传脚本变量（`type="script"`）或脚本字面量（如 `@main.next`）。
- 传入空值/无效值/不存在脚本时，返回 `false`。

```xml
<module name="main" export="script:main,callee,next">
  <script name="main">
    <temp name="callRef" type="script">@main.callee</temp>
    <temp name="a" type="boolean">is_call_kind_script(callRef)</temp>
    <temp name="b" type="boolean">is_goto_kind_script(@main.next)</temp>
    <text>${a},${b}</text>
  </script>
  <script name="callee" kind="call"><return/></script>
  <script name="next"><end/></script>
</module>
```

## 11. 综合示例

```xml
<!-- import shared from shared.xml -->
<module name="main" export="script:main">
  <script name="main" args="string:name">
    <temp name="hp" type="int">3</temp>
  <text once="true">你好，${name}</text>

  <for
    temps="boost:int:0;limit:int:2;"
    condition="boost LT limit"
    iteration="boost = boost + 1;"
  >
    <code>hp = hp + 1;</code>
  </for>

  <while when="hp > 0">
    <choice text="动作">
      <option text="攻击" when="hp > 1">
        <code>hp = hp - 2;</code>
      </option>
      <option text="防御" once="true">
        <continue/>
      </option>
      <option text="结束" fall_over="true">
        <goto script="@main.result" args="name, hp"/>
      </option>
    </choice>
  </while>
  </script>
</module>
```

## 12. 参考

- 运行示例：`crates/sl-test-example/examples/`
- Engine API：由 API 专题文档负责（见 README 导航）
