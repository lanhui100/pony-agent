# task-6（B6 契约迁移）进度 · Test Agent

## 已完成（不依赖 executor-core）
1. `tests/TelemetryPage.spec.ts:176` 契约迁移：
   - toast 断言 `至 PonySentry` → `至 Langfuse`；
   - 追加 `wrapper.html()` 不含 `PonySentry`（覆盖 `aria-label`/tooltip 属性级泄漏）；
   - 追加 `telemetry-upload-independent-note` 存在断言（A5/UI-2）。
2. `tests/settings.store.spec.ts:44` 严格载荷断言：
   - `toEqual` 载荷列出 D7 全部键 `{ workspaceMode: "work", agentTraceReporting: false }`
     （TS `AppSettings` 契约仅这 2 个字段；`toolRules` 不在前端契约内）。
3. OT-7 扩扫用例 `ot7_retired_symbols_have_zero_hits_repo_wide`：
   - 扫描面 `crates/**`、`src-tauri/src/**`、`src/**`、`tests/**`（.rs/.ts/.tsx/.js/.vue）；
   - 排除 `docs/`、`.dev-team/`、`openspec/changes/archive`（`openspec`/`archive` 目录名整棵跳过）、
     `target*`/`node_modules`/`.git`/`dist`，以及守卫文件自身（自指符号清单）；
   - `[PENDING task-4]`：3 个待删存量测试的 20 处命中登记为「已知待清理」，不计入失败；
     task-4 删文件后自动收紧为全仓 0 命中（常量可留可删，无副作用）。

## 红相收据（真实输出）
### A. `cargo test --manifest-path src-tauri/Cargo.toml -p pony-agent-core --test otel_trace_migration_test --target-dir target-test-exact-a ot7_` → Exit 101
```
OT-7 全仓守卫：D6 退役符号必须 0 命中（[PENDING task-4] 已登记待清理 20 条不计入失败），实际残留 23 条：
```
真实残留分布：`ponysentry/{mod,models,client}.rs`、`turn_flow.rs:978`（`maybe_report_turn_trace`）、
`control_plane/query_commands.rs:62,70`。
`test result: FAILED. 1 passed; 2 failed`（通过的 1 条为保留面双向守卫 `ot7_issue_reporting_keep_surface_still_exists`）。

### A′. 自指排除方式修订（Lead 复核项，已改为按常量定义块掩码）
原实现按**整个守卫文件**跳过（`continue`）——会漏掉该文件其他位置的残留。现改为：
- 只掩码 `RETIRED_TRACE_SYMBOLS` 常量定义块（`mask_retired_symbol_list`：逐字节等长替换为空格、保留换行 ⇒ 行号不漂移）；
- 文件其余内容照常扫描，并有两条完整性断言：
  `masked.len() == source.len()`（掩码等长）+ 13 个符号必须都落在被掩码块内（掩码不得漏扫）；
- 断言 `guard_file_scanned`：守卫文件**必须**处于扫描面内（机械证明不存在整文件排除）。
复跑结果与修订前完全一致（23 条残留 / 20 条 PENDING / `1 passed; 2 failed`，Exit 101），
且守卫文件自身 0 命中 —— 证明掩码生效且未整文件排除。

### B. `npx vitest run tests/TelemetryPage.spec.ts tests/settings.store.spec.ts` → Exit 1
```
Test Files  2 failed (2)
      Tests  2 failed | 8 passed (10)
```
- `expected '已成功上传会话 Trace（包含 3 轮对话链路）至 PonySentry' to contain '…至 Langfuse'`
- `expected "vi.fn()" to be called with arguments: [ 'save_app_settings', …(1) ]`（缺 `agentTraceReporting: false`）
即：2 处按新契约必红，其余 8 条无连带回归。

## 未完成（阻塞于 task-3）
- ~~OT-8d~~ **已转绿**（17:23）：`lib.rs:588/596` 双路径桥接 + `otel/mod.rs:332` core API（转调门禁）。
- ~~OT-8e~~ **已转绿**（17:23）：`lib.rs:1236 otel::init` / `lib.rs:1348 RunEvent::Exit → otel::shutdown()`。
- 收据：`.dev-team/evidence/wave43/test-agent-ot8de-green.md`、`green-ot8de-1726.log`。
- 前端契约同步复核：`TelemetryPage.spec.ts` + `settings.store.spec.ts` + `otel-reporting-switch.spec.ts`
  → 3 files / 14 tests 全绿。
