# 元框架复盘报告（Meta-Retrospective）

> 本轮对象：Wave-5（会话成本估算 + 预算软告警 + CLI/TUI 可观测性引导）。复盘重心为**协作元框架与组织协议本身**。

## 一、 通信拓扑与信噪比 (Topology & Noise)
- **拓扑**：Lead + Protocol + Test + Executor-Rust + Executor-Frontend（5 名，≤8）。审查走 Subagent（Wave-5 单轮合并双视角）。
- **并行有效性**：Rust 与前端写域正交，两 Executor 全程并行无冲突；跨侧契约（`costUsd` / `costBudgetUsd`）由契约先冻结。
- **本轮信噪比最好的实践**：Executor-Rust 在遇到「冻结测试自身缺陷」时**不擅自改测试、而是给出根因+两态探针+最小补丁建议**，Lead 一次裁决即通过（k4 sink 不回包问题）。这比"先改了再说"省掉至少两轮返工。
- **仍需改进**：Lead 回派缺陷时仍出现过一次"只描述现象、没给目标态"，本轮已改为"必须贴改后应长的代码片段"（Wave-4 教训复用后生效）。

## 二、 门禁穿透与误杀率 (Gate Penetration / False Negatives & Positives)
**本轮抓到的真问题（全部有机器证据）**：
1. **冻结测试自身缺陷（生产性拦截）**：`wave5_cost_test` 的 mock OTLP sink 只读不回包 ⇒ 导出层必然 3000ms 超时 ⇒ `force_flush()` 恒 Err ⇒ K4 wire 断言**无论实现对错都红**。Executor 用「复刻该 sink → flush=Err」vs「回 200 → flush=Ok 且两个 cost 属性都在 wire」两态探针证明是测试缺陷，Lead 裁决派 Test 修 helper（断言零改动）。这是"红相不可信"的典型，值得沉淀为门禁自检项。
2. **零假测试（前端同义反复）**：L3 指出「软告警不阻断」用例断言的 `canSubmitPrompt` 是**无消费者的 shim** ⇒ 恒真。改为挂载真实 `HomeStatusPanel` 断言告警样式 + 草稿保留 + 提交链路未被阻断，并做变异证伪。
3. **第二份价格真源**：`budget.rs` 残留 `ModelPricing` + 编造的 `$3/$15/$0.75` Default，与「`cost.rs` 唯一真源 / 宁缺勿错」冲突。处置：删 Default、结构改为引用 + 计价委托唯一公式（既有 frozen `wave33` 用例保持绿）。
4. **真实运行暴露的 P0 断链**：`ponycli`/`pony` 对 `otel`、`ponysentry` **零命中** —— 只有跑真实轮次才看得出来（静态巡检会把 T1/T2/T3 判 PASS，因为依赖与打点在 core 里）。
- **误判/超纲**：0 起 NFR-DEGRADED。Wave-5 L3 共 13 条 ADVISORY，我判定 6 条收口（`>=` 阈值语义、来源 URL、唯一真源、假测试、K4 负向用例、来源链接），其余（结构化 warn 字段、Anthropic cache 语义、子 span session 下沉等）转 backlog 并写明风险。
- **既有红复核**：`config.rs:1616`（断言 256_000 vs 常量 1_000_000）经 HEAD 源码核实为既有红，本轮未顺手修（避免夹带）。

## 三、 分工契约与隔离有效性 (Contract Isolation)
- **测试分权有效**：本轮 Test Agent 唯一越界是"改 helper 修断言外的死锁"，我明确背书并要求 diff 仅限 helper 区域；断言逻辑零改动。
- **写域扩展 0 次**：本轮通过"事先在任务描述里列全写域"避免了 Wave-4 的频繁扩权。Executor 仍做到"越界前先问"。
- **契约演进**：`wave-5-cost.md` v1.0 → v1.1（K3.1 告警落点分工、K5.1 价格表生命周期/宁缺勿错），原文保留。

## 四、 元协议迭代建议 (Self-Evolving Protocol)
1. **P0 冻结测试也要接受"可执行性审计"**：本轮 k4 的 sink 不回包导致"红相不可信"。建议在红相锚定后加一步机器自检：**对每条 wire 级用例，确认 mock sink 会回包**（否则用例恒红，会把实现者逼去吞错）。可固化为 `dev-team` 的 Join Barrier 前置项。
2. **P0 交付验证必须跑"真实形态"**：CLI/TUI 三处断链（引导/日志/issue）静态巡检全 PASS，只有真跑一次轮次才暴露。建议把「交付形态矩阵」（desktop / cli / tui / headless）列入可观测性接入的必测项。
3. **P1 shim 禁令**：凡为让测试通过而新增的 `canSubmitPrompt` 类"未被任何 UI 消费"的 getter，一律视为假测试温床。建议在验收模板里加一条：**测试断言的消费点必须是生产代码路径**（可在 PR 描述里给出 `grep` 证据）。
4. **P1 mock sink 契约**：所有 OTLP sink 类测试 helper 收敛到一处共用实现（当前 `otel_trace_migration_test` 会回包、`wave5_cost_test` 曾不回包 ⇒ 不一致即事故源）。建议抽到 `tests/support/otlp_sink.rs`。
5. **P2 证据文件自污染治理**：`cargo test --test '*'` 仍会重写 `.dev-team/evidence/wave15/wire-red.json`（本轮又手工复原 3 次）。建议该用例写入临时目录或标 `#[ignore]`，否则每次全量测试都污染工作树、干扰提交门禁。
6. **P2 enforcement_points 空转**：`audit-diff.sh` 对本站输出「规则集为空」WARN。属诚实空转（reqwest 仍 4 处直连，正则无法可靠拦截），需进 OBS-GOV backlog 建 `http_client` 收口点，否则该门禁长期无判定力。
