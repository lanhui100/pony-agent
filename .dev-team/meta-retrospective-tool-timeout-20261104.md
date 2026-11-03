# 元框架复盘（Wave-TT：Run 工具超时策略对齐 DSH）

> 本轮对象：按用户「工具超时是不是应该在调用时自由设定？参考 DSH 源码设计」→「同意改」，实施 Run 超时策略改造波次一。
> 契约：`.dev-team/contracts/tool-timeout-policy.md`（v2）；红相：`fecc512`；审查：`.dev-team/review-tool-timeout.json`（PASS，3 ADVISORY）。
> 团队：Lead + test-agent + executor（常驻 2）+ 1 次侦察 Subagent + 1 次 L3 审查 Subagent。

## 1. 通信拓扑与信噪比（Topology & Noise）

- **拓扑正确**：test-agent（冻结验收）与 executor（实现）是跨阶段有状态连续体，落为常驻 Teammate；侦察与 L3 审查是无状态一次性求值，落为 fresh Subagent。名册峰值 3 ≤ 8。
- **写域正交**：`tests/tool_timeout_policy_test.rs`（test-agent）‖ `src/agent/tools.rs` + `src/lib/runtime/browser-preview.ts`（executor）全程零冲突。
- **真实浪费（Token/墙钟）**：Lead 亲自做了 6 次定点读（tools.rs 各段 + jobs.rs + browser-preview），其中 3 次本可由侦察 Subagent 覆盖——命中「≥5 文件」阈值时的**并行化收益被低估**：我先派侦察、又自己读了同一批文件，属重复取证。
- **审查端浪费**：给 L3 的指令写了「实测」但没限定命令，它跑成了 `cargo test -p pony-agent-core` 全量（124 target），耗时 ~12 分钟。**指令里未限定命令范围 = 让审查者自选成本**，应显式给 `--test <target>`。

## 2. 门禁穿透与误杀率（Gate Penetration）

- **命中（红相拦截了错误设计）**：test-agent 在写测试时**主动报告** CT-6 与既有冻结用例（`harness_optimizations_test.rs`「仅 timeoutMs 不同 ⇒ 指纹相同」）方向相反，并实测该用例通过。Lead 据此撤销了 CT-6——若照原契约实施，会破坏刻意的反重试风暴设计。**这是本轮最有价值的一次对抗**：审查不是靠 LLM 复核发现，而是靠「实现者被迫读既有测试」发现。
- **命中（红相二次拦截）**：红相首次只暴露 1 个编译错误（接缝缺失），test-agent 另用临时副本证明 CT-4 是**断言级**真红（3 failed），避免「仅编译挡住」的假红。
- **命中（L3 抓到 3 条 ADVISORY）**：轮询 deadline 与 `kill_after` 的窄竞态（可能把 timeout 误判成 non_zero_exit，进而丢 hint 且指纹带 timeoutMs）、默认 120s 与 `subagent_turn_timeout_ms=120000` 预算碰撞、`kill_after` 无 disarm 导致线程睡满超时。
- **误杀/漏杀**：0 BLOCKING、0 误判；`dev-review-lint` 首次未过（v1 收据缺溯源字段），补 v2 溯源后 Exit 0——说明机械门禁确实拦住了「无溯源的自签审查」。
- **基线污染识别**：executor 报的 `tui_acceptance::test_provider_selector_state_logic` 红，经 `git show 22241e9 -- src/bin/tui.rs` 定位为**外部波次 commit 引入**（改 `select_current_provider` 4 处），与本次改动无关，未被误算成本波回归。

## 3. 分工契约与隔离有效性（Contract Isolation）

- **契约先行有效**：v1 契约由 Lead 写，但两处设计错误（CT-6 与冻结语义冲突、CT-5 撞逐字断言）在执行前被 test-agent 的事实反馈暴露，改为 v2（CT-6 撤销、CT-5 延后波次二并登记 `B004`/`B005`）。**说明「契约冻结」不等于「契约正确」，需要一条「实现前的事实反查」回路**。
- **常数隔离**：executor 首版把 `DEFAULT_RUN_TIMEOUT_MS` 复用为全工具默认（隐性把其他工具 deadline 10s→120s），Lead 判定为夹带并下发 task-6 隔离（新增 `DEFAULT_TOOL_TIMEOUT_MS=10_000`）。属「最小闭包」原则的一次实际生效。
- **信息下发**：给 executor 的 prompt 含完整契约 + 明确写域 + 禁止改测试，未泄露测试实现细节；给 L3 的 prompt 只给契约与红线，未给实现者叙述（证据不对称成立）。

## 4. 元协议迭代建议（Self-Evolving Protocol）

1. **契约冻结前必须有「既有冻结测试反查」步骤**：本轮两次设计错误都可由「先 grep 既有断言」一步发现。建议 dev-team 在 Phase 1 契约矩阵后追加机械步骤：对每条契约条目，`grep` 其断言关键词在 `tests/` 的命中，若命中且方向相反 ⇒ 必须先裁决（改契约 or 走演进流程）。
2. **审查指令必须自带成本上限**：给审查 Subagent 的任何「实测」都必须附具体命令与 target 列表，否则它会自选最贵的命令（本轮 12 分钟）。
3. **侦察与 Lead 自读不应重叠**：命中委派阈值后 Lead 只做裁决性复核（≤2 次 read），本轮重复取证 3 次。
4. **ADVISORY 需要落账而非只落 JSON**：3 条 ADVISORY（竞态、预算碰撞、线程滞留）当前只在 `review-tool-timeout.json` 里，建议统一登记 backlog（本轮已手工登记 B006，说明该动作应机械化）。

## 5. 波次二（CT-5）增量复盘

- **范围**：`classify_timeout_hint` 指向后台执行（B004）。改动 2 文件 20 行，含 1 处冻结断言迁移。
- **门禁**：红相双路（`71b9280`：S2 未 pub ⇒ E0603；wave9_green 逐字断言失败）→ 实现 `7675017`（15 + 2 passed、clippy/audit-diff Exit 0）。
- **契约演进门禁真实生效**：冻结用例 `wave9_green:196` 的逐字断言由 test-agent 在**独立 `test(freeze)` 提交**中迁移，实现者无法顺手改测试——这正是分权设计要防的动作，本轮验证通过。
- **Lead 复核发现的可接受偏差**：executor 主动删掉网络分支句尾重复的「或显式传入更大的 timeoutMs」（否则同一 hint 出现两次且缺上限语义），属改进而非夹带，已接受并写进 commit message。
- **元协议改进（新增）**：小而可验证的文本类契约（hint 文案）也应走「红相 → 绿相 → 冻结迁移」三段，而不是当作 trivial 直接改；本轮三段的成本约 3 次工具往返，低于事后追查文案漂移的成本。
