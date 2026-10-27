# 元框架复盘（Wave-44：前端健壮性收敛 + 可观测性审计）

> 本轮对象：Wave-44（PA-115 启动期 host-read 有界超时 / PA-135 组合态撤销 / PA-117 调试输出门控）+ 基于 Tempo/Loki/Prometheus/PonySentry/Langfuse 真实数据的项目审计。
> 历史复盘保留于 `meta-retrospective-wave6.md`；本文只记本轮。

## 一、 通信拓扑与信噪比 (Topology & Noise)
- **拓扑**：Lead + Protocol Agent + Test Agent + Executor-A + Executor-B（5/8 槽位）；方案审查 A/B、L3 A/B、L3-P 共 5 次 Subagent 挂载；观测事实采集用 1 个只读 Explorer Subagent。
- **有效实践**：Explorer 一次交付 25 条带 command/exit_code 的观测事实（77 trace / 11 issue / 47 Langfuse trace），Lead 只做抽验（复跑 `count(up)`、TraceQL 计数、ClickHouse count），把「数据采集」与「裁决」解耦，显著降低 Lead 上下文占用。
- **噪声与浪费**：第一批方案审查子代理 2 个中有 1 个长时间无产物（疑似上下文耗尽），导致 Gate-1 重复派发一轮；Test Agent 在 `tests/acceptance/wave44/` 留下临时 `_probe117c.spec.ts`，由 Lead 清理。**结论：子代理必须有“产物路径 + 时限 + 失败信号”三件套，临时文件必须写在 /tmp 或带下划线且被 exclude。**

## 二、 门禁穿透与误杀率 (Gate Penetration / False Negatives & Positives)
本轮门禁抓到 5 个真问题，全部有机器证据：
1. **Gate-1 BLOCKING（真缺陷，非误杀）**：3500ms watchdog 解锁输入后，15000ms 超时若整包快照回滚会静默吞掉用户草稿。审查 B 用 fake-timer 反例证明；契约新增 `PA115-T2a` 草稿保全条款，实现以“回滚前保存/回滚后恢复 draft”最小闭包修复。
2. **测试自身缺陷（TEST-DEFECT）**：`PA117-T1` 在开启态测量后读活窗口缓冲，忠实实现必红。Test Agent 改为读关闭态快照并重新锚定红相；实现未迁就错误断言——这是测试分权制生效的关键证据。
3. **NFR 机械性缺口（BLOCKING 号）**：超时值只在常量层钉住、调用点未钉，`1000ms` mutant 可存活。补“14000ms 未收敛 / 16000ms 收敛”后 mutant 被击杀（Test Agent 实测）。
4. **L3-B BLOCKING（实现级，交付前拦截）**：关闭态 markdown `render-complete` 仍写 `streamDebugState` 5 个键并落盘，违反契约 §2.3。修复为 `updateStreamDebugReveal` 入口 no-op 门控（+4 行）；Test Agent 补“落盘无 markdownRender* 键”断言；L3-B 复核探针由 5 键降为 0 键，改判 PASS。
5. **写域机械审计 0 越界**：`git diff --name-only` 与 4 个 write_scopes 完全一致；blast-keywords 0 命中；union diff 58 行 <100。
- **误杀/降级**：0 起 NFR-DEGRADED；审查 A 6 条、B 9 条 ADVISORY 中，采纳 5 条（契约探针口径、legacy IME Enter、restoreAllMocks 污染、NFR 0 口径、调用点钉住），其余登记 Non-Goal 或 backlog。
- **红相纯度**：`dev-red-purity.py` PASS（pure_red=true），8 条均落点 SUT 栈。

## 三、 分工契约与隔离有效性 (Contract Isolation)
- **写域隔离**：Executor-A 仅 `src/stores/runtime.ts`（+9 行）；Executor-B 仅 3 个前端文件（+54 行）；Test Agent 仅 `tests/acceptance/wave44/**`；Lead 仅 `.dev-team/**` 与 `docs/**`。零交叉污染。
- **契约演进**：Gate-1 后 `contract-matrix-wave44.md` +26/−12、NFR +7；Test Agent 同步迁移断言；Lead 以 `git stash push -- src/` 在“实现已落地”状态下重锚红相（8/8 红），随后 pop 恢复实现——证明契约演进不破坏红相纪律。
- **分权制的代价与收益**：Test Agent 不能修实现、Executor 不能动测试，导致“测试缺陷”需 Lead 裁决 + 两轮往返；但正是这条约束让 `1` 类假绿（开启态读活窗口）没有变成“改实现迁就测试”的隐性缺陷。

