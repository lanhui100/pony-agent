# F1 速度口径可取证 + 显示诚实化 —— 物理收据（exec-metrics / task-4）

- 时间：2026-11-12 19:37–20:20 (+08:00)，工作区 `/home/dm/pony-agent`（HEAD 之上含 43 项无关未提交改动，全部保留）
- 契约：task-4 I1–I5 + Lead 两个追加点；口径修订见 [`contract-v3.md`](contract-v3.md)

---

## 1. 改了什么（文件:行 + 契约映射）

| 契约 | 落点 | 实现要点 |
|---|---|---|
| I1 结构化事件 | `crates/pony-agent-core/src/agent/turn_event.rs:202`（新变体 `TurnSpeed`，`type_name="turn/speed"`）；派发在 `crates/pony-agent-core/src/agent/turn_flow.rs:1033`（`turn/end` **之前**，同批同事务 flush） | 每轮终态一条，键 `turnSpeedTokensPerSecond`/`turnSpeedBasis`/`turnCallsTotal`/`turnCallsEligible`/`turnBurstExcluded`（+`turnExcludedTotal`/`turnCallsTtftMissing`）；`None` ⇒ serde 整键缺席（禁 0/NaN 占位）；无记录且无快照 ⇒ 不写孤儿事件 |
| I2 同帧日志 | `turn_flow.rs:1281`（`settle_turn_speed_evidence`），`TURN_SPEED_LOG_MESSAGE="turn speed settled"`（`turn_flow.rs:2846`） | `otel::log::log_json_with_fields("info", …)` 单行 JSON，键与事件逐字同名 + `turn_id` |
| I3 同源同帧 | 速度/口径直取 `payload.metrics`（= `build_turn_metrics_snapshot` → `weighted_turn_speed`）；样本数走唯一判据出口 `turn_flow::turn_call_census`（`turn_flow.rs:2806`，复用 `call_valid`/`decode_eligible`/`decode_floor_ms`） | TUI/TS 均只读快照/映射，无第二公式 |
| I1 wire 取证 | `crates/pony-agent-core/src/agent/metrics_snapshot.rs:56/59/66`（`turn_calls_total`/`turn_calls_eligible`/`turn_burst_excluded`，`Option`+`skip_serializing_if`），赋值 `:314-316` | `valid==0` ⇒ 三键全缺席（样本量无定义，不写 0） |
| I4 TUI | `crates/pony-agent-core/src/bin/tui.rs:4075`（`format_speed_segment`），调用点 `:4189` | 逐字：`120 t/s decode n=1` / `≈120 t/s e2e n=3` / 突发 `>0` 追加 ` 剔突发N`；basis 缺席只出裸值；`turn_calls==0` 不写 `n=` |
| I4 桌面 | `src/components/HomeStatusPanel.vue:107/114`（tooltip 样本量 + 剔除数）、`:481-483`（`data-speed-calls`/`data-speed-calls-total`/`data-speed-burst-excluded`）；`src/lib/runtime/token-metrics.ts:699/722`（`formatSpeedSampleLabel`/`formatBurstExcludedSuffix`）与 `:845-866`（`resolveMetricsSnapshotDisplay` 追加样本字段） | 行内位点仍只「图标+值」两节点（wave-61 §1 L-4/R1 冻结结构），诚实化信息走 tooltip + `data-*`；无值不占位 |
| 数据透传 | `src/types/runtime.ts:1172-1186`（3 个可选键）；`src/lib/runtime/useTraceProjection.ts:393-397`（快照分支透传 `callsTotal`/`callsEligible`/`burstExcluded`） | 仅新增可选键，语义不变（Lead 批准扩大写域） |
| 契约门禁同步 | `crates/pony-agent-core/tests/ts_contract_snapshot.rs:121-127`（**仅** 补 3 个字面量 `Some(3)/Some(1)/Some(1)`，未动任何断言）；`docs/contracts/ts-contract.snapshot.json` 由工具重生成（+3 键） | Lead 授权范围内 |

### 口径（契约 v3，代码 doc 已各写一行）
- `turnBurstExcluded` = **真突发整包数**（valid 且有 ttft、`dur>ttft`、`out>=2`，但 `gen < floor(dur)`）；
- `turnExcludedTotal` = `valid − decode_eligible`（全部解码剔除 = 突发 + 无 TTFT + 单 token）；
- `turnCallsTtftMissing` = `valid − 有 ttft 的 valid`（无 TTFT 不可测；诊断里 73% 整包到达的根因面）。

---

## 2. 验证命令 + Exit Code（真跑）

