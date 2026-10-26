# Nest

## 版本与发布

版本标识的形式是 `nest-v<x.y.z>-<YYYYMMDD><NN>`，例如 `nest-v0.3.1-2026102501`：

- `x.y.z`：每次发布只升 `z`；`x`、`y` 只有维护者明确要求时才升。
- `YYYYMMDD`：发布当天的日期。
- `NN`：当天第几次发布，从 `01` 起。用 `git tag -l 'nest-v*-<YYYYMMDD>*'` 查当天已有的标签，取最大序号加 1。

`Cargo.toml` 的 `[workspace.package] version` 写成 `x.y.z-YYYYMMDDNN`，不带 `nest-v` 前缀——Cargo 不接受前缀。

发布步骤：

1. 子模块 `core/` 检出到主仓库锁定的那个提交（`git submodule update`），否则 `Cargo.lock` 记下的是工作区里碰巧检出的 AttaCore。
2. 改 `Cargo.toml` 的版本号，`cargo update -w` 同步 `Cargo.lock`。
3. `scripts/check.sh` 通过（跑不了的层要说明，不能当作通过）。
4. 版本号改动单独一个提交。
5. 在这个提交上打附注标签（标签名就是完整的版本标识，标签消息写本次发布的要点），和分支一起推送。

`v0.3.0` 及更早的标签保留原样，不改名。
