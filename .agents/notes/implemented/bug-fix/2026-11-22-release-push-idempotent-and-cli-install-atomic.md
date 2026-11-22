# Agent Note: 发版通道两处硬伤——`--push` 不再自行 bump，CLI 安装改原子替换

Status: implemented

## Problem

2026-11-22 v0.1.203 实发过程中，发布面暴露两处「出错即污染、失败还静默」的缺陷：

1. **`npm run release:push` 会先 bump**。`scripts/release.mjs` 的 `--push` 只是给「先 bump 再推」这条路径加了个开关：
   脚本先跑完整 bump（写四个版本文件 + `cargo metadata` + commit + `git tag`），之后才校验 `HEAD` 是否为
   `origin/main` 祖先。合并发版 PR 后再执行 `release:push`，即使一切正常也会在本地 main 上多造一个
   `0.1.204` 的 bump commit 和一个孤儿 tag；一旦祖先校验失败（常见：忘记先合并、或 fetch 落后），
   脚本 exit 1，但脏状态已经落地，只能手工 `git reset --hard origin/main` + `git tag -d` 恢复。
   发布通道的「校验」发生在破坏性写入之后，等于没有校验。

2. **`npm run cli:build` 用 `cp` 覆盖运行中的二进制**。`cp` 对正在执行的 `~/.local/bin/pony` 会抛
   `Text file busy`；由于它是 `cargo build && cp && cp` 链式命令，失败被 npm 折叠，人只看到 cargo 成功，
   二进制其实没换（实测 exit 0 + 未替换，属于静默安装失败）。这正是用户按下「更新」后以为生效、
   实际跑旧二进制的场景。

两者共同点：**发布面是唯一不可「试错后回滚」的通道，却用了「先写后验 + 非原子写」的实现**。

## Decision

**① `--push` 变成纯推 tag 的只读模式**（`pushTagOnly()`）：读 `.version.json` 算出 `vX.Y.Z`，随后只做只读校验——

- 本地无该 tag → exit 1，并打印完整发版五步（不再代替人 bump）；
- `git fetch origin main` 后校验 tag 提交是 `origin/main` 祖先，读不到 `origin/main` 或校验不通过 → exit 1；
- 远端已有同名 tag：指向同一提交 → 幂等 exit 0；指向不同提交 → exit 1 并打印「以谁为准」的两条人工命令（不自动 force）；
- 通过后只执行 `git push origin refs/tags/<tag>`。

任何失败分支都**不写工作区、不动 HEAD、不建 commit/tag**——回滚指引不再需要，因为无状态可回滚。
bump 路径（`npm run release`）保留原语义，但不再内含 push，末尾只提示「先走 PR 合并，再 `release:push`」。

**② 新增 `scripts/install-cli.mjs` 替换 `cp`**：同目录临时文件 `dest.new-<pid>` → `chmod 755` → `rename` 原子替换
（对运行中进程安全，旧进程继续持旧 inode，不会出现半截文件）；装完比对源/目标字节数，失败一律 exit 1。
替换**前**扫描 `/proc/<pid>/exe` 采集占用者（rename 后路径会变成 `<path> (deleted)`，事后扫不到），
命中则打印「旧进程仍在运行（pid …），重启当前实例后新二进制才生效」。`cli:build` 改为
`cargo build … && node scripts/install-cli.mjs`，`cli:install` 别名不变。

**③ 两处都配负样本自检**，接入 `npm run verify` 前置段：
`scripts/release-push.spec.mjs`（沙箱 bare origin + 5 组正负样本：缺 tag / 未合入 / 正常推 / 幂等重跑 / 远端指向冲突，逐条断言 HEAD、tag 集、工作区脏状态不变）、
`scripts/install-cli.spec.mjs`（首次装、覆盖装、缺产物 exit 1、目标被运行中进程占用仍装成功且提示 pid）。
沙箱注入点 `PONY_RELEASE_REPO_ROOT` 仅供 spec 使用，生产不设。

## Alternatives considered

- **保留「bump + push 一条命令」，只在失败时自动 `git reset`/`tag -d` 清理**：否决。清理代码本身要判断
  「哪些 commit/tag 是我刚造的」，在多会话/多 worktree 场景下极易误删他人的 tag；而且默认发版动作
  会写 main 工作区，与「main 只能经 PR 前进」的契约在语义上就冲突。把破坏性动作从验前挪到验后（即删除）
  不如直接不产生。
- **`--push` 保持现状，仅把祖先校验提前到 bump 之前**：能挡住「未合并」这一种，但合并后误触仍会
  产生一个无人认领的 bump commit + tag（本次实测正是这种：PR 已合并，脚本却仍 bump 了 0.1.204）。
  半修不解决问题。
- **`cli:build` 里改 `cp -f` 或 `install -m 755`**：`install` 走的是同一条「打开目标路径写入」路径，
  对运行中的可执行文件同样 `Text file busy`（Linux `ETXTBSY`），Windows 上则直接拒绝替换。只有
  「新 inode + rename」能绕开，所以必须落到脚本里而不是换一个 shell 命令。
- **装前先 `pkill pony` 强杀旧进程**：否决。会打断用户正在进行的会话（TUI 里可能有未落盘的对话），
  且「谁该被杀」无法在安装器里安全判定；提示重启把决定权交回用户。
- **把校验放在 release workflow（CI）侧**：CI 已经在 tag 上构建，但「本地是否已造出脏 commit/tag」
  是本地事实，CI 看不见；缺陷发生在本地写盘那一刻，只能在本地拦。

## Consequences

- `release:push` 的语义从「bump 后推」收窄为「只推已存在的 tag」：误触最坏结果是「一条报错 + 零副作用」。
  代价是流程多一步心智负担——bump 与 push 必须分别执行（已在 `AGENTS.md` §4 与 git-flow 手册里如此描述，
  代码终于与文档一致）。
- 发版前若本地 tag 缺失（例如从零重新检出），`release:push` 不再「顺手修好」，必须回到 `npm run release`；
  这是刻意的 fail-closed。
- `cli:build` 失败不再静默：`cp` 的 exit 0 假成功消失，取而代之是「缺产物/替换失败/大小不符」三种明确非零退出。
- 占用检测仅覆盖 Linux `/proc`；macOS/Windows 下退化为「不提示」，但原子替换本身照样工作。
- 新增两个自检后，`npm run verify` 前置段多约 2 秒（两组沙箱都在 tmpdir 内，无网络）。