## 四、 元协议迭代建议 (Self-Evolving Protocol)
1. **[P0] 子代理交付契约前置**：Subagent 任务模板必须含 `output_path`、`deadline`、`on_failure` 三字段；Lead 以文件 mtime 作为唤醒信号，超过时限未见产物直接重派（本轮 Gate-1 因此损失约 10 分钟）。
2. **[P0] 契约演进后的红相重锚应脚本化**：`stash src → 跑验收 → commit tests → stash pop → 跑验收` 是通用动作，建议固化为 `dev-red-reanchor.sh`，避免手工顺序错误。
3. **[P1] TEST-DEFECT 豁免通道**：冻结测试被发现自相矛盾时，当前流程要求“改测试 + 重锚 + 重跑”，建议在契约矩阵增加 `test_defect_waiver` 段（含反证命令与 Lead 裁决），减少往返。
4. **[P1] 观测驱动的 backlog 生成**：本轮审计产出 9 条优化点（root span 错误承接、探针隔离、issue 治理、usage 属性等），建议把“审计报告 → backlog 卡片”作为 wave 收口的固定动作，让观测数据真正进入迭代闭环。
5. **[P2] 审计工具口径修正回写 skill**：`audit-project.sh` 对 desktop 项目的空 service/project 与 `limit` 当计数的缺陷，应作为 `observability-onboarding` 的 selftest fixture 固化（否则每轮都要人工校正）。

---

# Wave-45 增补复盘（可观测性语义收敛）

## 通信拓扑与信噪比 (Topology & Noise)
- 拓扑：Lead + Protocol-45 + Test-45 + Executor-Rust + Executor-TS（5/8）；计划审查 2 路、实现审查 L3 A/B + L3-P 共 3 路，全部 Subagent。
- 有效点：一次把“审计发现 → 契约 → 红相 → 实现 → 证据”跑通；Protocol 主动指出“测试 id 映射缺失”与“`quota_blocked` 已在 HEAD 子 span 写出”的风险，避免了零鉴别力断言。
- 噪声：Rust 首次冷编译 target-test 约 5 分钟；三个审查子代理并行读取大文件导致等待 10 分钟；建议冻结“审查输入包”（diff + 契约 + 2 个测试文件）而非让审查员自行 grep。

## 门禁穿透与误杀率 (Gate Penetration / False Negatives & Positives)
- 抓到真问题：① 计划审查 A 判 BLOCKING——`quota_blocked` 子串在 HEAD 已存在，PA154-T2 零鉴别力 → 契约演进为唯一前缀 `turn_error: `；② 全量 lib 出现“新增失败”——经模块级 20 次复跑证明是既有 ponysentry 共享全局队列 flake（2/20），非本波引入，已登记 backlog；③ L3-P 指出 `take_active_turn_error` 缺归属校验（slot=None 时可能清空同 task 另一 turn），登记 advisory。
- 误杀：0 起 NFR-DEGRADED；两路审查共 13 条 ADVISORY，采纳 3 条（声明登记、env 文档、flake 收据），其余明确登记 Non-Goal。
- 红相：Rust 4+1 条断言红、0 编译失败；TS 2/2 红；重锚以 `git stash` 实现态复验后提交，红相纪律未被“实现已落地”破坏。

## 分工契约与隔离有效性 (Contract Isolation)
- 写域：Rust Executor 仅 `otel/mod.rs`（+261），TS Executor 仅 `runtime.ts`（+8），Test Agent 仅 tests，Lead 仅 docs/.dev-team；`git diff --name-only` 与声明完全一致。
- 契约演进 2 次（turn_error 前缀、outcome 值），均由 Protocol 落盘、Test 迁移、Lead 重锚，未出现实现方改测试。
- 代价：Rust 集成测试的全局 OTel init 与 sink 预算带来一次不可复现 flake（1 次/9 次），提示“并发测试共享全局态”应默认串行或每测试独立进程。

## 元协议迭代建议 (Self-Evolving Protocol)
1. **[P0] 冻结测试的 flake 检测入闸**：对涉及全局态（OTel init、sink、LAST_ENQUEUED）的用例，加入“连跑 N 次”稳定性门禁，N=5；不达标不得红相锚定。
2. **[P0] 既有 flake 必须与本次回归分离**：全量失败清单需与基线逐项对照并附“模块级复现率”，否则会把既有 flake 误判为 IMPL-DEFECT（本轮已按此口径执行）。
3. **[P1] 断言唯一性优先于语义描述**：涉及父/子 span 同名错误时，契约必须冻结“只可能出现在目标 span”的唯一字面量（本轮 `turn_error: `），否则 contains 断言零鉴别力。

