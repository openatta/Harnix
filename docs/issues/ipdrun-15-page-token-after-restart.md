# ipdrun-15：服务重启后，已经打开的页面永远连不回去

类型：缺陷｜优先级：中

## 现象

页面的令牌在载入时由服务端注入 `<meta name="harnix-token">`（`ui/packages/core/src/client/client.ts:449-453` 的 `tokenFromDocument`，
外壳在 `ui/packages/shell/src/main.tsx:34` 用它建客户端）。令牌是每进程一个（`crates/app/src/run.rs:406`），客户端里是只读的
（`client.ts:86`、`:111`）。

服务停了再起（同端口）：

- 新进程换了令牌，旧令牌的 `/ws` 升级被拒（`crates/transport/src/ws.rs:57-69`，401）；
- 浏览器的 WebSocket API 拿不到 401，客户端只看到 `onclose`，按退避一直重连（`client.ts:355-378`），用的还是旧令牌；
- 页面一直显示「已断开」，直到人手动刷新。

桌面壳、进程管理器重启服务、开发时重编重启，都会撞上。

## 建议

只加不改：

1. `HarnixClientOptions` 加一个可选的取令牌函数：
   ```ts
   /** 重连失败时取一次当前的令牌；返回 null 表示拿不到。缺省不取，行为同现在。 */
   refreshToken?: () => Promise<string | null>;
   ```
   连接曾经打开过、断了以后重连又失败时调它（每次失败最多调一次）；拿到的令牌与当前不同就换上再连。`token` 字段由 `readonly` 改为内部可变，
   公开类型不变。
2. `@harnix/core` 再导出一个现成的实现：
   ```ts
   export function tokenFromPage(baseUrl?: string): () => Promise<string | null>;
   // fetch(`${base}/`, {credentials: "same-origin"})，从返回的 HTML 里读 meta harnix-token
   ```
   同源请求带上启动链接的 cookie，与载入页面时同一条授权；服务还没起来时 `fetch` 失败，返回 null，继续退避。
3. 外壳在 `main.tsx` 里传 `refreshToken: tokenFromPage()`。
4. 退路：连续 N 次（例如 10 次）重连失败且取不到新令牌时，状态条显示「服务已重启，请刷新页面」（新文案键），不再只是「已断开」。

## 兼容性

- 不传 `refreshToken` 的客户端：行为与现在一样。外壳传了，所以外壳里所有 space 都受益，不改代码。
- 令牌不经新的端点下发：仍然只在页面 HTML 里，授权条件与载入页面相同，没有扩大暴露面。
- 启动链接模式（`--launch-link`）下页面本来不带令牌，靠会话 cookie（`crates/transport/src/statics.rs` 的 `LaunchLink` 一支）；
  cookie 是上一个进程发的，新进程不认，`tokenFromPage` 取到 401 就返回 null，走第 4 条的退路提示。这一模式的自动恢复不在本条范围。
- 重连成功后，各订阅与会话的恢复走现有的重连路径（与网络断开后恢复相同）。
- Helm、hello-app：页面行为改善，不改代码。

## 验收

- `ui/packages/core/test/client.test.ts`：模拟服务端在第一次连接后换令牌、拒绝旧令牌；带 `refreshToken` 的客户端在下一次重连时换上新令牌、状态回到 `open`；
  不带的客户端保持 `closed`（回归）。
- `tests/live/ui/interface.spec.ts`：用 testkit 的 `restart()`（同端口）重启服务，页面不刷新，30 s 内连接状态回到 `open`，发一条消息能收到回答。

## 依赖

无。

## 来源

IPDrun 的「服务重启后界面恢复」用例只能在重启后 `page.reload()`（`qa/ui/specs/stage0/uf11-realtime.spec.ts`），桌面版里服务重启后窗口一直显示断开。

## 实现记录

后端（F3a）：

- 不用改：令牌只在页面 HTML 的 `<meta name="harnix-token">` 里，每次 `GET /` 注入当前进程的令牌，且该应答已带 `Cache-Control: no-store`
  （`crates/transport/src/statics.rs`）；旧令牌的 `/ws` 升级与 `POST /handshake` 都答 401。`tokenFromPage()` 需要的服务端行为都在。
  启动链接模式下没有 cookie 的 `GET /` 答 401，合「走退路提示」。
- 测试（锁住这份契约）：`tests/live/api/page-token.spec.ts` 的「after a restart the page carries the new token, and the old one is refused」
  （同端口 `restart()` 后 `GET /` 的 meta 是新令牌、`no-store`；旧令牌握手 401，新令牌 200）。

外壳部分待做（建议第 1–4 条全部在 `ui/`）：`HarnixClientOptions.refreshToken?: () => Promise<string | null>` 与重连失败时换令牌（`token` 改内部可变）；
`@harnix/core` 导出 `tokenFromPage(baseUrl?)`（`fetch(`${base}/`, {credentials: "same-origin", cache: "no-store"})` 读 meta，401 / 失败返回 null）；
外壳 `main.tsx` 传 `refreshToken: tokenFromPage()`；连续 N 次失败且取不到新令牌时的「服务已重启，请刷新页面」文案；`ui/packages/core/test/client.test.ts`
与 `tests/live/ui/interface.spec.ts` 的用例。

外壳（F3b）：

- `HarnixClientOptions.refreshToken?`（`ui/packages/core/src/client/client.ts`；`token` 改为内部可变，公开类型不变）：连接打开过、之后某次重连的 socket
  没打开时调一次，取到的与当前不同就换上再连。`@harnix/core` 导出 `tokenFromPage(baseUrl?, fetch?)`（同源 `GET /`、`credentials: "same-origin"`、
  `cache: "no-store"`，读 meta；非 2xx 或连不上返回 null）。外壳 `main.tsx` 传 `refreshToken: tokenFromPage()`。
- 退路：带 `refreshToken` 的客户端上次打开以来连续 10 次重连失败、又没取到新令牌时，发一次带说明 `TOKEN_STALE`（导出）的 `closed`；外壳据此显示横幅
  「服务已重启，请刷新页面」（`conn.restarted`），重新连上时收起。重连照旧继续。
- 只管单双向流（`/ws`）这一路；分离流的重连在 `connectSplit` 里，不在本条。
- 测试：`ui/packages/core/test/client.test.ts` 的「服务换了令牌」四条（换上新令牌回到 open；不带的保持连不上；取不到时带 `TOKEN_STALE`；`tokenFromPage` 的三种应答）；
  `tests/live/ui/host.spec.ts` 的「after the backend restarts on its port with a new token, the open page reconnects by itself」（同端口 `restart()`，页面不刷新，
  客户端换上新进程的令牌，回到 open，新建会话能收到回答）。偏离：放在 `host.spec.ts` 而不是 `interface.spec.ts`。
