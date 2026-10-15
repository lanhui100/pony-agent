# Wave-11 元复盘（PA-STATUS-FLOW 运行状态条优化）

> 复盘对象：本次 Agent Team 的**协作元框架与组织协议**，不纠缠业务代码细节。
> 物理账本：`.dev-team/state.json`（wave 11）、契约 `.dev-team/contracts/wave-11-status-flow.md`、收据 `.dev-team/evidence/*`。

## 1. 通信拓扑与信噪比

| 拓扑要素 | 实际 | 评价 |
| :-- | :-- | :-- |
| 常驻名册 | Lead + Test Agent + Executor（3/8，留足预算） | 合适：单模块前端任务，无 Protocol 独立槽位（契约由 Lead 落盘，见 §3） |
| 计算池（不占槽位） | 4 个 Subagent（L3-A/L3-B/L3-P + 1 路增量复审） | 正确：审查为无状态单轮求值，未挤占名册 |
| 看板任务 | 4（T1 红相 / T2 绿相 / T3 契约演进测试 / T4 引擎改动→CANCELLED） | 干净：终态全部 completed，无 in_progress 残留 |
| 零废话 JSON 协议 | 3 路 L3 均落盘 `review-l3*.json` 且返回同构 JSON | 有效：findings 全部可定位 file:line，无过程性吹捧 |
| 冗余通信 | Test Agent 首轮把 E2E spec 委派给子 subagent → 子 agent 上下文耗尽且零产出，Lead 一次 steer 后改为自写 | **真实损耗**：多耗 1 个 wait 周期；教训见 §4-① |
| 对话内噪声 | Executor 两次在未获批时推进（§3 偏差 1）、Test Agent 一次基于未验证假设（"改动前后均 48px"）下结论 | 均被 Lead 的物理复测拦下，未污染交付 |

信噪比结论：**机器门禁产出（EXIT/逐帧物理量）占主导**，主观陈述占比低；但"未经自测的因果断言"仍出现 2 次，说明「物理凭证先决」需要显式写进角色 Prompt（本波后半段已补）。

## 2. 门禁穿透与误杀率

- **红相真实性**：T1 红相 5 条断言在基线 commit 上真实失败（含 M1 实测极差 392.70px）；红相 commit `2a83ae7` 锚定后 Executor 才写 `src/**`（除 1 次偏差，见 §3）。
- **独立测试分权有效**：Test Agent 全程未写 `src/**`，Executor 全程未改 `tests/**`；机械写域审计（`git diff --name-only`）零越界。
- **假门禁拦截（本波最大价值）**：Lead 用同探针 A/B 复测推翻了 Executor 的「smooth→auto 消除 179px 滞后」结论（实测 48 vs 47px、超调 20px vs 0px），并按「严禁夹带重构」回退 → **避免了一次进入核心滚动引擎的无效改动**。
- **L3 三路 FAIL 的产出**：9 条 findings 全部为真实缺陷（mode-blind 误隐藏、pointer-events 吞点击、label 闪跳、320px 遮挡、图标对比度 1.43:1、`will-change` 常驻、M3a flaky、NFR 未 supersede、无冻结产物 EXIT=0 收据），**零误杀**——Lead 逐条复核后 9/9 采纳（其中 320px label 挤压一项经产品包络证据 `tauri.conf.json:19 minWidth=780` 判为越界、不修）。
- **指标演进（3 轮）**：M3 从「绝对单帧 ≤32px」（不良定义、flaky）→ M3a/M3b/M3c → M3a 容差 8px + M3c 96px。每一次均由**实测分布**驱动（16 次运行 39–67px；+6px 微抖），非事后放宽；supersede 全部登记在 NFR 文件与契约 §7.1–7.3。
- **NFR-DEGRADED 欠账：0**；未使用任何豁免。
- 误判率：L3 findings 误杀 1/9（11%），假放行 0/9。

## 3. 分工契约与隔离有效性

