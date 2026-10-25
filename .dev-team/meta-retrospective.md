# 元框架复盘报告（Meta-Retrospective）

> 本轮对象：Wave-4（可观测性四维断链修复）。复盘重心为**协作元框架与组织协议本身**，非业务代码细节。

## 一、 通信拓扑与信噪比 (Topology & Noise)
- **拓扑**：Lead + Protocol(1) + Test(1) + Executor-Rust(1) + Executor-Frontend(1)，共 5 名（≤8）。审查全部走 Subagent：L3-A/L3-B 两视角互补，共 4 轮（A×2、B×4）。
- **并行收益真实**：Rust 侧与前端/CI 侧写域正交（crates+src-tauri vs src+.github），两 Executor 全程并行，无一次越界踩踏；关键跨侧契约（settings 字段名 `otelEndpoint/otelToken/otelServiceName/otelSamplingRatio`）靠**契约文件先行冻结**对齐，未出现字段漂移。
- **信噪比问题（需改进）**：同一 BLOCKING 被我连续回派 3 次（F1 跨轮误归因），根因不是执行者能力，而是**我的回派描述不够可判定**——前两次只说"去掉 `|| this.activeTraceId`"，执行者改了最终表达式却保留了 `?? this.activeTraceId` 兜底。第 3 次改为贴「:4351 改后应为哪三行原文」后一次通过。**教训：回派缺陷必须给"改后应长什么样"，不能只给"别写什么"。**

## 二、 门禁穿透与误杀率 (Gate Penetration / False Negatives & Positives)
**有效穿透（本轮价值最高的部分）**：
1. L3-A 捕获 **BLOCKING：门禁关闭时本地上下文伪造 trace_id 写入 Issue**（比 null 更误导，指向 Tempo 中不存在的 trace）。修法引入 `exported_trace_id()` 与 `current_trace_id()` 语义分离。
2. L3-B 捕获 **BLOCKING：前端 trace_id 接线空转**（写入点读 `payload.traceId`，但 wire 无该字段）。该缺陷**全部门禁全绿**——这正是"绿而不真"的典型；连续 3 轮才彻底闭环（started → failed → 跨轮误归因）。
3. Lead 二次捕获 **假绿测试**：Test Agent 的用例 #6 通过，但陷阱未布好（`activeTraceId` 实际 undefined），删除兜底逻辑仍绿。改为「先断言陷阱已生效（`toBe(X)`），再断言不误归因」并做变异验证后，才具备证伪力。
4. Lead 捕获 **环境级假绿**：Executor 用共享 `target/` 在 `/tmp/pony-head` 复现基线红，导致主工作树后续编译复用陈旧 unit 指纹（`cargo test` 编译失败但报错指向 `/tmp`）。已 `git worktree prune` + `cargo clean -p` 修复。
- **误杀率**：0 起 NFR-DEGRADED；两轮共 21 条 ADVISORY，其中 4 条被我判定需收口（脱敏漏面、计量白名单放行 `access_tokens`、跨轮误归因、终态覆盖 traceId），其余进 backlog。
- **既有红隔离**：2 条既有红（`config.rs:1616` 断言 256_000 vs 常量 1_000_000；`pa110 test_ac01_subagent_dispatch`）由执行者用 pristine HEAD worktree 证明与本轮无关，我另行核对 HEAD 源码确认属实。

## 三、 分工契约与隔离有效性 (Contract Isolation)
- **有效**：Protocol 只写 `.dev-team/`；Test 只写 `tests/` 与 `crates/*/tests/`（仅有 2 行 `..Default::default()` 机械适配 + 1 处 `sk-` 形状假 token 被我改为 `sk-test-dummy` 以过 secrets 门禁）；两 Executor 写域互斥。
- **写域扩展的裁决成本**：共 5 次扩权（app_settings.rs、authored runtime/+projection.rs、control_plane/sqlite_session、sse_adapter、turn_flow），每次都由执行者**主动上报缺口 + 给选项**，未发生私自越界。这一协作模式有效，建议固化为协议：**越界前先问，附最小补丁预览**。
- **契约演进**：v1（C1~C5）→ v2（R1~R5，含"仅导出态 trace_id""secret 子串脱敏""header insert 语义""退出预算三段≤3000ms"）单文件增量保留历史，未删 v1。

## 四、 元协议迭代建议 (Self-Evolving Protocol)
1. **P0 修缺陷回派必须给"目标态原文"**：本轮 F1 三轮返工的直接教训。建议 dev-team skill 增加一条：回派 BLOCKING 时，必须贴出「期望的代码/配置片段」，仅描述"删除某段"不算合格回派。
2. **P0 禁止跨工作树共享 `target/`**：`CARGO_TARGET_DIR` 必须独立。已由执行者确认并内化；建议写入 skill 的「共享工具链互斥清单」。
3. **P1 新增"陷阱先布"测试范式**：凡断言"不应该发生"的用例（不误归因/不回落/不带占位值），必须先有一条断言证明**陷阱已生效**（否则恒真）。本轮用例 #6 即范本。
4. **P1 门禁必须能证明"错误实现会红"**：本轮 3 次假绿（两次 E2E 弱断言、一次 store 用例）都靠人工变异对照发现。建议把「关键契约用例需附变异对照收据」写入验收模板。
5. **P2 声明文件与门禁的实际效用**：`audit-diff.sh` 对本仓当前输出 `WARN 规则集为空（门禁空转）`——因为 `docs/observability.yaml` 刻意不声明 `enforcement_points`（reqwest 仍 4 处直连，无法用正则可靠拦截）。这是**诚实的空转**，但需在 OBS-GOV backlog 里建 http_client 收口点，否则该门禁长期无判定力。
6. **P2 证据文件自污染**：`cargo test --test '*'` 会重写 `.dev-team/evidence/wave15/wire-red.json`（工具清单快照）。本轮 3 次手工 `git show HEAD:` 复原。建议该用例改为写入临时目录或 `#[ignore]`，避免每次全量测试污染工作树。