### Rust
```
$ cargo check -p pony-agent-core
EXIT 0        # 唯一 warning：provider/mod.rs:37 unused import（exec-core 写域，非本次改动）

$ cargo test -p pony-agent-core --lib
EXIT 101      # 1525 passed / 2 failed —— 两条失败均只是「期望事件序列缺 turn/speed」，已由 Lead 派给 test-agent：
              #  agent::control_plane::tests::graph_sync_run_turn_persists_event_stream_end_to_end
              #    left : [..., "provider/usage", "step/end", "turn/speed", "turn/end"]
              #    right: [..., "provider/usage", "step/end",              "turn/end"]   (control_plane/mod.rs:8063)
              #  agent::runtime::tests::run_turn_and_start_turn_stream_produce_equivalent_event_sequences
              #    (runtime/tests.rs:4267，同一差异)
              # 两条都在他人写域（control_plane/、runtime/），本 Agent 不改。

$ UPDATE_CONTRACT_SNAPSHOT=1 cargo test -p pony-agent-core --test ts_contract_snapshot
EXIT 0        # 重生成 docs/contracts/ts-contract.snapshot.json（+3 键）

$ cargo test -p pony-agent-core --test ts_contract_snapshot
EXIT 0        # 1 passed（去掉 UPDATE 环境变量复跑确认）
$ cargo test -p pony-agent-core --test tui_metrics_snapshot
EXIT 0        # 133 passed
$ cargo test -p pony-agent-core --test tui_metrics_dual_caliber
EXIT 0        # 137 passed
$ cargo test -p pony-agent-core --test f1_speed_observability
EXIT 0        # 4 passed（Test Agent 冻结文件，未改动）
```

### 前端
```
$ npx vitest run src/lib/runtime
EXIT 0        # 6 passed（含冻结的 src/lib/runtime/__tests__/f1-token-metrics-sample.spec.ts 6/6）
$ npx vitest run tests/metrics-snapshot-live.spec.ts tests/metrics-percent-honesty.spec.ts
EXIT 0        # 9 + 7 passed（既有 D1 逐字段断言不被新字段打破：旧 payload ⇒ 新增键值 undefined）
$ npx vue-tsc --noEmit
EXIT 0
$ npx vitest run          # 全量
EXIT 1        # 161 files passed / 2 failed（1517 passed / 2 failed）——两条均为**工作区环境态**，与 F1 代码无关：
              # ① tests/acceptance/wave47-ob2-enforcement.spec.ts T7：外部 `ponygo observe audit --level ob2` 返回 1，
              #    原因输出逐字：「[D1] 声明在本次变更中被修改，需要显式批准」，即工作区里 `docs/observability.yaml`
              #    已被**他人**改动 +39 行（git status: ` M docs/observability.yaml`，本 Agent 未触碰）且无
              #    `[decl-change-approved]`。ob0/ob1 与四维一致性均 PASS，判定只看提交态，脏工作区结构上不可绿。
              # ② tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts V10-3：内层**再跑一遍全量套件**并断言 exit 0
              #    ⇒ 由 ① 派生（同一根因，非第二条独立缺陷）。
```

---

## 3. 取舍与风险（明确登记）

1. **I1 事件新增 `TurnEvent::TurnSpeed` 变体**：`turn_event.rs` 原不在写域，但如果只在既有 `TurnEnd` 上扩字段，会要求改 `session/history.rs`、`sqlite_session.rs` 等更多域外文件的穷尽构造；新增变体是改动面最小的路径（仅 2 处 match 臂 + 1 个变体），已获 Lead 批准。
2. **突发口径 v3**：`turnBurstExcluded` 只数真突发整包（与冻结夹具一致），契约原文的 `valid−eligible` 保留为 `turnExcludedTotal`（两个数都落盘/落日志，互不冒充）。若后续要单值口径，改 `turn_flow::turn_call_census` 一处 + 快照字段映射一处即可。
3. **桌面显示走 tooltip + `data-*` 而非行内后缀**：wave-61 §1 L-4 / 裁决 R1 冻结「速度位行内恰为图标+值两节点」，加行内子行/后缀会直接打破既有布局验收；契约 I4 明确允许 tooltip 路径。
4. **`resolveMetricsSnapshotDisplay` 新增字段全部「undefined 可省」**：`tests/metrics-snapshot-live.spec.ts` D1 用 `toEqual` 逐字段相等（旧 payload 下新键值为 `undefined` 才不破坏），因此新字段在「F1 三键全缺」时一律 `undefined`，而不是 0/null。
5. **未改：`llm.chat` span 的 per-call dur 残差（Lead 追加点 1）**。取证结论（read 了 `agent/retry.rs` 与 `runtime/turn_*.rs`）：attempt 账本是**线程局部一次性**台账——`begin_provider_attempt_window()` 开账、`take_provider_attempt_window()` 取走即清空；运行时的调用序是「runtime 开账 → 调 `turn_flow::provider_decision_*`（span 在此）→ runtime `take_pure_call_window` 拿纯净窗口写 per-call 记录」。因此在 turn_flow 内：
   - **取走**台账 ⇒ 运行时拿不到（`F2` 的 per-call dur 立刻退回含重试等待的脏值）；
   - **再开一次** `begin_provider_attempt_window()` ⇒ 用新状态覆盖外层台账，同样毁掉 F2；
   - `retry.rs` 无 peek 接口，且该文件不在写域。
   正确修法在 F2 侧：运行时把已结算的 `PureCallWindow` 传进 span（`record_call_timing`），或 `retry.rs` 提供 `peek/offset` 只读接口。**本项未改，留给 exec-core / F2 决策**（OTel 导出当前 401 全丢，非 F1 取证面，故不影响 I1–I5）。
6. **全量前端 2 红为工作区环境态**（见 §2 末），非本次改动引入；`docs/observability.yaml` 属他人写域，本 Agent 不触碰、不回退。
