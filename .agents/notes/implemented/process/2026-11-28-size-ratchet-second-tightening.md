# Agent Note: size 棘轮第二次收口——按实际值下调基线并清掉两条可瘦身项

Status: implemented

## Problem

`scripts/file-size-baseline.json` 是规模债的**棘轮**：只允许「删条目 / 下调 value」，
上调必须显式说明（`npm run size:check:update` 后逐项 review diff）。B249 登记的问题是：
B233/B239/B247 三次重锚（含 #99 的 6 层 crate 拆分、#90 的 204 个集成测试整合）之后，
`provider` / `turn_flow` / `control_plane` / `sqlite` 这几处的基线**已明显高于实际值**——
棘轮松了，等于给未来增长留了免费额度。

## Decision

按当前实际值重跑一次 `npm run size:check:update`，并逐项核对 diff（下表为全部变化）：

| 方向 | 条数 | 说明 |
|---|---|---|
| 下调 | 20 | 全部是「实际值 < 基线」，无一条上调 |
| 删除 | 2 | `pony-tools/src/governed_executor.rs::decide_with_context`（206）、`pony-storage/src/session/store.rs::append_turn_fallible_with_kind`（204）——函数已降到阈值内，条目不再需要 |
| 上调 | 0 | —— |
| 新增 | 0 | —— |

代表性下调：`control_plane/mod.rs` 10789→10740、`provider/mod.rs` 6907→6699、
`turn_flow.rs` 3713→3359、`sqlite_session.rs` 6335→6324、`session/store.rs` 5777→5397、
`runtime/turn_stream.rs` 3465→3404、`otel/mod.rs` 4427→4410。

结论：这是**单向收紧**，没有任何一条把上限抬高。收口后
`npm run size:check` = `0 new violation / 56 over limit / 56 grandfathered`。

## Alternatives considered

- **只删两条失效条目、保留下调项不动**：棘轮语义会长期停在旧水位，
  下次任何小改动都能在「基线以下」偷偷长回去，等于自废棘轮。
- **继续做抽取（把 control_plane 降到 3000 行阈值内）**：那是真正的拆分工程
  （该文件 10740 行、97 个命令面），与本条「先把松掉的棘轮拧紧」是两件事；
  在收紧之前先做大拆分，会让 diff 既含结构调整又含基线变化，review 无法分辨。
- **不做任何事（现状 0 new violation 已绿）**：绿的是「没有新增违规」，
  不是「额度已收紧」；B249 要的正是后者。

## Consequences

- 棘轮水位 = 当前实际值，任何后续增长立刻判红（`0 new violation` 从「松」变「紧」）。
- 条目数 58 → 56；文件行数阈值与函数阈值不变（3000 / 200 / 8）。
- 下次重锚必须同样在 PR 里贴 diff 表，否则无法区分「真增长」与「忘了收紧」。

## Verification

- `npm run size:check` → `FILE-SIZE GATE: 0 new violation(s) / 56 over limit / 56 grandfathered`
- 基线 diff 全量核对：0 上调、0 新增、20 下调、2 删除（见上表）
