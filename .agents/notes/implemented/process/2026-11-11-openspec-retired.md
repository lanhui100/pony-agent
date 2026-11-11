# Agent Note: OpenSpec 在本仓退役（冻结只读，执行层让位给 backlog 账本）

Status: implemented

## Problem

本仓的 OpenSpec 目录（`openspec/`，3.3MB / 351 文件）曾承担「细粒度行动计划 + `tasks.md` 执行级 checklist」的职责，与 `dev-backlog` 技能的三层模型（backlog / openspec / lane）绑定。但实际状态是：

1. **执行层真源重复**：事项层已在 `backlog/tasks.jsonl`（见既有记录 `.agents/notes/implemented/process/2026-11-11-unified-backlog-jsonl-ledger.md`），OpenSpec 的 `tasks.md` 与之语义重叠；
2. **门禁已空转**：`scripts/check-openspec.ps1` 首行即声明「DEPRECATED：openspec 校验已从工程体系移除」，`npm run verify` 不再调用它，残留脚本只是误导；
3. **维护成本存在**：新事项需要同时维护 change 目录与账本，且 `openspec validate` 不再被 CI 执行，写入即无人校验。

## Decision

1. **退役为冻结只读**：`openspec/` 保留文件与路径不变（`docs/INDEX.md`、部分 ADR 与任务卡以相对路径引用其历史 spec，删除会造成大面积断链），新增 `openspec/RETIRED.md` 标注 `Status: retired`，禁止新增 `changes/`。
2. **删除空转脚本**：`git rm scripts/check-openspec.ps1`（其自身已标 DEPRECATED 且无调用点）。
3. **执行层真源单一化**：新事项一律进 `backlog/tasks.jsonl`；执行级 checklist 由账本的 `description` / `touched_paths` 与任务级租约（`backlog/.locks/<ID>.lock`）承担，不再产生 `tasks.md`。
4. **技能契约按仓库生效**：全局 `dev-backlog` SKILL 顶部增加「OpenSpec 状态（按仓库）」注记——pony-agent 仓已退役，其余仓库若仍启用 OpenSpec 不受影响；不删除技能内的 OpenSpec 段落，避免污染其它仓库的流程。
5. **不迁移历史内容**：`openspec/` 的历史 spec 不搬进账本（散文与 checklist 无法机械校验，搬入只会把 351 个文件变成账本噪声）；历史结论以 ADR / 任务卡为准。

## Alternatives considered

- **彻底删除 `openspec/`（351 文件 / 3.3MB）**：落选——`docs/INDEX.md` 有 10+ 条相对路径链接、2 篇 guide 以其中的 spec 为「现行契约」引用，删除需同步改十余处文档且历史交付证据断链；收益（3.3MB）低于代价。
- **保持现状（继续双轨）**：落选——`check-openspec.ps1` 已自证空转，双轨写入无人校验，正是本次要消除的漂移源。
- **把 `openspec/changes` 全量导入 `backlog/tasks.jsonl`**：落选——历史 spec 是散文 + checklist，导入后无法通过账本 schema（无 fingerprint、无状态机语义），只会制造一批永远 `todo` 的僵尸任务。
- **删除全局 `dev-backlog` 技能中的 OpenSpec 段落**：落选——该技能是全局资产，被其它仓库（如 job_copilot）使用，其中 OpenSpec 仍可能是现行流程；按仓库注记才是正确的作用域。

## Consequences

- **物理变化**：新增 `openspec/RETIRED.md`；删除 `scripts/check-openspec.ps1`；`openspec/` 其余 350 文件保持原样、只读。
- **检索收据**：`rg -n "openspec" scripts/ .meta/ package.json tests/` → **0 命中**（门禁与脚本面已完全解耦）；`rg -n "check-openspec" --glob '!node_modules' .` → 仅命中 `management/task-system/**` 的历史日志与任务卡（历史叙述，非活引用）。
- **流程变化**：新事项 → `node scripts/backlog.mjs record`；执行中 → `start`（获取任务级租约）/`heartbeat`/`resolve`；不再创建 `openspec/changes/*`。
- **限制**：`docs/INDEX.md` 仍列出 OpenSpec 条目（历史索引，指向冻结文件）；两篇 guide 若再引用 `openspec/specs/*` 作为「现行契约」属过期表述，需在下次触及该文档时改为指向 ADR。

## Reintroduction conditions

满足任一即可评估复活：(a) 出现需要**跨多会话、多阶段**推进且账本 `description` 无法承载的规格（例如需被外部审计逐条核对的验收矩阵）；(b) 需要机器校验的分层 spec 校验器重新进入 CI（届时须同时恢复 `openspec validate --strict` 门禁与 `check-openspec` 脚本，不能只恢复目录）；(c) 账本因缺少执行级 checklist 导致同一事项被反复重开（可观测信号：同一 fingerprint 的 `occurred_count` 持续增长）。

## Removed surface

`scripts/check-openspec.ps1`（DEPRECATED 空转脚本，无调用点）——检索见上方 Consequences 的 `rg` 收据。
