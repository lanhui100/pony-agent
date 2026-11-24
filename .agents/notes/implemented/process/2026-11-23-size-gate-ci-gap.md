# Agent Note: size 棘轮门在 CI 侧的空转缺口与收口顺序（B249 拆分）

Status: implemented

## Problem

`npm run verify` 里**包含** `npm run size:check`（`package.json:66`），但 CI 的 `verify` job 是
分步近似实现（版本同步 / 门自检 / ADR / 账本 / 单测 / 构建 / clippy 棘轮 / Core 测试…），
**没有** `size:check` 这一步。后果是机械可见的：`main` 上跑 `npm run size:check` 直接红：

| 位置 | 实测 | 基线允许 |
|---|---|---|
| `crates/pony-agent-core/src/agent/config.rs` | 3085 行 | ≤ 3078 |
| `crates/pony-agent-core/src/agent/process.rs:1303` `start_inner` | 280 行 | ≤ 278 |
| `crates/pony-agent-core/src/agent/provider/tests.rs` | 3057 行 | 基线无此条目（新增超限） |
| `crates/pony-agent-core/src/agent/runtime/tests.rs` | 13913 行 | ≤ 13902 |

`FILE-SIZE GATE: 4 new violation(s) / 60 over limit / 56 grandfathered` —— 而同期 `main` 的 CI
全绿（`verify` / `core-platform-check` / `governance` 均 success）。也就是说：**一个在 `verify`
里声明的门，在 CI 上从不执行**，它的红灯只对愿意在本地跑全量 `verify` 的人可见。

B249 登记的「size 棘轮再收口」正是这件事的收尾，但原描述只提「provider/turn_flow/control_plane/sqlite
抽取回基线」，没有覆盖「门不在 CI 里」这个更根本的缺口——只修 4 条数字，下个 PR 又会悄悄长回去。

## Decision

1. **先修 4 条超限，再接 CI**，顺序不可颠倒。
   - 先接 CI：`main` 立刻红灯，等于用一个新门把所有在途 PR 一起挡停（且真实原因不是他们造成的）。
   - 先修：把 4 条压回基线（真实抽取，**不**调高 `scripts/file-size-baseline.json`），
     再接线，接线那一刻起门才是"绿着上岗"。
2. **修复必须走抽取，不接受调基线**：B249 的语义就是"重锚后把抽取真正落地"，
   调高基线等于把棘轮变成装饰品。`provider/tests.rs`（3057）与 `runtime/tests.rs`（13913）
   属测试文件，优先把整组用例拆到同目录的兄弟文件（`tests/waveXX_*.rs` 之类），
   而不是压缩断言可读性。
3. **B249 拆成两个 PR**：
   - `PR-6c-1`：4 条超限的抽取（纯代码，`npm run size:check` 必须转绿）；
   - `PR-6c-2`：把 `npm run size:check` 加入 CI 的 `verify` 步骤，并附一条"故意超一行 → CI 必须红"
     的负样本说明（本仓既有 spec 风格：`scripts/*.spec.sh`）。
4. **窗口纪律**：`provider/tests.rs` 与 `runtime/tests.rs` 正被在途 PR 触碰（B139 的
   `upstream_status` 字段补齐），所以 6c-1 必须等这些 PR 合入后再动手——同一文件两处改动的
   冲突成本高于收益。

## Alternatives considered

- **直接调高基线让门变绿**：与 B249「收口」的语义相反；且这 4 条都是"最近长出来的"，
  调高等于承认棘轮失效。
- **先接 CI 再修**：`main` 立即红，所有在途 PR 的 `verify` 全红——把别人的绿灯当作自己缺口的代价。
- **只修 `config.rs` / `process.rs` 两条小的，测试文件登记进基线**：测试文件行数是这轮超限的主因
  （3057 / 13913），豁免它们等于放过最大两块；且"测试可以无限长"本身是坏示范。
- **把 `size:check` 从 `verify` 里删掉**：等于承认这个门不成立，但仓里仍有 56 条 grandfathered
  存量在靠它守——删门比空转更糟。
