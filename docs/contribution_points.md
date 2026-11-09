# 界面贡献点

Harnix 的界面由八个具名的贡献点拼起来（0.4 的 v2：在 v1 的六个之上加了 `space` 与 `home.card`）。

**这首先不是给第三方代码用的扩展面。** 扩展 agent 是 Agix 的事——脚本、manifest、能力门控、沙箱、披露，全在那边；
Agix 插件的 `[[ui]]` 段是几个预编译的 ES 模块，Harnix 把它们发给浏览器，与页面同级（[概念](concept_and_architecture.md) §5.8，产品可以用
`[ui] runtime_modules = false` 整个关掉）。插件 APP 的界面也落在这些点上，但商店的插件不直接注册：它的面与行内点由外壳照清单替它注册
（见文末）。这些点存在的理由是别的：让界面不是一堆 switch。工具行、流块、详情面板、
侧栏分组、命令、设置节，每一样都是一次具名注册，外壳问注册表而不是自己认识每一种情况。
换一整套界面因此是换一批注册，不是改一堆分支。

## 这张表怎么读

**频率是约束，不是花边。** 一个会话触发一次的点养得起很多东西；每个流式分片触发一次的点
什么都养不起。所以这里没有一个点跟着流式增量走——一轮十几次谁都养得起，一轮几千次谁都
养不起。

**八个，加第九个是一个需要论证的决定。** 一个没人用的点不是接缝，只是这张表里的一行，所以每个点
都要有人在用：内建的会话、项目、系统 space 注册 `space`，其余六个也都有内建注册。`home.card` 是例外：
M8 去掉 HOME 之后它没有内建注册了，外壳在新对话的输入框下读它，`examples/hello-app` 注册了一张。

令牌覆盖（`r.tokens`）与语言包（`r.strings`）不是贡献点：它们改的是主题和词典本身，不在任何时机被「问」。

**外壳必须真的去问。** 一个没人问的点，等于只存在于目录里，产品照旧自己直接画——这件事
真的发生过一次，七个点里的五个，而所有测试都是绿的。`ui/packages/shell/test/points.test.ts` 现在盯着它（外壳读了每个点），`ui/packages/react/test/contrib.test.ts` 盯着点的清单与这张表一致。

<!-- BEGIN GENERATED TABLE — see harnix_module::catalog::render_markdown -->

| 贡献点 | 类型 | 贡献什么 | 求值时机 | 频率 |
|---|---|---|---|---|
| `space` | 注册 | 一个 space:左栏入口与条目、右栏页签、中区模式、新会话的默认值 | 界面启动时 | 每进程(10⁰ 量级) |
| `home.card` | 呈现 | 新对话输入框下面的一张卡 | 新对话的空白页打开时 | 每次交互(10⁰–10¹ 量级) |
| `tool.row` | 呈现 | 一次工具调用在流里的那一行:图标、标题、摘要 | 该工具块状态变化时 | 每次块状态变化(10¹ 量级) |
| `details.panel` | 呈现 | 详情栏里的一页 | 选中项变化时 | 每次交互(10⁰–10¹ 量级) |
| `flow.block` | 呈现 | 流里的一种新块 | 该块状态变化时 | 每次块状态变化(10¹ 量级) |
| `sidebar.group` | 注册 | 会话列表的分组与排序 | 会话集合变化时 | 每次交互(10⁰–10¹ 量级) |
| `command` | 注册 | 一条 `/` 命令或 `@` 引用的补全候选（`trigger` 选哪个菜单） | 命令面板打开时 | 每次交互(10⁰–10¹ 量级) |
| `settings.section` | 呈现 | 设置面板里的一节 | 设置页打开时 | 每次交互(10⁰–10¹ 量级) |
<!-- END GENERATED TABLE -->

上面这张表由 `harnix_module::catalog` 生成，这份文件一旦和它对不上，
`crates/harnix/tests/catalog_doc.rs` 就会失败。要改改 catalog，别改表。

---

## 认领的顺序

