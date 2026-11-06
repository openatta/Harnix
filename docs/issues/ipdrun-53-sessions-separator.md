# ipdrun-53：左栏的 space 入口与下面的会话之间太挤，没有分隔

类型：界面｜优先级：低

## 现象或需求

左栏上部是固定的功能入口（space 列表），下部是前台 space 的列（没有自己的列时是它的会话，按最近使用排）——一个是固定导航，
一个是随使用变化的动态区。两者现在紧挨着（`Sidebar.tsx` 里 `SpaceList` 后面直接是 `#sessions`），看起来像一个列表，太紧密。

## 建议

`SpaceList` 与 `#sessions` 之间加一条分隔线（与"新会话"按钮下面那条同样式 `.sep`），上下留多一点距离；收起成图标栏时隐藏
（会话列本来就隐藏）。只有前台 space 存在时才画。

## 兼容性

只加一个元素与一条样式：`<div class="sep chats-sep" role="separator" data-testid="sessions-sep">`，`#side .chats-sep { margin: 10px 8px 6px }`。
按 `#side > .sep` 数分隔线的测试或样式要排除 `.chats-sep`。

## 验收

shell 的 vitest 过（新用例：分隔线在 `nav.spaces` 与 `#sessions` 之间）；IPDrun 左栏 space 入口与会话之间有线、有间距。

## 落地

落地提交：见 README 总表（main，2026-11-06）。
