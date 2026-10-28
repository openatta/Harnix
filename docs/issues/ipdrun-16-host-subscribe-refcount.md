# ipdrun-16：`useNest().subscribe` 的退订不计数，同一页里另一个订阅者从此收不到推送

类型：缺陷｜优先级：高

## 现象

外壳的宿主（`ui/packages/shell/src/host.ts:22-24` → `:61-74`）每次 `subscribe(topic, params, fn)` 都发一次 `harnix.subscribe`，
退订时发 `harnix.unsubscribe { topic, params }`。服务端按**连接**记订阅，同一条连接上同一个 `(topic, params)` 只有一份。

于是同一页里两个组件订同一个主题、同样的参数（例如列表和详情同时显示同一个对象），其中一个卸下时发出的 `unsubscribe` 把这条连接的订阅撤了，
另一个组件再也收不到推送，也没有任何错误。

另一个小问题：后来的订阅者要等自己那次 `harnix.subscribe` 的应答才拿到当前值，即使页面里已经有人持有它。

## 建议

不改接口，改实现：在外壳里按客户端维护一张 `(topic, 规范化的 params) → {count, last, listeners}` 的表（所有 `hostFor(...)` 共用，
因为它们共用一条连接）：

- 第一个订阅者：发 `harnix.subscribe`，应答的值记为 `last` 并分给所有监听者；
- 后来的订阅者：只加监听者、计数 +1；已有 `last` 就在下一个微任务里先给它一次；
- 收到 `harnix.topic`：更新 `last`，分给这个键的所有监听者；
- 退订：计数 −1，到 0 才发 `harnix.unsubscribe` 并删掉这一项；
- 重连：按表里的键重新 `harnix.subscribe`（若现有重连路径还没做）。

`params` 的规范化用键排序后的 JSON，避免 `{a,b}` 和 `{b,a}` 被当成两个订阅（现在 `host.ts:62` 的比较是直接 `JSON.stringify`）。

## 兼容性

- `NestHost.subscribe` 的签名与返回的退订函数不变。
- 只有一个订阅者时：线上行为与现在完全相同（一次 subscribe、一次 unsubscribe）。
- 多个订阅者时：从「一个卸下、其余失联」变成都正常。依赖失联行为的调用方不存在。
- Helm 的 space 里有用 `subscribe` 的地方（`spaces/helm/ui/src/layers/intel/model.ts`），不改代码，只会更稳。

## 验收

`ui/packages/shell/test/` 加一个 host 的单元测试（用假的 `HarnixClient`）：

- 两个订阅者订同一个 `(topic, params)`：只发出一次 `harnix.subscribe`；第一个退订后，服务端推一个新值，第二个仍然收到；第二个退订后才发出 `harnix.unsubscribe`。
- 后订阅的那个在第一个已经拿到值之后订阅：不等应答就拿到 `last`。
- `params` 键顺序不同视为同一个订阅。

另在 `tests/live/ui/` 里用 testapp 的一个主题做一遍端到端：两个组件、卸一个，另一个继续更新。

## 依赖

无。

## 来源

IPDrun 的同一个对象在会话里的卡片和右栏里同时显示、订同一个主题，一个卸下后另一个不再更新；IPDrun 现在在自己的 kit 里做引用计数（`ui/kit/src/runtime.ts` 的 `Kit.watch`）。

## 实现记录

后端（F3a）：本条全部在外壳（`ui/packages/shell/src/host.ts`），后端不改——服务端「按连接一份订阅」是对的，计数归客户端。留给外壳批次：
按客户端的 `(topic, 键排序后的 params JSON) → {count, last, listeners}` 表、首个订阅者才发 `harnix.subscribe`、最后一个退订才发 `harnix.unsubscribe`、
后来者在下一个微任务拿到 `last`、重连时按表重订；`ui/packages/shell/test/` 的 host 单测与 `tests/live/ui/` 的端到端用例（可用 testapp 的
`app.probe.counter` 主题与 `app.probe.bump`）。
