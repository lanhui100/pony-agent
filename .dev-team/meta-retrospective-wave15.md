# Wave-15 元复盘（Token 指标口径：输入上下文单次调用化 + 生成速度调用级即时化）

> 复盘对象：本次 Agent Team 的**协作元框架与组织协议**。账本 `.dev-team/state.json`（wave 15）、契约 `.dev-team/contracts/wave-15-token-metrics.md`、收据 `.dev-team/wave15-green-receipts.md`、根因 `.dev-team/evidence/wave-15-root-cause.md`。

## 1. 通信拓扑与信噪比

| 要素 | 实际 | 评价 |
| :-- | :-- | :-- |
| 名册 | Lead + Protocol + Test + Executor-Rust + Executor-Frontend（5 常驻 ≤ 8） | 合适：跨 Rust 运行时 ↔ 前端投影双写域，正交可并行 |
| 计算池 | 只读侦察 3 路 + 审查 3 路（L3-A/L3-B/L3-P）全部走 Subagent，未占常驻槽位 | 符合「无状态求值下沉」公理 |
| 看板 | 7 个任务（T-P1/T-T1/T-E1/T-E2/T-T2/T-T3/T-E1b）全部 completed，无 in_progress 残留 | 干净 |
| 侦察信噪比 | 3 路只读侦察均以 `file:line + 原文引用` 交付，**在任何代码写入前**锁定 5 条根因（RC1–RC5） | 本波最大增益：避免了「凭猜测改口径」 |
| 通信纪律 | 队友回报均为 ≤20 行 JSON（零吹捧）；跨阶段裁定以简洁消息下发 | 良好 |
| 噪声与浪费 | ① Lead 先自行做了约 10 次深读侦察，随后又派生 3 路侦察，存在**重复侦察**；② 自动续轮 + 3 次模型切换使会话变长 | 需改进（见 §4-⑤） |

## 2. 门禁穿透与误杀率

| 事件 | 事实 | 判定 |
| :-- | :-- | :-- |
| 红相真实性 | `cargo test --lib wave15_multihop` Exit **101**；`vitest run tests/acceptance/wave15` Exit **1**（17 failed/3 passed）；纯度分类：SUT 空桩 throw 13 例 + SUT 断言 4 例，**基建失败 0**（无 import/语法/缺文件型假红） | 门禁有效，未穿透 |
| **假绿拦截（本波最高价值）** | executor-rust 首版把 hop_complete 写成 `provider_call_records[..len-1]`（滞后一跳），而冻结用例当时的映射 `hopInputs[index]` 恰好与该错误实现同向 → 套件 **20/20 全绿**。Test Agent 主动上报 P2 异常（hop0 累加 710000 而 records 仅 350000），Lead 复核后裁定不变量 **I1（Σrecords == payload 累加）**，重锚红相（Exit 101/1），再修实现转绿 | **门禁在"测试与实现共享同一误读"时本会穿透**；靠 Test Agent 的独立复核 + Lead 集合不变量裁定救回。已转成元协议建议（§4-①） |
| 用例自缺陷 | 冻结用例 2 处自身缺陷（A4 夹具与 A5-b 语义互斥；wire R1 累加基线自指导致断言恒失败），由 Executor 上报而非自行改测；Lead 逐行复核确认后指派 Test Agent 以 `TEST-DEFECT` 方式修正，并用 pre-fix 空桩/冻结快照复跑证明**红相纯度未受损**（14 failed/14） | 分权有效；Executor 未越权改测 |
| 审查灵敏度 | blast-radius >100 行 → 强制 2 路互补 L3 + 1 路 L3-P，结论均 PASS/0 findings | **误杀率 0，但假阴性率未测**：未做"故意注错应被审出"的阴性对照（见 §4-①） |
| NFR 欠账 | `nfr-degraded` 0 项；无 L2-AT 用例被豁免 | 无欠账 |
| 债务登记 | `wave15-D1`（事件折叠 settle 覆盖 per-call duration）、`wave15-P3`（初始调用无 hop 事件窗口）显式登记，未夹带修复 | 符合最小闭包 |

## 3. 分工契约与隔离有效性

