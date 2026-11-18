# Agent Note: file-size 棘轮再锚（B233/B239/B247 合并后的必要增长）

Status: implemented

工单：B233/B239/B247 收口（Lead 执行）；后续瘦身：B249。

## Problem

B233（run-bound 审批续轮）、B239（flush 并发幂等 + eventTimes 解码）与并发会话的 B247
（degraded 轮终态/重试预算收口）合并后，`npm run size:check` 出现 7 条新增/恶化：

| 文件 | 现值 / 基线 | 主因 |
|---|---|---|
| `agent/control_plane/mod.rs` | 10785/10737 | B239 幂等 flush 协议 + B233 挂起释放 helper（已外移部分到 event_buffer.rs） |
| `agent/provider/mod.rs` | 6994/6948 | B247 同步/流式 HTTP 客户端同口径（connect/read timeout、最小可用闸门） |
| `agent/runtime/tests.rs` | 13902/13894 | B214/B247 等价/降级用例增量 |
| `agent/session/tests.rs` | 6469/6463 | B247 历史重建用例增量 |
| `agent/sqlite_session.rs` | 6334/6320 | B239/ B247 事件时间与降级持久化接线 |
| `agent/turn_flow.rs` | 3711/3676 | B247 终态/降级事件接线 |

## Decision

按 B242 先例做**一次性重锚**（`--update-baseline`），逐条理由如上；同时登记 B249
「size 棘轮再收口」要求后续把这些增量抽回独立模块（优先 provider/mod.rs 与 turn_flow.rs），
避免重锚变成长期放行。重锚只影响 file-size 存量账，不改变阈值（3000 行/200 行/8 层）。

## Alternatives considered

- **A. 本波内全部抽取到位。** 落选：B233/B239 已按各自规模门禁抽过一轮（event_buffer、
  ask_resume 委托、graph helper），继续抽会扩大本波闭包并与 B249 的专项重构重复。
- **B. 不重锚，让 verify 保持红。** 落选：三张已合入 P0/P1 卡的必要功能增长不能作为长期红；
  红门会掩盖后续真新增。

## Verification

- `node scripts/check-file-size.mjs --update-baseline scripts/file-size-baseline.json` 后
  `npm run size:check` exit 0（0 new）；
- B249 登记后续收口要求与验收（各文件回到基线或逐项 ADR）。
