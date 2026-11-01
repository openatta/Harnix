# ipdrun-36：`composer.insert` 不能指定会话，刚打开的会话插不进去

类型：新能力｜优先级：中

## 现象或需求

`host.composer.insert(text)` 插进「此刻页上的那个输入框」（`ui/packages/shell/src/host.ts:71`，`app.composer` 由
`useBindComposer` 绑，`ui/packages/shell/src/Shell.tsx:309`）。外壳按会话给中区的对话换一个 key（`Shell.tsx:208`
`<Conversation key={keyOf(...)}>`），`session.open(ref)` 之后新会话的输入框要等下一次画完才挂上；在那之前调
`insert`，字落进旧会话的输入框，或者（旧的已经卸下、新的还没绑上）直接丢掉。space 只能自己等新输入框出现再插。

## 建议

```ts
composer: {
  /** 不给 session：同以前，插进页上的输入框。给了：那个会话的输入框在页上就插进去；不在就替它存着
   *  （已有存着的就接在后面），它挂上时放进去。在 session.open 之前、之后调都行。 */
  insert(text: string, options?: { session?: SessionRef }): void;
};
```

- 外壳按会话键记「哪个会话的输入框在页上」（打开的会话的输入框绑上 / 卸下时记 / 删，卸下只删自己那一份）；
  另记每个会话待插的字，那个会话的输入框绑上时插进去并清掉。
- 不给 `session` 的调用完全照旧。

## 兼容性

只加一个可选参数；不给它的调用行为不变。待插的字只在页面内存里，刷新即没。贡献点 API 版本不必升。

## 验收

`ui/packages/shell/test/composer-insert.test.ts`：

- 不给 `session`：插进页上的输入框（同以前）。
- 给 `session`：在 `open` 之前与之后各插一次，旧会话的输入框收不到；新会话的输入框绑上时一次收到两段拼起来的字；之后再插直接进去；
  重新绑一次不会再插一遍。

## 依赖

无。

## 来源

IPDrun 的「先打开会话再往输入框里放一句」（`ui/kit/src/goahead.tsx` 的 `insertWhenOpen`，HARNIX-WORKAROUND C1-#32）：
现在是等新会话的输入框挂上再插。

## 落地

落地提交：`03c7c6b`（main，2026-11-01，未推送；版本待下次发布）。
外壳：`App.insert(text, session?)` 与 `App.composerBound / composerGone`（`ui/packages/shell/src/app.ts`），
`useBindComposer(app, sessionKey)` 让打开的会话的输入框按会话绑上（`Shell.tsx`）。