- **写域正交**：Rust（`turn_stream/trace_timeline/turn_persist/turn_sync`）与前端（`token-metrics/useTraceProjection/stores/Home*`）零重叠，真并行；`crates/.../runtime/tests.rs` 由 Test Agent 独占。
- **跨域编译阻塞规避**：R2 若直接改 `build_persisted_trace_timeline` 签名会砸掉 tests.rs 既有调用点（属 Test Agent 写域）。Lead 冻结「保留旧签名 + 新增 `*_with_provider_calls` 薄委托」方案 → 零跨域阻塞，legacy 行为与既有断言全部保持。
- **衍生豁免**：唯一越界 `src/lib/runtime/types.ts`（仅新增 `RuntimeState.activeProviderCallRecords` 类型字段）由 Executor 主动自报并经 Lead 核准。根因是 Lead 在任务卡写错路径（`src/types/runtime.ts`）——**Lead 侧声明缺陷**，已记入改进项。
- **并发多会话共享工作区**：另一波次（wave-15→16 ask-cas）在同一工作树并行写入（`HomeWorkspace.vue`、`AskPanel.vue`、`ask.ts`、`.gitignore` 等）。本波坚持**定向 `git add`、零全局 git 写操作**，全量前端套件唯一失败项（`workspace-runtime-status-streaming` 的 `data-visible`）被准确归因到对方的 `HomeWorkspace.vue` 未提交重写，而非本波。
- **契约演进可追溯**：3 次演进（§7.6 S1 前置条件、§7.10 R1 不变量、ADR 0035）全部由实证发现驱动，无漂移式改约。
- **协议特派员价值**：空桩 + NFR 基准 + §7 契约文本由 Protocol Agent 唯一落盘（含 `noUnusedParameters` 等工程细节），Executor 只填实现，避免"边写边定契约"。

## 4. 元协议迭代建议（对 dev-team skill 自身）

1. **绿相阴性对照（mutation negative control）** —— 本波出现"测试与实现共享误读 → 20/20 假绿"。建议 skill 增加硬门禁：Join Barrier 前，由 Test Agent 对 SUT 施加**一次单行语义突变**（如把 per-call 记录改回排除末条），断言冻结套件必须转红；若仍绿则视为门禁失效并打回。此项可直接机械化，成本 ≈1 次测试运行。
2. **指标/事件载荷契约必须写"集合一致性不变量"** —— 契约模板增加必填行：载荷中 per-item 集合与聚合字段的关系式（本波 I1: `Σ items == aggregate`）。本波正是靠该式识破滞后一跳；仅描述"应携带记录"不足以约束实现顺序。
3. **`TEST-DEFECT` 上报通道正式化** —— Executor 发现冻结用例自相矛盾时，必须输出 `{type:"TEST-DEFECT", minimal_repro, contradiction}` 并停止；严禁自行改测或降级断言。本波流程奏效，建议写入 skill 的协作元协议。
4. **波次编号与写域预检前置** —— 两会话同用 wave-15，对方被迫改为 wave-16。建议开局第一步：`git log --oneline` + `state.json` 最大波次编号 +1，并扫描未提交改动以识别并发写域（本波靠 git status 事后发现）。
5. **Lead 侦察预算纪律** —— 应先派生 3 路只读侦察拿全景，Lead 仅复核关键行（本波重复侦察浪费约 10 次工具调用与上下文）。建议 skill 写明"Lead 先分派侦察、再就 crux 行做单点复核"。
6. **审查灵敏度抽检** —— 3 路审查 0 findings 时应至少做一次低成本阴性对照（如故意引入越界写或吞异常），以区分"真干净"与"审查过松"。

## 5. 终态

- **红相锚定**：`f38b6b9`（tag `wave-15-red`）→ 用例自缺陷修正 + §7.10 重锚 `3475e53`
- **绿相**：`5901ef4`（tag `wave-15`）
- **门禁全绿（原始 Exit Code）**：wave15 Rust 用例 0 / 两条既有 Rust 回归 0 / `cargo check` 0 / `vue-tsc --noEmit` 0 / `vitest run tests/acceptance/wave15` 0（20 passed）/ 目标回归 5 文件 0（46 passed, 2 skipped）/ 全量 `vitest run` 865 passed（唯一失败归因于并发波次的未提交 `HomeWorkspace.vue`）/ 真实 Chrome 无头烟测 Console Error=0 + 2 张截图
- **审查**：L3-A / L3-B / L3-P 全部 PASS，findings=[]（JSON Schema 校验 Exit 0）
- **交付**：`trace_timeline.rs`（R2 per-call 条目）、`turn_stream.rs`（R1 per-call 记录透传 ×2）、`turn_persist.rs`/`turn_sync.rs`（调用点适配）、`src/lib/runtime/token-metrics.ts`（S1/S3/S4 纯函数）、`useTraceProjection.ts`（R3）、`stores/runtime.ts`（R4 活跃轮记录 + 复位）、`HomeSidebar.vue`/`HomeStatusPanel.vue`（显示与 tooltip）、ADR 0035、交付归档 `docs/release/2026-10-16-wave15-token-metrics/`
- **债务**：`wave15-D1`（projection settle 覆盖 per-call duration）、`wave15-P3`（初始调用窗口），均已登记
