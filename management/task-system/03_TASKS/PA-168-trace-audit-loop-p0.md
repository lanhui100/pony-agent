# PA-168: Trace 审核 Loop P0 —— 测量地基与维度 6 首轮

## 状态
- **状态**: 进行中（loop-state / corpus / skill 入口 / 首条 collect 回执已落盘；维度 6 断言立法与完整 round 待推进）
- **优先级**: P1
- **Complexity**: B
- **创建日期**: 2026-11-08
- **前置依赖**: 无（PA-164 headless 契约已在主干，replay 复用不阻塞）
- **决策依据**: [ADR 0101](../../../docs/decisions/0101-trace-audit-loop-automation.md)（已批准，用户唤起形态）

## 需求背景

七维度 trace 审核 loop 已批准为中期持续机制。缺口：无持久 loop 状态、无固定重放任务集、无逐维度断言命令、无唤起入口。维度 6（上报链路自检）是测量地基——它未通过前，其余维度的测量都不可信，不得自动化其上。pa_01a1a134 的 P1-3（上传 e2e 门禁）未闭环，由本卡维度 6 首轮承接。

## 目标

建成可跨会话断点续跑的 loop 地基，并以人工驱动方式走完维度 6 首个完整 round（collect → audit → propose → 批准 → implement → build → verify → verdict）。

## 交付契约

- **C1 状态文件**：`docs/audit/loop-state.json` 为单一真值源；字段与阶段流转按 ADR 0101；每次迁移 CAS 写入并落回执；lease 防双唤起。
- **C2 corpus**：`docs/audit/corpus/` ≥3 个只读重放任务 + manifest（逐任务 hash）；corpus_hash 在提案批准时冻结，漂移 = 红。
- **C3 唤起入口**：`.agents/skills/trace-audit-loop/SKILL.md` 定义程序；新会话说「继续 trace 审核」即可凭状态文件续跑（不依赖对话记忆）。
- **C4 维度 6 断言**：阈值与断言命令经批准提案立法（`docs/audit/rounds/r1-dim6/proposals/`），非零退出语义；覆盖重放 vs 实时属性对齐、开关关闭零出口、脱敏、release/session 归因。
- **C5 首个完整 round**：人工驱动各阶段，每阶段回执（命令 + 退出码 + 产物路径）落 `docs/audit/rounds/r1-dim6/receipts/`；verdict 只认断言退出码。
- **C6 归属声明**：`docs/observability.yaml` 更新 loop 自举归属说明。

## 非目标

- 不脚本化 replay/fetch/assert/build/verdict（PA-169）。
- 不做整轮自动推进与 patrol（PA-170）。
- 不引入定时器、不建常驻 dev team。

## 验收标准（物理收据）

1. 新会话唤起验收：仅凭 `loop-state.json` 正确续跑或 no-op（await_approval/escalated 停点行为正确），lease 冲突时拒绝双跑。
2. corpus manifest hash 与文件实算一致（`sha256sum` 对拍）；任务全部只读。
3. 维度 6 断言命令存在且红相可证（构造缺口时非零退出）。
4. r1-dim6 receipts 覆盖全阶段，verdict 回执由断言退出码计算。
5. `grep -L "^Status:" docs/decisions/[0-9]*.md` 为空（ADR 索引一致性不回归）。