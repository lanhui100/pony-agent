# Wave-47 · ob2 收口点「真实链路」验证报告

> 目标：验证本轮 ob2 改动（`docs/observability.yaml` 的两处 `enforcement_points` + CI 挂载）
> 在**真实链路**上成立，而不只是静态声明通过。
> 所有结论均附可复跑的机器收据（同目录 `*.log`）。

## 验证矩阵

| 层 | 验证对象 | 手段 | 结果 | 收据 |
|---|---|---|---|---|
| **L0** | 门禁拦截力（真实工作树） | 在真实仓里写入违背收口点的代码 → 跑 `audit-diff.sh` | **两条规则均拦截（exit 2）**，清理后放行（exit 0） | `l0_gate_probe.txt` |
| **L2** | `http_client` 的 `traceparent` 保证 | 裸 `TcpListener` 捕获 reqwest 实际写出的 HTTP/1.1 原始字节 | **线上真实出现 `traceparent: 00-<32hex>-<16hex>-01`**；无活跃 turn 时不注入 | `l2_wire_traceparent.txt` |
| **L3** | `error_boundary` 真实渲染链路 | 真实 Vue 运行时 + 真实 HTTP 服务端；默认静音 vs 主动开闸两态对比 | **默认零出网；开闸后真实出网，breadcrumbs/trace_id/归因三件保证齐全** | `l3_realpath.txt` |
| **L1** | CI 是否真的执行门禁 | 推分支 + PR，看 Actions 实跑 | **Observability Gate `success`**，日志有 `base=origin/main` 与扫描行；且**一次真实拦截**证明非橡皮章 | `l1_ci_gate.txt` |
| **回归** | 全量前端套件 | `npx vitest run` | **107 文件 / 1055 用例通过**，exit 0 | `full-vitest-suite.txt` |

## L0 · 门禁拦截力（真实工作树）

复跑：`bash tests/acceptance/receipts/wave47/verify-l0.sh`

- 违背 `http_client`（手写 `req.header("traceparent", ...)`）→ `EXIT=2`，报错定位到 `文件:行号` 并给出修复指引；
- 违背 `error_boundary`（别处散落 `onErrorCaptured(`）→ `EXIT=2`，同理；
- 清理后 → `EXIT=0`（`[OK] audit-diff: 无越界实现、收口点完整`）。

**结论**：收口点不是纸面声明，真实改动会被物理拦住。

## L2 · `traceparent` 真的上线（最关键的一条）

用例：`crates/pony-agent-core/tests/wave47_ob2_wire_traceparent.rs`

不复用既有单测的 `HeaderMap` 层面断言，而是：

1. `TcpListener` 绑 `127.0.0.1:0`，收一条真实连接；
2. `begin_turn_trace(...)` 建立活跃 turn（注入前提）；
3. 用**真实 reqwest client** 经 `with_current_traceparent` 发出真实 HTTP 请求；
4. 从套接字读回原始字节，断言头真的在报文中。

实测收据（每次随机，形态固定）：

```
GET /v1/probe HTTP/1.1
traceparent: 00-6eb3ce536b0f481bada70e7b6d04d804-fe0cf6d9b8d541bb-01
```

反相对照用例（同文件 `wave47_l2at_*`）：**无活跃 turn 时线上不得出现 traceparent**。
这条对照是鉴别力来源——若头来自 reqwest 默认值或夹具污染，反相用例会红。

## L3 · 前端：默认静音 + 主动开闸后真实出网（已闭合）

**默认态（T1）**：真实 Vue 运行时里挂载真实 `ErrorBoundary.vue`，子组件在 `setup()` 抛错后
渲染 `data-testid="error-boundary-fallback"` 降级卡片并 `emit("error")`；同时
`process.env.VITEST` 静音守卫拦下上报，**零出网**——这是默认行为。

**主动开闸（T2）**：复用本仓既成放行模式（`tests/telemetry-credential.spec.ts`
的 `enableBrowserEgressPath()`：`vi.stubEnv("VITEST", "")`，作用域限于当前用例，
`afterEach` 复原），把上报打到真实 HTTP 服务端。服务端真实收到的报文（节选）：

```json
{
  "event_id": "b8ee2c6c5b524af899583da60a9816c3",
  "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736",
  "message": "WAVE47-L3 simulated component explosion",
  "tags": { "project": "pony-agent", "source": "error_boundary", "componentTag": "Wave47ProbeSection" },
  "breadcrumbs": [
    { "category": "navigation",  "message": "wave47:open-workspace" },
    { "category": "user_action", "message": "wave47:send-message" }
  ]
}
```

三件保证全部落地：`breadcrumbs` 有内容、`trace_id` 与 Rust 侧出网注入同源、
`tags.source === "error_boundary"` 证明走的是该收口点而非旁路上报。

