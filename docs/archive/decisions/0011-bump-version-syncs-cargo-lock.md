# 0011 版本 bump 自动同步 Cargo.lock

Status: implemented

## 背景

86f9bfd 更新了 `crates/pony-agent-core/Cargo.toml` 的版本号但未同步根
`Cargo.lock`，仓库进入 manifest（0.1.80）/ lock（0.1.79）自相矛盾状态：
任何 `--locked` 构建直接失败，且此后任意一次 cargo 命令都会把这笔"补账"
混进无关提交的 diff。根因是 bump-version.ps1 只写 Cargo.toml /
package.json / .version.json，对 lockfile 没有任何同步逻辑。

## 候选方案

| 方案 | 结果 |
|------|------|
| A. bump-version.ps1 写完版本文件后自动执行 `cargo metadata --format-version 1` | **采纳**。零新增依赖；只刷新 workspace 本地成员条目，不升级第三方依赖版本；失败降级为 Warning 提示手动补跑，不阻塞 bump 本身 |
| B. 维持人工纪律："改完 Cargo.toml 记得手动刷新 lock" | 落选——86f9bfd 正是该路径的失败实例，靠人记必然复发 |
| C. CI 增加 `--locked` 门禁拦截不一致 | 落选（暂缓）——反馈周期晚于提交，不一致进入历史后才被发现；可作未来加固，不能替代源头修复 |

## 决策

bump-version.ps1 在版本文件写入完成后、git stage 之前，检测到本次 bump
触及任何 `Cargo.toml` 时，自动从仓库根运行 `cargo metadata` 刷新根
`Cargo.lock`，并将 lock 文件计入 changedFiles（随 `-Stage` 一并入库）；
cargo 失败时输出 Warning 要求提交前手动补跑，不使 bump 失败。

## 影响

- bump 流程新增对 cargo 的软依赖：cargo 缺失或 metadata 失败不阻塞 bump，
  但提交前必须人工确认 lock 已同步；
- 根 `Cargo.lock` 从此应随任何 workspace 成员版本变更出现在同一提交内。
