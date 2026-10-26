# 测试：LIVE 框架

> 设计见 [0.4 设计 §13](harnix-0.4-design-and-plan.md)。这里写怎么用：怎么跑、怎么写、怎么录像、覆盖表怎么读。

---

## 1. 一句话

**功能测试都对着真的 `harnix` 二进制跑。** 不用假 DOM，不 mock 中枢，不用假后端。后端经它自己的
协议测（`api` project，不开浏览器），界面在 chromium 里测（`ui` project）。只有一个运行器：
Playwright Test。

`@harnix/core` 的 Vitest 是单元测试，**不算功能覆盖**。

## 2. 跑

```sh
pnpm install && pnpm install:browser   # 第一次
pnpm live                              # api 和 ui 两个 project，模型调用回放
pnpm live --project api                # 只跑 API
pnpm live --project api --topology split_streams   # API 套件走分离流
pnpm live --live                       # 连 .env 里的真模型
pnpm live --no-model                   # 不碰模型：要模型的套件跳过（只给没有录像的机器用）
pnpm live api/hub.spec.ts              # 一个文件；其余参数原样交给 playwright test
scripts/check.sh                       # 所有层，从便宜到贵
```

`pnpm live` 每次都问一遍 cargo 要不要重建（cargo 是唯一知道要不要重建的东西；对着昨天的二进制跑出的
绿比不跑更糟）。刚构建过可以设 `HARNIX_BIN=target/release/harnix` 跳过。

### 三种模型模式

| 模式 | 用途 | 怎么选 |
|---|---|---|
| **回放**（默认） | 确定、离线、不花钱。引擎按位置重放 `tests/live/fixtures/recordings/` 里的调用 | 不加参数 |
| live | 刷新录像、冒烟 | `--live`（`HARNIX_MODEL=live`），读 `.env` |
| 无模型 | 没有录像的机器 | `--no-model`。它看着像回放模式，覆盖却少得多，别误用 |

`ui` project 永远回放：一个布局测试因为模型慢了而失败，大家就学会了无视它。

标着 `replayed: true` 的 API 套件在 live 模式下也自己起一个回放的后端：它的价值就是每次在同一处
调同一个工具，对着真模型就成了另一个同名的测试。标着 `needsLiveModel` 的只在 `--live` 下跑
（`turns` 要真实耗时的轮，`recordings` 要的是录像**被写下**，回放的会话不写）。

## 3. 结构

```text
ui/packages/core/            @harnix/core：协议类型（生成）、协议客户端；Vitest
ui/packages/testkit/         @harnix/testkit
  src/launcher.ts            startHarnix：独立的 HOME、数据根、引擎根、项目、端口、拓扑、回放目录
  src/api.ts                 connect：基于 @harnix/core/client 的测试客户端；finish、inconclusive；覆盖计数
  src/fixtures.ts            Playwright 夹具：harnix（每个 worker 一个）、api（每个用例一个）、mode
  src/suite.ts               defineSuite：0.3 套件形状的适配
tests/live/
  playwright.config.ts       两个 project：api、ui
  api/<名字>.spec.ts          API 套件
  ui/<名字>.spec.ts           界面套件
  fixtures/recordings/       模型录像
  features.mjs               功能模块的行，以及每一层为什么可以是零
  coverage.mjs → coverage.md 覆盖表（生成）
  record-fixture.mjs         录一份新的录像
```

## 4. 写一个测试

### 新写的：直接用夹具

```ts
import { expect, finish, test } from "@harnix/testkit";

const TAGS = { tag: ["@feature:sessions", "@layer:api"] };

test("a new session is idle and in the list", TAGS, async ({ api }) => {
  const { session_id } = await api.call("session.create", { scene: "coding", project_root: api.project });
  try {
    expect((await api.call("session.get", { session_id })).turn_state).toBe("idle");
    const { sessions } = await api.call("harnix.sessions");
    expect(sessions.some((s) => s.session_id === session_id)).toBe(true);
  } finally {
    await finish(api, session_id);
  }
});
```

界面测试拿 `page` 和 `harnix`：`await page.goto(harnix.url)`。

要一个配置不同的后端（种子数据、只开一种拓扑、带某个 profile 段），在用例里
`startHarnix({ seed, topologies, profile })`，用完 `stop()`。

### 0.3 迁过来的：`defineSuite`

