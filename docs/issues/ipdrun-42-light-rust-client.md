# ipdrun-42：没有轻量的 Rust 客户端

类型：新能力｜优先级：中

## 需求

应用的命令行（`<app> open / check`）、集成测试、桌面壳都要以客户端身份连上自己起的那个 Harnix 进程：split_streams 握手（进程令牌换凭据）、
`/rpc`、启动链接（`/auth/link`）、事件流（`/events/session`、`/events/host` 的 SSE）。`harnix` crate 把整个服务端拉进来，这些调用方都不想链；
于是每个调用方手写一份握手，还要自己对上 `protocol_version` / `contrib_api_version`（桌面壳不链 Harnix，只好从源码里抠常量）、自己拆 chunked 应答。

## 建议（新增 crate，只加不改）

`crates/client`（`harnix-client`）：只依赖 `harnix-contract` 与 `serde_json`；同步、`std::net`，一个请求一条连接，明文 HTTP（本机与局域网）。

```rust
let client = harnix_client::Client::connect("http://127.0.0.1:7801", &token)?;   // 握手，版本取 harnix-contract 的常量
let v = client.call("harnix.instances.list", json!({}))?;                         // /rpc；认 chunked；错误带 code 与 data
let v = client.call_within("app.x.migrate", json!({}), Duration::from_secs(120))?;
let link = harnix_client::launch_link(url, &token)?;                              // 完整地址
let mut events = client.events(Face::Host, Duration::from_millis(500))?;           // SSE，读超时给 StreamItem::Idle
// ClientError { kind: Transport | Handshake | Unauthenticated | Rpc, code, message, data }
// feature "tokio"：AsyncClient（同样的调用放在 spawn_blocking 里）、launch_link_async
```

## 兼容性

新 crate，服务端不改。应用的 workspace 收 `harnix/crates/*` 作成员时它跟着进来（`scripts/sync-app-deps.mjs` 生成依赖表时带上 `harnix-client`）。
不支持 TLS：跨网络的客户端经代理，用正经的 HTTP 栈。

## 验收

- `crates/client` 单测（回环上的替身服务）：握手带令牌与本 crate 的版本；握手被拒（401、版本不对）说明原因；调用带凭据、读 chunked 应答（含多字节字符）；
  错误保留 code / data，凭据失效 ⇒ `Unauthenticated`；没人监听 ⇒ `Transport`；启动链接是完整地址；事件流跨慢写（多字节字符被拆在两次写之间）给出整帧、
  中间给 `Idle`，服务走了报错；事件流的凭据被拒 ⇒ `Unauthenticated`。
- `crates/harnix/tests/layering.rs`：`client` 只认 `harnix-contract`，不链服务端与 HTTP 框架。

## 依赖

无。桌面模板用它见 ipdrun-43。

## 来源

IPDrun 的 `ipdrun open / check / mcp-config` 与 `app/tests` 手写一份握手客户端，桌面壳又手写一份（HARNIX-WORKAROUND C1-#30，N5）。
