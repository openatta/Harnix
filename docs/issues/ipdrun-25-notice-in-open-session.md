# ipdrun-25：space 的提示只在新对话首屏出现，打开的会话里没有

类型：新能力｜优先级：中

## 需求

`SpaceContribution.notice()`（`ui/packages/react/src/contrib.ts:105-106`：「One line under the composer (what the Agent may and may not do here)」）
只画在新对话的首屏（`ui/packages/shell/src/Shell.tsx:409`，`.disc`）。打开一个已有会话以后，输入框下面就没有它了
（`Shell.tsx:598-620` 的 `ChatPanel` 的 `notice` 只用来放「去配置模型」的提示，`:613`）。

一句「这里不该说什么」的提醒，恰恰是在会话进行中最需要：人在第三十轮时和第一轮时一样可能说错话。不同的会话还可能需要不同的提醒。

## 建议

不改 `notice()` 现在的行为（Helm 等应用的首屏照旧），另加一个按会话的：

```ts
export interface SpaceContribution {
  // …原有成员不变
  /** 打开的会话里，输入框上方的提醒条；没有就返回 null / undefined。每次画对话时读。 */
  composerNotice?(session: SessionRef): ReactNode;
}
```

- 外壳把它与「去配置模型」的提示一起放进 `ChatPanel` 的 `notice`（模型未配置时两条都显示，模型提示在前）。
- 想让首屏那句也在会话里出现的 space，写 `composerNotice: () => notice()` 即可；SDK §8.2 写一句。
- 可选的简化：若维护者认为 `notice()` 本来就该在会话里也出现，可以改成「两处都画」并在发布说明里写明——那是一处缺省行为变化，本条不要求。

## 兼容性

- 不给 `composerNotice` 的 space：没有变化；`notice()` 照旧只在首屏。
- Helm 的 space 有 `notice()`（`spaces/helm/ui/src/layers/intel/index.tsx:42`），不受影响。
- 贡献点 API 版本：可选字段，可以不升。

## 验收

- `tests/live/ui/conversation.spec.ts`：testapp 的 space 给 `composerNotice` 返回「别贴密钥」；打开任一会话，输入框上方有这句；新对话首屏仍是 `notice()` 的那句。
- 模型未配置时：两条都在，顺序为模型提示、space 提醒。
- 不给 `composerNotice` 的 space：打开会话后输入框上方没有多出东西（回归）。

## 依赖

无。

## 来源

IPDrun 的一类会话要在输入框旁常驻一句「不要透露实现细节」；现在改写了 placeholder，并在右栏另放一条。
