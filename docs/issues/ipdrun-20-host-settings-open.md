# ipdrun-20：space 不能打开设置面板的某一节

类型：新能力｜优先级：中

## 需求

模块可以经 `settings.section` 注册自己的设置页（`ui/packages/react/src/contrib.ts:118`）。space 里经常要一个「去设置」的入口：
缺配置时的提示条、空状态里的按钮。外壳自己有 `openSettings(app, section?)`（`ui/packages/shell/src/settings.tsx:26-29`，
「去配置模型」就是这么打开的，`Shell.tsx:249`、`:339`），但它不在宿主 API 里；space 只能去点头像 → 设置 → 再点左边那一节。

## 建议

```ts
export interface NestHost {
  // …原有成员不变
  settings: {
    /** 打开设置面板；给了 section 就选中它（`settings.section` 的 id，或内建的 "models" 等）。没有这一节时打开第一节。 */
    open(section?: string): void;
  };
}
```

实现就是转给 `openSettings(app, section)`。内建各节的 id 在 SDK §8.3 列出来（它们现在就是注册在 `settings.section` 上的，有 id）。

## 兼容性

- 纯新增；`openSettings` 的行为不变。
- 贡献点 API 版本：只加成员，可以不升。
- Helm、hello-app 不受影响。

## 验收

- `tests/live/ui/interface.spec.ts`：testapp 注册一个 `settings.section`（id `testapp`）；space 里的按钮调 `host.settings.open("testapp")`，
  设置面板打开、左侧选中的是这一节，右侧是它的内容。
- `host.settings.open("models")` 打开内建的模型页；`open("nope")` 打开第一节、不抛错。

## 依赖

无。

## 来源

IPDrun 的 space 里有「去设置」的入口，现在由 kit 依次点头像、设置、再点「IPDrun」那一节（`ui/kit/src/shell-bridge.ts` 的 `openSettings`）。

## 实现记录

外壳（F3b）：

- `NestHost.settings.open(section?)` 转给 `openSettings(app, section)`；不是已注册的 `settings.section` id 时打开第一节。内建各节的 id：`general`、`scenes`、
  `models`、`mcp`、`plugins`、`devices`、`diagnostics`、`about`（`ui/packages/shell/src/settings.tsx` 的 `builtinSections`，SDK 参考里列出）。
- 测试：`tests/live/ui/host.spec.ts` 的「settings.open opens the panel at a section, a built-in one too, and the first for one that is not there」。
  偏离：`settings.section` 在页面里注册（id `probe`），不改 testapp。
