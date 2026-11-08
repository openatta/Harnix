# Harnix

## 版本与发布

版本标识的形式是 `Harnix-v<x.y.z>-<YYYYMMDD><NN>`，例如 `Harnix-v0.3.2-2026102501`：

- `x.y.z`：每次发布只升 `z`；`x`、`y` 只有维护者明确要求时才升。
- `YYYYMMDD`：发布当天的日期。
- `NN`：当天第几次发布，从 `01` 起。用 `git tag -l 'Harnix-v*-<YYYYMMDD>*'` 查当天已有的标签，取最大序号加 1。

各 crate 的 `version` 写成 `x.y.z-YYYYMMDDNN`，不带 `Harnix-v` 前缀——Cargo 不接受前缀。版本写在每个
crate 自己的 `Cargo.toml` 里，不从 workspace 继承（应用把这些 crate 收进自己的 workspace 时，继承到的会是
应用的值），所以改版本用 `scripts/set-version.sh`，`tests/crate-metadata.mjs` 检查它们一致。

每个 Harnix 发布对应一个 Agix 发布，一一对应，不兼容多个 Agix 版本：子模块 `agix/` 锁在一个带 `Agix-v…` 标签的提交上，
升 Agix 就是出一个新的 Harnix 发布。`tests/crate-metadata.mjs` 检查锁定的提交恰好带一个 Agix 标签、且检出的就是它。

发布步骤：

1. 子模块 `agix/` 检出到主仓库锁定的那个提交（`git submodule update`），否则 `Cargo.lock` 记下的是工作区里碰巧检出的 Agix。
   升 Agix 时检出到它的发布标签（`git -C agix checkout Agix-v…`），再 `git add agix`。
2. `scripts/set-version.sh x.y.z-YYYYMMDDNN` 改所有 crate 的版本号，它会顺带 `cargo update -w`。
3. `scripts/check.sh` 通过（跑不了的层要说明，不能当作通过）。
4. 版本号改动单独一个提交。
5. 在这个提交上打附注标签（标签名就是完整的版本标识，标签消息第一行写对应的 Agix 标签，如 `Agix: Agix-v0.3.8-2026110801`，
   再写本次发布的要点），和分支一起推送。

### 开发分支

跨多个提交、要先在主线外做完的工作在独立分支上进行（如 `harnix-apps`：应用与插件系统），放在单独的 git worktree 里
（`git worktree add ../Harnix-apps harnix-apps`），构建目录也随之独立，不打扰主工作区。

- 开发分支不是发布：不改版本号、不打标签；分阶段的工作每阶段在分支上跑完 `scripts/check.sh`。
- 什么时候合回 `main`、合回时是否升 `x.y`，由维护者定；合回之后照上面的步骤从 `main` 发布。
- 并行的子任务可以各开一条从开发分支拉出的短分支（`harnix-apps-<部分>`），做完合回开发分支再删。

改名为 Harnix 之前的标签（`v0.3.0` 及更早、`nest-v0.3.1-2026102501`）保留原样，不改名。

## 开发

- `docs/` 只放架构与二开的文档：架构与理由在 `docs/concept_and_architecture.md`，用词照 `docs/glossary.md`，
  二开接口在 `docs/harnix-sdk.md`、`docs/harnix-sdk-reference.md`，贡献点 `docs/contribution_points.md`，测试 `docs/testing.md`。
  版本迁移写在 `docs/migrations/<x.y.z>.md`（要应用动手的版本各写一份，登记在 `docs/migrations/README.md` 的表里）：只在本地，
  已在 `.gitignore`，不提交。
  过程（计划、进度、设计记录）不进 `docs/`。下游项目的需求在 GitHub issue 里记（每个项目一个标签），
  `docs/issues/README.md` 只是代码里的编号与 issue 的对照表。代码注释里的「设计 §x」「roadmap Mx-y」指 0.4 的设计记录与路线，
  已从仓库移出，在 git 历史里。
- 改了 `README.md` 或 `docs/README.zh-CN.md`，两边一起改，再 `node tests/readme-pairing.mjs --write`。
- 开始一个新的里程碑，要维护者明确说开始。
- 界面照 `../Helm/docs/design/11-gui-workbench-v0.1.md` 的工作台框架；Harnix 本身做薄，产品面是应用的 space。
- 构建缓存及时清：里程碑或发布之后跑 `scripts/prune.sh`。