---

# Wave-47 增补复盘（可观测性升级至 ob2：收口点与变更门禁落地）

## 一、 通信拓扑与信噪比 (Topology & Noise)
- **拓扑**：Lead + Protocol-47 + Test-47 + Executor（4/8 槽位）；无状态审查挂载 L3-P（review-l3p.json）。
- **有效实践**：Protocol Agent 基于既有 Wave-46 ErrorBoundary 与 Wave-4 OTel 机制设计真实存在的收口点，拒绝假收口；Test Agent 独立在 `tests/acceptance/wave47-ob2-enforcement.spec.ts` 编写覆盖 7 项维度的对抗性门禁测试，断言拦截率达 100%。
- **信噪比控制**：会话严格遵循极简交互规范与 JSON 零废话协议，各代理完成各自 write_scopes 闭包后有序流转。

## 二、 门禁穿透与误杀率 (Gate Penetration / False Negatives & Positives)
- **拦截能力**：7 项断言覆盖 D1 未批准拦截、Rule A 前端非法 onErrorCaptured、Rule A 后端手写 traceparent 头注入、Rule B 路径清空、非声明越界扫描等。真实 Exit Code 2 拦截率 100%。
- **门禁打通**：通过将 audit-diff 挂载至 CI（`.github/workflows/ci.yml`），彻底解决 `mount_missing` 缺口，使 `ponygo observe audit --level ob2` 物理 Exit Code 为 0。
- **误杀率**：0 起误杀，所有规则均由项目实际收敛逻辑映射，无泛化硬编码。

## 三、 分工契约与隔离有效性 (Contract Isolation)
- **写域隔离**：Protocol Agent 负责声明契约与 NFR 落盘；Test Agent 编写验收测试；Lead 与 Executor 完成收敛；写域完全正交。
- **真实收口点**：
  - `error_boundary`: `src/components/common/ErrorBoundary.vue`（guarantees: breadcrumbs, trace_id）
  - `http_client`: `crates/pony-agent-core/src/agent/otel/mod.rs`（guarantees: traceparent, timeout）

## 四、 元协议迭代建议 (Self-Evolving Protocol)
1. **[P0] 变更门禁载体自动化探测适配**：在 `ponygo observe audit` 针对各类型项目时，可提供对主流 CI 载体（GitHub Actions / pre-commit / husky）的智能挂载检测或自动初始化提示。
2. **[P1] 测试套件前置隔离**：在多 Agent 协同执行 vitest 测试时，确保测试文件临时目录使用独立命名空间，避免并发运行互相争用。

---

# Wave-51 增补复盘（可观测性 P0 三项修复：版本单一来源 / 治理会话归属 / 思考与缓存 token 出口）

## 一、 通信拓扑与信噪比 (Topology & Noise)
- **拓扑**：Lead + Protocol Agent + Test Agent（两任）+ Executor（常驻峰值 5/8）；Gate-1 两轮（2 路独立）、
  Join 双路（L3-P + L3-B）全部以 Subagent 挂载；Explorer 1 个只读侦察（E1）。
- **有效实践**：Protocol 的契约矩阵 v1→v4 把「env 仅 trim-only」「cache 仅 OpenAI 族」「wire 只断键名 +
  纯函数值级断言」写死，实现/测试/文档三方没有出现口径漂移；Gate-1 第 1 轮拦下 4 个 BLOCKING
  （semconv 键名错、OT-8e 冻结锚点死锁、C3 无接线条目、NFR 优先级 3 级 vs 4 级），v3 修正后复评 PASS。
- **噪声与浪费**：① 首任 Test Agent 大量前置阅读后上下文耗尽，0 文件产出，换 test-agent-2（最小读集）
  后一轮落盘 —— 证明「阅读预算」必须写进 prompt；② 治理 span 断言首版用整段 body `contains`，被同
  trace 根 span 满足造成假绿，Test Agent 自检后按 Lead 裁决改为解码目标 span 自身属性；
  ③ Lead 起草 task 卡时未先与 Protocol 对齐「根聚合」范围，导致契约 Non-Goal #4 与任务卡冲突，
  多花一轮消息往返。

## 二、 门禁穿透与误杀率 (Gate Penetration / False Negatives & Positives)
- **真拦截**：Gate-1 两路 BLOCKING 全部有 file:line 反例；L3-P 六问 0 BLOCKING（2 ADVISORY：
  非法版本静默回落无日志、C2 的 9/9 缺真实 trace 收据）；L3-B 复核未发现实现缺陷（见 review-l3b-wave51.json）。
