# wave-64 元框架复盘（Meta-retrospective）

任务：桌面端点停止不即时终止 + 中止/错误应只由「消息区最后一条状态消息」承载（含移除非体系色绿与额外消息）。
分级 B。红相锚定 `af599ef4`，实现 `801cafe0`，整改 `5f295060`。

## 1. 通信拓扑与信噪比

- 拓扑：Lead（契约/裁决/门禁/提交）+ test-agent（红相所有权）+ executor-fe / executor-rs（正交写域）+ 4 轮 Subagent 审查（L3-A/L3-B/L3-P/delta 复审），常驻名册 4 人，符合 Roster ≤ 8。
- 有效降噪手段：探查交给 2 个只读 Explorer（E2 档），Lead 仅做裁决性定点复核；审查产物强制 JSON + `dev-review-lint.py`，避免长文互吹（4 份 review JSON 全部 lint exit 0）。
- 冗余成本：Explorer 两条线之间有小幅重复（sessionNotice 与 stop 路径同时被两侧触及），但换来根因定位「桌面走 stop_graph_run 而非 stop_turn」这一关键事实，ROI 为正。
- 浪费点：Lead 自跑浏览器 E2E 红了 2 次（都是测试时序/选择器缺陷，非产品缺陷），每次约 1 轮墙钟；本可在 spec 里先核对按钮 testid 切换（`workspace-submit-action` → `workspace-stop-turn`）再执行。

## 2. 门禁穿透与误杀率

- 真实拦截（有效）：L3-B 抓到 1 条 BLOCKING —— 后台会话 `turn:failed` 仍落占位正文（我方向 Executor 下发的契约漏了后台分支）；L3-A 抓到 1 条 BLOCKING —— C5 宽限监督后 in-band 二次收口产生**两条** `turn:cancelled`（R3 红相实测序列 `…turn:stop_requested, turn:cancelled, turn:cancelled`）；delta 复审再抓到 1 条同类覆盖不全（`prepare_turn` Err 分支漏闸，10 处内联只盖 9 处）。三条都是单点编码/单路审查会漏的真缺陷。
- 误杀/超纲：0。L3-P 7 条 ADVISORY 中 5 条被采纳（证据化/日志留痕/aria/假绿/口径），2 条按「既有行为、非本波回归」明确不扩大范围（`runtime.ts:4843` 截断覆盖、`turn_sync` 路径）。
- 反向风险：审查预算 4 路（3+1 delta）对 B 级偏重，但每条 BLOCKING 都能给出机器可复现反例（R3 用例、grep 计数），说明预算花在真问题上而非形式。
- 门禁穿透（真实缺口，已登记）：`turn_sync.rs` 非流式 `aborted→failed`、非命令类工具 handler 执行期不可中断、子代理 120s 同步等待不观测取消 —— 本轮按最小闭包未动，登记 backlog。
- 流程偏离（诚实登记）：本波**未执行契约要求的 Phase-1 方案对抗审查**（Lead 直接以 Explorer 取证 + 契约冻结 + 红相推进），`dev-meta-lint` 对 plan 收据的检查命中的是历史 `review-plan.json`，不构成本波证据。该偏离以「红相先冻结 + 4 路实施后对抗审查」部分补偿，但不等于方案阶段对抗。

## 3. 分工契约与隔离有效性

- 测试分权有效：test-agent 单独持有 `tests/acceptance/wave64/`、`crates/.../stop_graph_run_wave64.rs` 与 3 份既有 spec 的迁移权；两 Executor 全程只读测试（`git diff --stat` 复核：executor 提交不含 tests/）。Executor 两次被审查打回都是**实现**问题，未出现「测试被改以迁就实现」。
- 写域正交有效：前后端写域天然不相交；唯一重叠加 `WorkspaceTurnItem.vue`（并发会话在同一文件有 P1-2 WIP）通过 `git update-index --cacheinfo` 定向暂存隔离，commit 前后 `git status` 证明未吞并他人未提交改动。
- 契约演进受控：3 次契约修订（优先序裁决、failed wire text 归一化、terminal-once 语义）均由 Lead 落盘后再下发给 Executor/Test，无「实现边写边改契约」。
- 弱项：Executor 首轮把 `DEFAULT_FAILED_TURN_MESSAGE` 常量整体删除（属实现侧顺手扩大），需 Lead 复核引用面；说明「最小闭包」在常量类依赖上仍靠人复核。

## 4. 元协议迭代建议

1. **契约必须枚举「同一语义的所有入口」**：本次两条 BLOCKING 都源于「只改前台/只改 9 处中的 1 处」的覆盖不全。建议 dev-team 在 Phase 1 契约模板中增加「入口普查」小节，要求先 `grep` 出该语义的全部写入/派发点并逐条登记，再冻结契约。
2. **终态类改动的必带判据**：凡涉及「终态事件 / 状态持久化」的任务，契约应强制一条「同一实体恰好一条终态」的幂等判据 + 可确定性复现的红相用例（本轮 R3 的成功做法可模板化：注入小 grace + 握手/闸门而非 sleep）。
3. **共享工作区并发会话**：本波暴露「同一文件被另一会话未提交 WIP 占据」为常态。建议 skill 增加标准动作：交付提交一律「按文件/按 blob 定向暂存 + `git diff --cached --stat` 自查 + 提交后 `git status` 只剩他人 WIP」三步。
4. **真实浏览器门禁前移**：E2E 时序缺陷应在红相阶段由 Test Agent 用真实浏览器跑一次，而不是留到 Lead 收口阶段才发现；建议把 `agent-browser`/Playwright 冒烟纳入 test-agent 的红相清单。
5. **跨语言字面量护栏**：Rust `FAILED_PARTIAL_TEXT_SUFFIX` ↔ 前端常量属隐式契约，建议纳入 CI 的契约一致性检查（`contract:check` 扩展）。

## 审查收据索引

- 红相：`.dev-team/wave64-red-receipts.md`（3 轮）、`.dev-team/l2at-done-wave64-tests.json`
- 绿相：`.dev-team/wave64-green-fe.md`、`.dev-team/wave64-green-rs.md`
- 浏览器：`.dev-team/wave64-browser-gate.md` + `docs/release/evidence/wave-64/stopped-terminal-status.png`
- 对抗审查：`review-wave64-a.json` / `review-wave64-b.json` / `review-wave64-l3p.json` / `review-wave64-recheck.json`（均 lint exit 0）
- 交付：`.dev-team/wave64-delivery.md`