**开闸护栏（T3）**：开闸前强制校验 ingest 端点为环回地址，非环回直接抛错。
即「主动开启」这条能力自带安全边界：即使 CI 误注入了生产端点，开闸也会当场失败，
不会把测试噪声打进生产 Sentry。

**为什么没改产品代码**：「默认关闭、可主动开启」**本来就成立**——守卫读的是运行时
`process.env.VITEST`，测试侧 stub 即可单点开闸。若在 `telemetry.ts` 里新增 env 开关，
等于把「测试能开闸」永久暴露到生产形态，还要修订已实现的 ADR 决策，收益为负。

> 修正：本报告早前版本曾判定「真实出网在 vitest 里结构性不可观测」，那是**错的**——
> 既有 `enableBrowserEgressPath()` 已提供单点开闸模式，本轮直接复用即可。

## L1 · CI 真的会跑门禁（本轮新增）

### 为什么原来那条 CI 步骤是坏的

1. `verify` job 是 `windows-latest`，默认 shell 是 **pwsh**，原步骤写的 `if [ -f ... ]` 会直接语法失败；
2. 即使 shell 对了，GitHub 托管 runner **没有** `~/.agents/skills/observability-onboarding/`，
   原步骤会**静默空转**——门禁「装了但什么都没拦」，是最糟的假绿。

### 处置

- 拆出独立 workflow `.github/workflows/observability-gate.yml`（只依赖 git + node，秒级独立红绿）；
- runner 无 skill → 走仓库内引擎 `scripts/observability/gate.mjs`；本机有 skill → 仍用权威器；
- 两个引擎的判定口径由 `gate-conformance.spec.ts`（4 条）与 D1 语义回归（3 条）锁死。

### 物理收据

| 运行 | 结论 | 含义 |
|---|---|---|
| [40253502463](https://github.com/lanhui100/pony-agent/actions/runs/40253502463) | **failure（exit 2）** | 首次上线即**真实拦截**：`[D1] docs/observability.yaml 在本次变更中被修改，但其提交信息缺少 [decl-change-approved]` —— 证明门禁不是橡皮章 |
| [40253725543](https://github.com/lanhui100/pony-agent/actions/runs/40253725543) | **success** | 日志：`base=origin/main` / `[OK] 可观测性变更门禁通过：2 个收口点，扫描 229 个变更文件` —— 证明**真的执行了**且解析到正确 base |

### D1 批准语义（本轮踩了两次坑才收敛）

- 逐提交全查 → 引入口令约定前的历史改动会一次性全红，门禁随即被无视；
- 任一提交带口令 → 实测可搭便车：先批准一次，再追加未批准改动即可绕过；
- 最终口径：**区间内最后一次动过声明的提交**必须带 `[decl-change-approved]`。两端都堵住，
  并由 D1-a/b/c 三条回归用例锁定。

## 仍未绿的部分（与本轮改动无关，需另开工单）

`CI` 主 workflow 仍红，三处均为**既有问题**，本轮未夹带修改：

1. `verify-decisions.ps1`：`0024-ponysentry-agent-trace-ingest.md` 状态为 `superseded` 但仍在主目录（应移入 `superseded/`）；`0048-usage-governance-enforcement.md` 缺 `Status` 行。
2. `core-platform-check`（ubuntu/macos）：`crates/pony-agent-core/src/bin/tui.rs` 两处既有告警被 `-D warnings` 升级为错误（第 11 行未使用 import、第 1913 行未使用函数）。

## 复跑方式

```bash
bash tests/acceptance/receipts/wave47/verify-l0.sh          # L0
cargo test --manifest-path crates/pony-agent-core/Cargo.toml \
  --target-dir target-test --test wave47_ob2_wire_traceparent -- --nocapture   # L2
bash tests/acceptance/receipts/wave47/run-l3.sh             # L3（注入本地端点）
npx vitest run                                              # 全量回归
bash ~/.agents/skills/observability-onboarding/scripts/audit-diff.sh --decl-approved  # 收口门禁
```

## 本轮附带修正（测试缺陷，非实现改动）

| 用例 | 问题 | 处置 |
|---|---|---|
| `T2-D1-UNAPPROVED` | 断言宿主仓「声明恰好未提交」的状态，声明一提交即假红 | 改为自包含影子 git 仓库（与 T5/T6 同构） |
| `T7-PONYGO-OB2-GATE` | 冻结在红相（断言 `exit 1` / `当前级: ob1`），ob2 达标后必然假红 | 契约演进重锚为绿相（`exit 0` / `当前级: ob2`） |
| `W47-L3-T2/T3` | 依赖 runner 注入的 env，常规 `npm test` 下假红 | `it.skipIf` 未注入时跳过 |
