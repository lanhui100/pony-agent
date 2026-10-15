# Wave-12 元复盘（PA-SESSION-FALLBACK 删除/归档当前对话后的落点）

> 复盘对象：本次 Agent Team 的**协作元框架与组织协议**。账本：`.dev-team/state.json`（wave 12）、契约 `.dev-team/contracts/wave-12-session-fallback.md`、收据 `.dev-team/evidence/{red,green}-phase-wave12*.log`。

## 1. 通信拓扑与信噪比

| 要素 | 实际 | 评价 |
| :-- | :-- | :-- |
| 名册 | Lead + Test Agent + Executor（3/8） | 合适：单模块状态机改动 + 既有冻结测试改写 |
| 看板 | T5（红相）/T6（落点实现）/T7（冷启动红相）/T8（冷启动实现） | 干净：全部 completed，无 in_progress 残留 |
| 独立审查 | 2 轮（wave-12 增量复审 PASS；wave-12.1 增量复审 PASS + 2 条非致命观察） | **由 Test Agent 承担**（见 §4-①） |
| 关键增益 | 复审报告「同族残留」→ Lead 真实浏览器复现 → 契约演进 wave-12.1 | 本波最大价值：避免只修一半（重启后又落回已归档会话） |
| 信噪比 | 两次「处方不充分」均由执行方主动上报并附 A/B 机械证明（Executor 的第三处早退分支、Test Agent 的 T-g 红绿证明） | 好：未出现「跑通即放行」 |

## 2. 门禁穿透与误杀率

- **红相真实性**：wave-12 红相 9 failed（含 4 个旧契约用例按新契约转红）；wave-12.1 红相 5 failed + e2e 1 failed。两次红相均锚定 commit 后执行方才写 `src/**`（除 wave-11 遗留的抢跑问题本波未复现）。
- **假测试拦截**：T-g 加固证明「all-archived 早退分支」在 store 级原本无判别力（移除该分支 A9 仍绿，仅 e2e 能红）；而仓库 CI 的 `verify` 作业**不跑 e2e** → 该分支在 CI 里等于无门禁。加固后 store 级可判别（移除分支 → EXIT=1，已留痕）。
- **误杀率**：2 轮复审共 0 findings（无误杀）；2 条非致命观察 1 条被采纳加固、1 条（首启无会话 vs 全归档的侧栏行不对称）判为既有 UX 不对称、登记为后续项。
- **回退/篡改防护**：Test Agent 为红绿证明临时改 `src` 时，因 `src` 未提交而不能用 `git checkout --`（会抹掉 Executor 实现），改用备份 + `sha256sum -c` 字节级还原校验 —— 该纪律值得写入 skill 模板。

## 3. 分工契约与隔离有效性

- `src/**` 与 `tests/**` 硬分权保持：整波 8 次往返无越界写入（`git diff --name-only` 逐次核对）。
- `blocked_by` 正确串行化：T6 依赖 T5、T8 依赖 T7；`write_scopes` 对 `runtime.ts` 的跨任务重叠被看板显式提示（T6 完成→释放→T8），未发生并发写冲突。
- Lead 仅做：侦察与复现（浏览器脚本）、契约/裁定落盘、git 锚定与合并、最终门禁与文档（ADR/复盘）；未写业务逻辑。

## 4. 元协议迭代建议

1. **subagent 计算池在本环境连续失败**：wave-12 的 2 路 L3 审查子代理全部「failed before it finished」（无 closing message），wave-11 的增量复审子代理也失败过 4 次。应急做法是改由常驻 Test Agent 承担独立复审（其未编写 src，独立性成立）。建议 skill 增加**降级路径**：当 subagent 池不可用时，允许由「未参与该模块实现的常驻 teammate」承担 L3，并在账本中显式记录降级原因与独立性依据。
2. **「处方充分性」应作为 Executor 的强制回报项**：本波 Executor 两次主动证明「按任务书处方改完仍不达标」（wave-11 的 M3 度量、wave-12.1 的第三处早退分支），避免了假绿。建议 skill 在 Executor 回报模板中固定一行：`处方充分性：是/否 + A/B 机械证据`。
3. **红绿证明的 src 还原纪律**：任务书要求「临时改 src 做红绿证明」时必须规定还原校验方式（本波用 sha256），否则在未提交工作区上极易误伤实现。建议写入 skill 的测试加固章节。
4. **CI 门禁盲区核查应前置**：本波发现仓库 CI 不跑 e2e，导致「只有 e2e 能红」的分支在 CI 中无门禁。建议 skill 在 Phase 1 契约冻结时增加一步「CI 覆盖核查」：对每条新断言标注其运行层级，并确认该层级在 CI 中被执行。
5. **同族缺陷排查应成为契约冻结的固定动作**：wave-12 的复审主动 grep 同族调用点，直接发现冷启动路径。建议 skill 在契约模板中加入「同族入口枚举」小节（列出同类触发路径并逐条判定是否在范围内）。

## 5. 终态

- HEAD `cb12358`（+ `9bd2964` 文档）：`typecheck` / `test:unit`(78 files·828 passed·10 skipped) / `build` / 两条 e2e（session-fallback、session-fallback-cold-start）全部 **EXIT=0**。
- 交付：`src/stores/runtime.ts`（`enterFreshSession` + 冷启动落点）、`src/lib/runtime/sessions.ts`（`pickPreferredSessionId` / `hasSelectableSession` / `deriveInitStrategy`）、ADR [0034](../../docs/decisions/0034-fresh-session-after-delete-or-archive.md)、契约 `.dev-team/contracts/wave-12-session-fallback.md`。
