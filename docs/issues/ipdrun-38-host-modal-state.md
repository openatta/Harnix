# ipdrun-38：space 不知道外壳的模态层开着，Esc 同时落到 space 的键盘处理上

类型：新能力｜优先级：中

## 现象或需求

1. 外壳的模态层（设置、会话菜单这类对话框，`ui/packages/shell/src/Shell.tsx:976` `ModalHost`，`#veil` / `app.snapshot.modal`）
   开没开着，宿主 API 里读不到，也没有开合的通知。space 自己有键盘快捷键（例如 Esc 退回上一层、方向键挪选中项）时，
   只能去看 `#veil.on` 这类 DOM 细节，免得模态框开着时还在响应。
2. 模态层开着时按 Esc：外壳在 `window` 的冒泡阶段关模态框（`Shell.tsx:133`），而 space 挂在 `document` 上的监听先收到同一个事件，
   于是 Esc 既关了模态框，又让 space 退了一层。

## 建议

```ts
interface NestHost {
  modal: {
    /** 外壳自己的模态层开着。 */
    isOpen(): boolean;
    /** 开了或关了；返回退订。 */
    onChange(fn: (open: boolean) => void): () => void;
  };
}
```

- 模态层开着时 Esc 只关它：焦点在模态框里，由模态框自己的 `onKeyDown` 关并停止传播（模态框里的部件自己先处理 Esc 的仍然可以）；
  焦点在模态框外，由 `window` 捕获阶段的监听关并停止传播。冒泡阶段挂在 `document` / `window` 上的监听都收不到。
- 没有模态框时什么都不变。

## 兼容性

`NestHost` 只加成员。行为变化只在「模态层开着 + Esc」：以前这个 Esc 也会传到页上其他冒泡阶段的监听，现在不会（这正是要修的）。
在捕获阶段挂在 `document` 上的监听仍然会先看到焦点在模态框里时的 Esc。

## 验收

`ui/packages/shell/test/modal.test.tsx`：

- `modal.onChange` 只在开 / 关时各通知一次；`isOpen()` 跟着变。
- 渲染外壳，`document` 上挂一个 keydown 监听：模态框开着时，从页上和从模态框里的输入框按 Esc，模态框都关上、监听都收不到；
  没有模态框时按 Esc，监听照常收到。

## 依赖

无。

## 来源

IPDrun 的 space 在 `document` 上处理 Esc 与方向键，要先知道外壳的模态层没开（`ui/kit/src/compat/modal.ts`，HARNIX-WORKAROUND C1-#5）。

## 落地

落地提交：`bc75665`（main，2026-11-01，未推送；版本待下次发布）。
外壳：`App.onModal`（`set` 里模态框有无变化时通知），`hostFor` 的 `modal`；`useShortcuts` 加 `window` 捕获阶段的 Esc，
`#modal` 加 `onKeyDown`（`ui/packages/shell/src/Shell.tsx`）。
