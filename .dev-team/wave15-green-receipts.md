# Wave-15 绿相物理收据（Token 指标口径修复）

- 契约：`.dev-team/contracts/wave-15-token-metrics.md`（含 §7.6 解析序、§7.10 R1 载荷不变量）
- 根因证据：`.dev-team/evidence/wave-15-root-cause.md`
- 红相锚定：`f38b6b9`（tag `wave-15-red`）→ 用例缺陷修正 + §7.10 重锚：`3475e53`
- 分级：B 级（跨 Rust 运行时 ↔ 前端投影协议语义；单特性域）

## 1. Join Barrier 前置机器任务

| 项 | 结果 |
|---|---|
| 机械写域审计 | 本波实际写入 = 10 个源文件 + 2 类测试文件，全部落在 task-2/3/4/5/6 声明范围内；唯一越界 `src/lib/runtime/types.ts`（仅新增 `RuntimeState.activeProviderCallRecords` 类型字段）按「编译衍生豁免」由 Lead 核准 |
| blast-radius | union diff = 329 insertions / 159 deletions（>100 行阈值）→ 自动触发 ≥2 路互补 L3 审查（A 并发/状态流转、B 协议一致性）+ L3-P 五问否决 |
| NFR 数值比对 | `.dev-team/nfr-baseline-wave15.json` 通过 `nfr-baseline.schema.json`（jsonschema Exit 0）；本波无新增外部调用/锁/队列/句柄 |
| 审查 JSON Schema | `review-l3a/-l3b/-l3p-wave15.json` 全部 verdict=PASS、findings=[]、字段齐全（脚本校验 Exit 0） |
| 非本波改动隔离 | `src/components/HomeWorkspace.vue`、`src/components/chat/WorkspaceComposer.vue`、`src/lib/useTimelineAutoScroll.ts`、`tests/acceptance/status-flow-contract.spec.ts`、`.gitignore` 属并发波次（wave-16 ask-cas），**未纳入本波提交、未做任何改动** |

## 2. 绿相机器收据（原始 stdout 尾部 + Exit Code）

| 命令 | Exit | 结果 |
|---|---|---|
| `cargo test --manifest-path src-tauri/Cargo.toml --target-dir target-test -p pony-agent-core --lib agent::runtime::tests::wave15_multihop_stream_exposes_per_call_metrics_in_hop_events_and_persisted_timeline` | **0** | `1 passed; 0 failed`（R1 不变量 I1/I2/I3 + R2 per-call 条目 + output_end 载荷） |
| `cargo test … --lib agent::runtime::tests::start_turn_stream_accumulates_token_usage_across_tool_followups` | **0** | `1 passed`（S2 累加口径回归，未破坏） |
| `cargo test … --lib agent::runtime::tests::persisted_trace_timeline_uses_canonical_monitor_semantics` | **0** | `1 passed`（legacy 薄委托路径回归） |
| `cargo check --manifest-path src-tauri/Cargo.toml --target-dir target-check -p pony-agent-core` | **0** | `Finished dev profile` |
| `npx vue-tsc --noEmit` | **0** | 无 TS 错误 |
| `npx vitest run tests/acceptance/wave15` | **0** | `2 files passed / 20 tests passed`（T1–T6 + A1–A5 + wire 跨语言 6 例） |
| `npx vitest run tests/context-display-tokens.spec.ts tests/AgentRuntimeStatusBar.spec.ts tests/runtime-status.spec.ts tests/HomeSidebar.spec.ts tests/TraceInspector.spec.ts` | **0** | `5 files passed / 46 passed / 2 skipped` |
| `npx vitest run`（全量前端） | 1 | `865 passed / 1 failed / 10 skipped`；唯一失败 `tests/workspace-runtime-status-streaming.spec.ts`（断言 `data-visible=false`）由并发波次对 `HomeWorkspace.vue` 的未提交重写引起（见 §1 隔离项），**与本波无因果** |
| `node tests/acceptance/wave15/browser-smoke.mjs` | **0** | 真实 Chrome(headless)：`consoleErrors=[] pageErrors=[] composerInput=true contextUsageCircle=true streamingFallbackRendered=true`；截图 `.dev-team/evidence/wave15/browser-smoke-red*.png` |

## 3. 跨语言物理凭证（wire-replay）

- 红相冻结：`.dev-team/evidence/wave15/wire-red-frozen.json` —— 真实 mock SSE 三跳事件序列，`hop_complete.providerCallRecords = null`，累加字段 710000/1080000（缺陷实况）。
- 绿相冻结：`.dev-team/evidence/wave15/wire-green-frozen.json` —— 同一用例修复后实况：`hop #0 records=[350000,360000]`（Σ=710000 ✓）、`hop #1 records=[350000,360000,370000]`（Σ=1080000 ✓）、`output_end records` 同（I1/I3 成立），持久化 timeline 三条 `call_model` 各持自身指标、`checkpoint_persist` 持整轮聚合。
- 运行期刷新文件 `wire-red.json` 已登记 `.dev-team/evidence/wave15/.gitignore`（避免每次跑测的脏 diff）；冻结凭证入版本库。

## 4. 验收标准达成（用户口径）

1. **输入 token（状态栏 + 输入框上下文）**：3 跳轮次圆环与状态栏「输入」均显示 **370K**（最近一次调用），不再显示 1.08M 累加值；无 per-call 来源的多调用轮显示无数据而非虚高（S4）。
2. **输出生成速度**：单次调用完成即产出（含轮内中间 hop）；测试覆盖 100 output / 1200ms / TTFT 200ms → 100 t/s，且不会用整轮累加值或整轮 elapsed。
3. **回归**：S2 累计计费口径、消息级 tokenCount、ModelMonitor 汇总语义未变更；既有 3 条上下文锚点测试全绿。
