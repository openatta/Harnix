# ipdrun-55：再点前台 space 的入口没有反应（`SpaceContribution.reenter`）

类型：新能力｜优先级：中

## 现象或需求

ipdrun-54 让 IPDrun 的类目 space 不在左栏列它的主会话（入口就代表它）。可在 space 里打开了某个条目的会话之后，
再点这个 space 的入口什么都不发生（它已在前台），于是没有办法回到主会话——原来是点左栏里的那一行。

## 建议

`SpaceContribution.reenter?(): void`：前台 space 的入口被再点一次时调用（应用回到自己的首页与主会话）；不在前台照旧切换。
没给 ⇒ 照旧什么都不做；抛错忽略。

## 兼容性

只加可选字段。

## 验收

shell 的 vitest 过（前台入口再点调 reenter，别的入口照旧切换）；IPDrun 需求 space 里打开条目会话后再点「需求」回到列表与类目会话（e2e uf08-10 A5）。

## 落地

落地提交：见 README 总表（main，2026-11-06）。