`tool.row`、`details.panel`、`flow.block` 三个点靠 `match(subject)` 认领。**后注册的先被
问到**，所以一个更具体的注册能盖掉一个更一般的。

由此得出一条不能忘的规则：**兜底必须最先注册。** `flow.block` 里那个"什么都认"的 note
排在第一位，于是它最后一个被问到；它要是排在最后，就会把 user、assistant、tool 全吞掉。
一个未知种类的块画成 note，而不是画成空白——空白让看到的人没有任何办法弄清发生了什么。

## 失败的注册只让它自己消失

`match()` 或 `render()` 抛异常，只有这一份贡献消失，别的什么都不受影响。连续失败若干次的
注册被置于一旁。**而且这些都记下来了**，设置面板的诊断一节能问出来：哪个点、谁、什么原因。
一个悄悄不出现的界面部件，是这里唯一真正难查的失败方式。

## 宿主 API

每个注册拿到的是一份小而可枚举的 API：挂载点、按可达方法过滤过的 RPC 客户端、store 订阅、
设计令牌、`t()`、图标集、信封读取。

**拿不到连接。** 所以同一份代码在任何传输拓扑下都一样跑——这不是巧合，是 §3.3 那条
"语义写死、拓扑可选"在界面这一侧的体现。

两条硬规矩：**只用令牌，不写死颜色**（写死颜色的部件在另一种主题下不可读）；**只用 key，
不写死文案**（写死文案让整个界面在切换语言后变成两种语言）。`tests/style-lint.mjs` 与
`ui/packages/react/test/i18n.test.ts` 检查词典这一半（两本键相同、占位符相同、不留空），`tests/style-lint.mjs` 检查令牌这一半。

## 插件 APP 落在哪些点上

插件 APP（[插件](harnix-plugin.md)）不加新的贡献点：外壳照它的清单，替它往这八个点上注册，注册的 owner 是应用的 id。

**面**（`[[surface]] point`，画在隔离的 iframe 里；iframe 的沙箱、CSP 与面的生命周期见 [插件](harnix-plugin.md) §6.2）：

| 面 | 落在哪个贡献点上 |
|---|---|
| 清单写 `space` | `space`：它的入口与中区（`overview`；`center = "chat"` 时中区是对话） |
| 清单写 `space.sidebar` | 那个 `space` 的 `sidebar()` |
| 清单写 `space.tab` | 那个 `space` 的 `tabs` 里的一页 |
| 清单写 `space.secondary` | 那个 `space` 的 `secondary()` |
| 清单写 `details.panel` | `details.panel`，只认它自己 `kinds` 的项 |
| 清单写 `settings.section` | `settings.section` |
| 清单写 `home.card` | `home.card`（自动高度） |
| 清单写 `modal` | 不是贡献点：外壳的对话框，页面经桥的 `openModal` 打开 |

**行内点**（`[inline]`，外壳用自己的组件照声明画，不跑插件的代码）：

| 行内点 | 落在哪个贡献点上 |
|---|---|
| 清单写 `tool_row` | `tool.row`：它的某个工具那一行 |
| 清单写 `flow_block` | `flow.block`：它写的某种扩展条目画成卡片，按钮调它的方法 |
| 清单写 `command` | `command`：候选来自它的后端方法 `complete` |
| 清单写 `sidebar_group` | `sidebar.group`：只分组它自己 space 的会话列表 |

受信发布者的插件 APP 另可以带进页面的 `[[ui]]` 模块，和 Agix 插件的界面模块一样直接注册。

## 装 Agix 插件

Agix 插件是引擎的，不出现在这份目录里。Harnix 在这件事上只做一件
Agix 有意不做的事：**收文件**——`plugin.install` 自己去取 URL，没有上传通道，而一个人
的包在自己笔记本上、后端在另一台机器上时就没有路可走。`harnix.plugins.upload` /
`harnix.plugins.install` 就是那条路，别的全部转手给引擎，包括它装完之后回的那份披露。