- **有效**：`src/**` 与 `tests/**` 硬分权使「测试冻结只读」在整波 6 次返工中未被破坏；`blocked_by` 使 T2 在 T1 完成后才 ready。
- **偏差 1（真实发生）**：Executor 在 T3（契约演进测试侧）未完成、红相未重锚时即写入 `src/lib/useTimelineAutoScroll.ts`（§7 auto 改动），违反两段式时序。**处置**：Lead 不追溯美化——用 A/B 实测直接判定该改动无效并回退，同时把「回退完整性」写入 L3-A 审查项。**教训**：`blocked_by` 只约束看板可认领性，不约束已激活 agent 的写手；应把"未收到 GO 消息禁止写 X 文件"作为**显式红线**写进 Executor 的任务书与后续波次模板（本波后半段已补）。
- **隔离有效性**：`HomeWorkspace.vue` 由 Executor 独占、`status-flow-*.spec.ts` 由 Test Agent 独占，两者并行返工零冲突；Lead 仅做只读侦察 + git 操作 + 契约/账本/文档落盘，未写业务逻辑（符合 Lead Write Prohibition）。
- **滚动契约账本**：本波活跃契约条目始终压缩在契约文件 §2（11 条）+ §7（3 小节），未发生跨波次注意力稀释。

## 4. 元协议迭代建议（对 dev-team skill 自身）

1. **子代理委派反模式入模板**：Test Agent 首轮把「写 22KB E2E spec」委派给子 subagent，导致子 agent 上下文耗尽 + 零产出 + 1 个 wait 周期浪费。建议 skill 在角色 Prompt 模板中显式加入：「**测试/实现产物必须由本 agent 亲自落盘；禁止把交付物编写委派给下级 subagent**」。
2. **「物理凭证先决」需写入所有角色的初始 Prompt**：本波出现 2 次「未自测的因果断言」（Executor 的 179px 归因、Test Agent 的"改动前后均 48px"）。建议 skill 把「任何因果结论必须附同口径 A/B 复测或明确标注为假设」列为通信协议硬约束。
3. **两段式时序需要机械锁**：`blocked_by` 无法阻止已激活 agent 抢跑。建议 skill 增加轻量机械手段：被依赖任务未完成时，Executors 的目标文件加入 `.dev-team/locks/<file>.lock`（Lead 在重锚后删除），并把「未解锁写入」列为 Join Barrier 的自动 FAIL 项。
4. **UI 类任务的 NFR 模板缺口**：现有 `nfr-lint.py` 只认后端 schema，本波被迫自建 UI-NFR 文件（布局位移/动画时长/对比度/窄屏避让）。建议 skill 沉淀一份 `nfr-baseline-ui.schema.json` + `dev-ui-nfr-lint.py`，并把「对比度 ≥3:1」「隐藏态暂停动画」列为默认 UI 字段。
5. **指标演进需强制记录 supersede**：本波 L3-P 抓到「断言删除但 NFR 基准未同步」的穿透。建议 skill 规定：删除/放宽任一验收断言时，必须同提交更新 NFR 基准并附 supersede 理由与实测分布，否则 Join Barrier 判 FAIL。
6. **审查预算与互补视角有效**：3 路互补视角（状态流转 / 交互视觉性能 / 证据审计）零重复、零误杀，成本-效能良好。建议保留"UI 任务=1 路状态流转 + 1 路视觉性能 + 1 路证据审计"作为默认矩阵。

## 5. 本轮物理门禁终态

- HEAD `46a0e7d`：`version:check`/`typecheck`/`test:unit`(76 files/813 passed)/`build`/冻结 e2e(4 passed) 全部 EXIT=0；回归对照 `checkpoint-branch`(6 passed/2 failed) 与 `browser-preview`(6 failed) 与基线逐条一致。
- 交付报告：`docs/release/2026-10-15-pa-status-flow/README.md`；ADR：[0033](../../docs/decisions/0033-runtime-status-bar-out-of-scroll-flow.md)。