0.3 的 17 个 API 套件原样搬了过来（M1-6），形状保留，好让「迁移后的套件与迁移前逐条对得上」
可以一眼核对：

```ts
defineSuite("hub", { feature: "sessions" }, {
  needsModel: true,           // 没有模型也没有录像时跳过
  setup: async ({ backend }) => ({ /* 并进每个用例的上下文 */ }),
  tests: {
    "what this shows": async ({ client, backend, replay }) => { /* 抛错即失败 */ },
  },
});
```

Playwright 会把这些用例的位置记成 `suite.ts`（`test()` 在那里调用）。看用例名前面的套件名即可。

### 客户端

- `call(method, params)`：被拒就抛出 JSON-RPC 错误。
- `refused(method, params)`：返回错误，走通了返回 `null`。
- `waitFor(predicate, { timeout, describe })`：等一帧；`describe` 是超时消息里写的「在等什么」。
  **等条件，不等时钟**：这是「慢」和「不稳」的区别。
- `eventsOfKind(kind, sessionId)`、`text(sessionId)`：收到了什么。
- `handshake`、`project`、`backend`。

每个用例**自己一个客户端**。事件日志是按客户端记的，共用一个时，一个用例能匹配到前一个用例产生
的帧，有两个用例就这样隔一次绿一次红。

### 收尾

起过一轮的用例都以 `finish(client, sessionId)` 结束：中断，**等结算**，再删除。删一个还在跑的
会话，引擎会继续为一个不存在的东西干活，下一个用例来买单。

### 三种结果，不是两种

通过、失败，以及**不确定**：权限测试的模型压根没去调工具，就没有证明权限是好的；报绿是撒谎，报红
会让人学会无视红。抛 `inconclusive(reason)`，报告里是跳过，原因写在注释里。

## 5. 覆盖表

每个用例带两个标签：`@feature:<id>`（[设计 §4.3](harnix-0.4-design-and-plan.md) 的一行）、
`@layer:api|ui`。`tests/live/coverage.md` 由标签生成：

```sh
pnpm --filter @harnix/live-tests coverage          # 重新生成
node tests/live/coverage.mjs --check               # scripts/check.sh 跑的就是它
```

`--check` 在这几种情况下失败：

- `coverage.md` 和标签算出来的不一样（删了一个用例、加了一个没重新生成）；
- 用例没带标签，或者标了不存在的 `feature`；
- 某一行在某一层是零，`features.mjs` 里又没写 `na`（不适用，写理由）或 `pending`（哪个里程碑补）；
- 一行已经有用例了，还标着 `pending`。

### API 方法覆盖

跑完 `api` project 会打印两组数，比的是后端 `harnix.reachable` 说客户端能调的方法，不是手写的清单：

```text
── API coverage ──
  56/56 reachable methods called (100%)
  25/56 also driven into an error (45%)
```

第二个数存在，是因为第一个数太容易满足。一个只回答过、从没被拒绝过的方法，参数没被检查过，前置
条件没被测过，拒绝路径没走过。**它不会也不该到 100%**：不带参数的方法（`harnix.hello`、各种
`*.list`）没有失败可走。它的用处是：加了一个有参数的方法，这个数掉了，那就是信号。

## 6. 录像

```sh
node tests/live/record-fixture.mjs <名字> "<提示词>"    # 需要 .env
```

起一个 headless 的 harnix，在独立项目里建会话，发提示词，所有权限提问一律允许（否则工具调用的
录像停在提问那里），轮结束后把录像拷到 `tests/live/fixtures/recordings/<名字>/`。用例建会话时带
`options: { recorder: { name: "<名字>" } }`，回放模式下的后端就取这一份；不在回放模式时这个名字被
忽略，调的是真模型。

录像按位置回放。每个后端都有自己的数据根、HOME 和项目，全局记忆与 telemetry 不会串进来（N-003）。
隔离以后回放仍然错位（同一轮里可选的旁路调用时有时无），就向 Agix 提「按调用种类匹配」，不在
Harnix 侧绕过。

## 7. 协议类型

`@harnix/core/protocol` 的 `generated/contract.ts` 由 `harnix-contract` 生成：

```sh
cargo test -p harnix-contract --features ts       # 改了 harnix-contract 的类型后
```

`scripts/check.sh` 的 `frontend` 层把它生成到临时目录再比对，手改或忘了重新生成都会失败。
`ts-rs` 只在 `ts` feature 下链接，不进发布的二进制。
