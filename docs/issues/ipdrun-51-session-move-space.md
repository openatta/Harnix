# ipdrun-51：会话建了就不能换 space（`harnix.sessions.move`）

类型：新能力｜优先级：低

## 现象或需求

会话的 space 只在 `session.create` / `session.fork` 时绑定（`crates/hub/src/spaces.rs` 的 `bind_space`），之后没有公开方法改绑；
`session.fork` 在 `spaces::CHECKED` 里，带 `space` 时先按原绑定校验，claimed ≠ bound ⇒ `SPACE_MISMATCH`，也不能"fork 到别的 space"。
应用调整了 space 的划分（IPDrun 重构 R1 把 SR / 切片的条目会话从 `features` 挪到 `reqs`）以后，老会话在新 space 里一调用就是
`session … belongs to the space features, not reqs`，只能在新 space 另建一个会话，老会话的转录带不过来。

## 建议

中枢方法 `harnix.sessions.move { instance, session_id, space }` ⇒ `{ space, from, moved }`；模块侧 `Sessions::move_to(&SessionRef, space)`。

- 只给模块与内核：会话放在哪个 space 是应用的划分，不是某个客户端想在哪看；设备调 ⇒ `refused`。
- 目标 space 要存在（`exists` 同建会话），系统 space 不能当目标；会话的实例若属于某个 space（`Owner::Space`），只能挪到那个 space（同 `place` 的规则）。
- 会话要在（引擎的 `session.get` 认得，冷会话也行）；已经在目标 space ⇒ `moved: false`，什么都不改。
- 改账本（`.harnix/spaces.json`），推 `harnix.instances.sessions { instance }`，两边 space 的会话列表跟着变；转录、条目、任务都不动。
- 不在 `spaces::CHECKED` 里（`space` 参数是目标，不是声称）。

## 兼容性

新方法，旧的都不变；方法表里与其他中枢方法同一行（`Reach::Device` 的中枢方法，设备在中枢里被拒，同 `harnix.entries.append`）。

## 验收

- live API `tests/live/api/spaces.spec.ts`：设备调 ⇒ refused。
- IPDrun `crates/agent/tests/host.rs`：会话绑在别的 space 时，`Chat::ensure` 把它挪过来再恢复，会话号不变。

## 依赖

—

## 来源

IPDrun 重构 R1（C1-#33，草稿 `.local/issues/harnix-session-move-space.md`）；`harnix-sync.md` §4.4 最后一处 HARNIX-WORKAROUND；R12 收口（stage-R12.md §1.10）。

## 落地

（待记）
