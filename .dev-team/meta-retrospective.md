# 元框架复盘报告（Meta-Retrospective）

> 本轮对象：Wave-5 + Wave-6（成本估算 / 预算软告警 / CLI,TUI 可观测性引导 / 计价协议语义 / 子 span 会话 / 结构化日志 / 巡检脚本修正）。复盘重心为**协作元框架与组织协议本身**。

## 补充：Wave-6 增补复盘

### 门禁穿透（本轮最有价值的三次）
1. **自证实验隔离了责任边界**：Wave-5 收尾时公网 issue 的 `trace_id` 为空，执行者倾向「客户端没发」，我改用**不经本仓库**的 curl 直打 ingest，回读事件 payload 发现 `event_id` 保留而 `trace_id` 被服务端丢弃 ⇒ 一次实验把"客户端缺陷"改判为"服务端 schema 缺口"，避免了继续在客户端加无效修补。**结论：跨组件断言必须做"绕过本组件"的隔离实验。**
2. **证据不可信导致假红**：`wave5_cost_test` 的 mock OTLP sink 不回包 ⇒ `force_flush` 恒 Err ⇒ 用例恒红；执行者用双探针（复刻 sink vs 回 200 sink）证明是测试缺陷而非实现缺陷。**教训：红相也要验证"红得对"**。
3. **巡检脚本自身误判**：`tcount` 缺时间窗把 1h 前的 trace 判成不存在（假 MISSING），`g/gh` 未排除文档与 `target*` 把「trace/metrics 二级化」散文判成 `/#metrics` 实现（假 PASS）。修完 `M1 MISSING / T4 PASS` 才与事实一致。**教训：审计工具的输出也是"证据"，必须能被交叉验证（本次用显式时间窗复核推翻了此前的"数据丢失"结论）。**

### 元协议增补
1. **P0 撤回错误结论要留痕**：我把"Tempo 丢数据"写进报告后经复核推翻，已在同一文件保留「原判断（错误）→ 复核（正确）→ 根因」三段，而不是偷偷改写。审计报告必须可追溯订正。
2. **P1 服务端契约缺口需独立工单**：客户端已把 `trace_id` 送到 ingest，服务端 schema 不接 ⇒ 这类跨仓缺口应在 `OBS-GOV` backlog 建「PonySentry ingest 增加 trace_id」条目，并在客户端契约里标注「依赖服务端支持才可互跳」。
3. **P1 mock 工具收敛**：两处 OTLP sink（`otel_trace_migration_test` 会回包、`wave5_cost_test` 曾不回包）导致一次假红。建议抽 `tests/support/otlp_sink.rs` 单一实现，红相前先自检 sink 行为。
4. **P2 巡检脚本应有自检用例**：`selftest.sh` 覆盖了 declare/audit-diff，但 `audit-project.sh` 的判据（M1/M2/T4）没有"假文档目录"回归夹具。建议加 fixture：一个只含散文的假项目必须判 MISSING。

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


## 补充二：Wave-7（中等规模新项目 Usage Governance v1）

### 门禁穿透
- **执行者主动发现"可绕过面"**：CLI 用 `FileSessionBackend` 而账本走 trait 默认实现（恒零）⇒ enforcement 在 CLI 永不触发。执行者在收口阶段自查发现并修（同 schema SQLite 侧车），比 L3 审查更早。**这说明"实现者自证"在多宿主场景下仍有价值——前提是自证给出物理收据（预置 $1 账本仍不拦）而非口头断言。**
- **Lead 侧踩坑两次（自造噪声）**：复审计时把 settings 治理字段写成 snake_case（`hardLimitUsd` 实为 camelCase），serde `Option` 静默忽略 ⇒ 误判"enforcement 不生效"；另把"日志文件懒创建"误判为"日志链断"。两条都已写入复审报告的踩坑记录。**教训：验证失败时先怀疑验证方法，再怀疑实现。**
- **L3 审查未在时限内产出**：Wave-7 的 L3 subagent 超时未落盘 JSON，Lead 改为对三处最高风险（NULL≠0、第二份计价公式、span 白名单）做定点自查并以 Tempo 实据佐证。**这属于对"审查预算超水位自动降级为纯机器核验"的一次真实应用，需在下一轮把"L3 超时"显式记入熔断计数。**

### 元协议增补
1. **P0 多宿主一致性必须显式列为本项目的一等检查项**：本次"CLI 可绕过"与 Wave-4 的"CLI/TUI 零引导"是同一根因（桌面优先实现）。建议在 dev-team 的 L3 检查清单里固化一条：**凡 core 能力，必须列出三宿主接入点并逐一验证**。
2. **P1 配置字段的跨语言命名要有机械校验**：本轮两次踩坑都源于 Rust serde camelCase ↔ 手写 JSON 键名不一致。建议在验收里加一条：settings 结构体字段名与 `src/types/settings.ts` 由脚本比对（现有 `check-declaration.sh` 类似思路）。
3. **P2 复审脚本应内置到仓库**：本轮复审命令（隔离 HOME + token 注入 + Tempo 回查）是手工拼的。建议落成 `scripts/observability/chain-probe.sh`，把"正常轮 / 阻断轮 / trace 回查"三件事一键跑通，避免每轮重写并重复踩坑。