- **机械门禁**：契约/NFR lint、cargo check（core + tauri）、验收测试 11 例、`--lib` 1175 例（=基线）、
  version:check、audit-diff、check-declaration 全 Exit 0；union diff 2402 行触发双路 L3 审查。
- **红相纯度**：9/11 失败（7 个桩 panic + 1 缺属性 + 1 接线缺失），2 例允许负向面均带正向对照；
  但 `c2_governance_span_carries_session` 首版不可证伪，属于「红相全绿≠断言可证伪」的真实样本。
- **测试缺陷穿透（TD-1）**：`l2at_concurrent_init` 的 `Barrier::new(3)` 主线程只 wait 一次，第 2 轮死锁；
  红相期被 `unimplemented!()` panic + `catch_unwind` 提前 return 掩盖，绿相才暴露。按 TEST-DEFECT 由
  Test Agent 最小修复并重跑 11 passed；残留风险（实现回退为 panic 时该例会 hang）已登记。
- **Join 期真拦截（BLOCKING）**：L3-B 独立复现出「协议门控零行为覆盖」——把 `cache_reportable_input_tokens`
  改恒 `None` 后 11 例仍全绿（契约 V4-3/T6② 无 wire 判定）。Lead 裁决补测：新增
  `c3_cache_protocol_gating_wire`（最小 `ProviderClient` mock 实调 `provider_decision`，OpenAI 族 cache=5 /
  Anthropic 族缺键 / reasoning=7）+ `c3_root_zero_visible_on_wire` + `c1_env_reverse_proof`，
  最终 **14 passed**。这条证明「Join 双路审查不是形式主义」，也暴露「静态计数型接线断言可被注释满足」。
- **自查测试缺陷**：`c1_env_reverse_proof` 首版用文本 `contains("1.2.3\\n")`，被 protobuf 字段 tag `0x0a`
  在 lossy 解码后伪装成换行误导；改为解码 resource 层做值相等断言。教训：**wire 层文本包含断言在字节域不可靠**。
- **零假测试**：验收测试无 `#[ignore]`、无空断言；wire 断言分两层（键名在 wire、数值在纯函数/解码器）。

## 三、 分工契约与隔离有效性 (Contract Isolation)
- **写域隔离**：Protocol 仅桩/契约/NFR；Test Agent 仅测试文件与 OT-8e 一处授权修订 + 收据；Executor 严格
  4 个 write_scope（`otel/mod.rs`、`turn_flow.rs`、`ponysentry/config.rs`、`src-tauri/lib.rs`）；Lead 仅
  文档/裁决/门禁。`git diff --name-only 090df55..HEAD` 与声明写域零越界；git 写操作 Lead 独占（5 次原子提交）。
- **契约演进**：v1→v2（根聚合纳入）→v3（semconv 键名/OT-8e 授权/优先级拆分/注入防御）→v4（三处澄清）；
  每版均由 Protocol 落盘、Lead 裁决、Test Agent 迁移断言后才重跑红相。
- **分权代价**：TD-1 需 Lead 裁决 + Test Agent 修复 + Executor 重跑，往返约 20 分钟；但避免了「实现方
  以改测试迁就实现」的隐性破坏。

## 四、 元协议迭代建议 (Self-Evolving Protocol)
1. **[P0] 最小读集强制**：Test/Executor 首条 prompt 必须给「精确文件 + 行范围 + 禁止通读」，大产物要求
   单次 write；本波首任 Test Agent 的失败就是反例。
2. **[P0] 红相可证伪性抽检**：冻结前对每条属性归属断言做一次「目标对象缺失时应转红」的变异抽检
   （本次治理 span 若早做此检查，可省一轮）。
3. **[P1] 并发用例禁 panic 短路**：L2-AT 并发用例必须让 worker 每轮到场（panic 信息累积到循环外返回），
   否则死锁被 catch_unwind 掩盖；本次 TD-1 已登记残留风险。
4. **[P1] 契约-任务卡双签**：Lead 任务卡只引用契约条目号（PA51-Cx-Ty），不重述规则；开波前做一次
   「任务卡断言集合 ⊆ 契约断言集合」的机械 diff。
5. **[P1] 生产证据补位**：合成 wire 断言不能替代 Tempo/Langfuse 真实会话收据；建议每次上报类修复后
   由用户在真实设备跑一跳，并以「会话过滤命中率 / usage 明细」作为收口证据（本次 C2 已列待证据项）。
6. **[P2] 审查输入包化**：给 Subagent 审查员冻结「diff + 契约 + 关键文件行范围」输入包，避免其自行通读
   导致超时（本波一个 L3 审查员超时后由精简版接替）。