- 剩余红相仅 OT-7 ×2（D6 退役未完成）：待 executor-core 删除 ponysentry trace 实现后，
  再由 task-4 删除 `PENDING_TASK4_CLEANUP_FILES` 三个存量测试，守卫自动收紧为全仓 0 命中。

## 收口（17:30，task-4 完成 + 守卫收紧）
1. `git status` 事实：`ponysentry_trace_test.rs` / `trace_auto_report_test.rs` 已被删除（task-4），
   `ponysentry_manual_upload_test.rs` 已重写为控制面形状 + 无回流断言（3 用例）。
2. 自指污染修复：重写文件原本用**负向断言字面量**（`!body.contains("send_agent_trace_direct")` 等）
   书写退役符号，会被 OT-7 扩扫误判为残留。现改为：
   - 退役符号 0 命中由 OT-7 独家机械守卫（避免双写导致的自指污染）；
   - 控制面改用 `!body.contains("ponysentry::")` 表达「不得回流」；
   - `PENDING_TASK4_CLEANUP_FILES` 与 `pending` 分支**整体删除** ⇒ OT-7 变为全仓 0 命中强断言。
3. 全量复跑：`cargo test --manifest-path src-tauri/Cargo.toml -p pony-agent-core --test otel_trace_migration_test --target-dir target-test-exact-a`
   → **`21 passed; 0 failed`，Exit 0**（含 OT-1/2/3/4/6/7×3/8×5）。
   收据：`.dev-team/evidence/wave43/green-all-1730.log`。
4. 已登记观察项（不擅自扩验收面）：`test-agent-gap-auto-trace-wiring.md` —— 生产代码中
   `TurnScope` / `llm_span_attributes` / `tool_span_attributes` 调用点均为 0，自动回合上报未接线，
   等 Lead 裁决是补 task-3 还是降级为 Non-Goal。

## 追记（2026-10-24 17:22 续跑）
OT-8d/8e 断言**已补写**（不再等桥接落盘，先红后绿），实现要点：
- `function_body()`：按花括号配对截取函数体，不闭合直接失败（禁止静默截断）；
- `find_core_gate_api()`：在 `crates/pony-agent-core/src/agent/**` 内定位 `pub fn set_agent_trace_reporting(`，
  并要求其函数体调用 `set_reporting_enabled(`（防「空桥接」假绿）；
- 8d：`load_app_settings` / `save_app_settings` 双路径都必须调用且必须传入持久化字段
  （`agent_trace_reporting` 字面量，防硬编码常量）；
- 8e：`src-tauri/src/**` 必须存在 `otel::init(`；`otel::shutdown()` 必须落在 `RunEvent::Exit` /
  `ExitRequested` **同一分支内**（用「两锚点之间不得出现下一个 `RunEvent::`」机械证明不跨 arm）；
- 防塌缩：扫描面文件数下限 + 锚点文件（`lib.rs` / `main.rs`）必须在扫描面内。

复跑（`target-test-exact-a`，源码头 17:20:35）→ Exit 101，`16 passed; 5 failed`：
- 8d：`core 公共 API set_agent_trace_reporting(bool) 未定义`（当时 otel 侧 API 尚未落盘）
- 8e：扫描面下限误设为 `>10`，实际 10 个 `.rs` ⇒ 已修正为 `>=5` + 锚点文件断言
- OT-6：`OTLP/HTTP trace 导出路径必须为 /v1/traces，实际 /`
- OT-7 ×2：退役符号残留 23 条（含 3 个 pending 文件 20 条）
收据：`.dev-team/evidence/wave43/red-ot8de-1721.log`。

> 注：otel 侧 `set_agent_trace_reporting` 已于 17:20:35 落盘（`agent/otel/mod.rs:332`，转调
> `set_reporting_enabled`），8d 剩余缺口仅在 `src-tauri/src/lib.rs` 双路径桥接；8e 缺口在退出路径。
