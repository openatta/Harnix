# Harnix

## 版本与发布

版本标识的形式是 `Harnix-v<x.y.z>-<YYYYMMDD><NN>`，例如 `Harnix-v0.3.2-2026102501`：

- `x.y.z`：每次发布只升 `z`；`x`、`y` 只有维护者明确要求时才升。
- `YYYYMMDD`：发布当天的日期。
- `NN`：当天第几次发布，从 `01` 起。用 `git tag -l 'Harnix-v*-<YYYYMMDD>*'` 查当天已有的标签，取最大序号加 1。

各 crate 的 `version` 写成 `x.y.z-YYYYMMDDNN`，不带 `Harnix-v` 前缀——Cargo 不接受前缀。版本写在每个
crate 自己的 `Cargo.toml` 里，不从 workspace 继承（应用把这些 crate 收进自己的 workspace 时，继承到的会是
应用的值），所以改版本用 `scripts/set-version.sh`，`tests/crate-metadata.mjs` 检查它们一致。

发布步骤：

1. 子模块 `agix/` 检出到主仓库锁定的那个提交（`git submodule update`），否则 `Cargo.lock` 记下的是工作区里碰巧检出的 Agix。
2. `scripts/set-version.sh x.y.z-YYYYMMDDNN` 改所有 crate 的版本号，它会顺带 `cargo update -w`。
3. `scripts/check.sh` 通过（跑不了的层要说明，不能当作通过）。
4. 版本号改动单独一个提交。
5. 在这个提交上打附注标签（标签名就是完整的版本标识，标签消息写本次发布的要点），和分支一起推送。

改名为 Harnix 之前的标签（`v0.3.0` 及更早、`nest-v0.3.1-2026102501`）保留原样，不改名。

## 0.4 开发

- 进度记在 `docs/status.md`：接手前先读它，停下前更新它。计划在 `docs/roadmap.md`，架构在
  `docs/harnix-0.4-design-and-plan.md`，用词照 `docs/glossary.md`。
- 开始一个新的里程碑，要维护者明确说开始。
