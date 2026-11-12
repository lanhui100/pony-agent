# Agent Note: 宪法不变量 4 改为 backlog-first（移除 OpenSpec 强制）

Status: implemented

## Problem

`.meta/constitution/constitution.md` 的不变量 4 仍要求「非平凡功能改动必须有 OpenSpec 规范」，
而 OpenSpec 已于 2026-11-11 退役（`2026-11-11-openspec-retired.md`，`openspec/RETIRED.md` 冻结，
不再产生 change）。每个会话都会经 `ponygo sync` 把该宪法投影进 `AGENTS.md` / `CLAUDE.md` 常载区，
于是 agent 被引向一套不存在的工作流；同时「账本必须留痕、闭环必须给证据」这条纪律只散落在 ADR 与
skill 中，不在最高层承诺里——宪法与现行机制互相矛盾。

## Decision

1. 不变量 4 改写为 backlog-first：非平凡改动必须有 ADR 决策记录；执行拆分与闭环记入
   `backlog/tasks.jsonl`（`record → start → resolve`，闭环必须写证据）；弃用或被替代的设计移入
   `superseded/` 或 `archive/`。
2. 常载命约新增第 5 条：非平凡交付必须在账本留痕（record 立项 / start 声明 owner 与 touched_paths /
   resolve 写入证据），账本一致性由 `backlog-gate`（lint + doctor）机械校验。
3. 修改只落 `.meta/constitution/constitution.md`，随后用 `ponygo sync` 投影到
   `AGENTS.md` / `CLAUDE.md` 的哨兵块；不手改镜像（手改会在下次 sync 被覆盖）。

## Alternatives considered

- **只改 AGENTS.md 投影体**：投影区由 `ponygo sync` 覆盖，手改会被抹掉，且宪法与入口分叉 → 落选。
- **保留 OpenSpec 条款并标注「已退役」**：宪法是现在时承诺，保留一条自我否定的条款只会让 agent
  判断分裂（读宪法的人与读 AGENTS.md 的人得到两种结论）→ 落选。
- **把账本纪律写进 `docs/AGENTS.md`**：文档标准管「事实放哪个家」，不管「交付必须留什么证据」；
  那是承诺层，应进宪法 → 落选。

## Consequences

- 宪法与两处投影同步更新；`ponygo status` 的 1.9 判定与本次无关（基线本就因
  `crates/pony-agent-sdk` 缺包文档而红，不属本次范围）。
- 命约 5 的可验证性由 `backlog-gate` 承接；「真的留痕且证据成立」仍靠 review——门只判记录合法性。
