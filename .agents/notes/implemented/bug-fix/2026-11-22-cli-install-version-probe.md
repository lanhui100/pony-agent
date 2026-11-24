# Agent Note: 安装收尾探测两 bin 版本并在漂移时告警（B261 后半）

Status: implemented

## Problem

`pony` 与 `ponycli` 由同一个 workspace 构建、同一次 `npm run cli:build` 安装，但它们是**两个独立文件**：
`~/.local/bin/pony` 与 `~/.local/bin/ponycli`。`scripts/install-cli.mjs` 逐个原子替换
（v0.1.203 修复了 `Text file busy` 导致的静默安装失败），却**不报告装上去的是什么版本**。

后果：自更新或手工只装一个 bin 后，两个文件版本不同，用户与脚本都无法感知——B261 称之为
「随 pony 自更新后静默版本漂移（同包两 bin 无一致性提示）」。仓库侧门禁（`npm run version:check`）
只能保证**源码四处文件一致**，对「机器上装着的二进制」没有话语权。

## Decision

1. **安装收尾处探测两者自报版本**：`install-cli.mjs` 对刚替换的每个 bin 执行 `<bin> --version`，
   取行尾版本 token，打印 `[install-cli] <bin> <version>`。
2. **不一致即告警**：两个已探测版本不同 ⇒ 打印
   `警告：两 bin 版本漂移（pony A ≠ ponycli B）—— 请重跑 npm run cli:install 使两者同步。`
3. **宽容而非 fail-closed**：探测失败（旧版 `ponycli` 没有 `--version`，或二进制无法执行）
   只打印「无法探测版本」并继续，**不**把安装判红——安装动作本身已成功（大小校验仍非零退出），
   不能因为「读不到版本」回滚一次成功安装。
4. **探测放在 rename 之后**：Linux 上旧进程仍持旧 inode，新启动的 `--version` 进程一定拿到新二进制，
   因此探测结果反映的是「装完之后的状态」，不是构建目录里的文件。

## Alternatives considered

- **只在构建时比较产物（`cmp` 两个二进制或比对 mtime）**：能发现「只替换了一个」，但回答不了
  「装上去的是哪个版本」，也无法在下次人工排查时自证；用户要的是可读的版本号。
- **探测失败即非零退出**：会让 v0.1.203 及更早的 `ponycli`（无 `--version`）在升级路径上装不进去，
  属于用新契约追溯惩罚旧产物。
- **在 `pony --version` 里顺带探 `ponycli`**：把安装期问题搬进运行期，且每次启动/查询都多一次进程
  （桌面环境可达数百 ms）；安装收尾只跑一次，成本结构更合理。
- **写一个 `<to>/VERSIONS` 清单文件**：多一份派生状态，需要与二进制同步维护，且用户 `cp` 单个二进制时会说谎；
  直接问二进制本身是唯一真源。
