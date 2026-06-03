# 2026-06-01 Session 60 - PA-024 trace semantics 收口与 OpenSpec 归档

## 本次目标
- 补齐 `PA-024` 在 OpenSpec 中要求的 trace semantic contract。
- 在验证通过后，完成 `add-model-monitor-telemetry-observability` 的归档收口。

## 本次实现
- 后端 runtime 生成的 trace timeline `kind` 已从旧值
  - `context / model / tool / return`
  收敛到 canonical 值
  - `prepare_retrieval / build_context / call_model / call_tool / return_result`
- 当 retrieval 参与请求准备时，新增显式 `prepare_retrieval` step。
- session 读取时会把历史旧 timeline kind 归一化，避免 monitor drill-down 暴露旧合同。
- 前端 `runtime store`、`HomeSidebar` 与 `ModelMonitorPage` 测试口径同步到 canonical trace semantics。

## 涉及文件
- [runtime.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/runtime.rs)
- [session.rs](/C:/Users/HUAWEI/Documents/pony-agent/src-tauri/src/agent/session.rs)
- [runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/types/runtime.ts)
- [runtime.ts](/C:/Users/HUAWEI/Documents/pony-agent/src/stores/runtime.ts)
- [HomeSidebar.vue](/C:/Users/HUAWEI/Documents/pony-agent/src/components/HomeSidebar.vue)
- [ModelMonitorPage.spec.ts](/C:/Users/HUAWEI/Documents/pony-agent/tests/ModelMonitorPage.spec.ts)
- [runtime-store.spec.ts](/C:/Users/HUAWEI/Documents/pony-agent/tests/runtime-store.spec.ts)
- [HomeSidebar.spec.ts](/C:/Users/HUAWEI/Documents/pony-agent/tests/HomeSidebar.spec.ts)

## 验证
- 前端：
  - `cmd /c npm run test:unit -- tests/runtime-store.spec.ts tests/ModelMonitorPage.spec.ts tests/HomeSidebar.spec.ts`
- Rust：
  - `cargo test persisted_trace_timeline_uses_canonical_monitor_semantics --manifest-path src-tauri/Cargo.toml`
  - `cargo test load_model_monitor_session_drilldown_returns_metrics_and_runtime_view --manifest-path src-tauri/Cargo.toml`

## 归档前判断
- `openspec/changes/archive/2026-06-01-add-model-monitor-telemetry-observability/specs/model-monitor-telemetry/spec.md` 与 [openspec/specs/model-monitor-telemetry/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/model-monitor-telemetry/spec.md) 哈希一致。
- `tasks.md` 已全部完成。
- `PA-024` 任务卡已更新为 canonical spec 链接与 archive change 链接。

## 结果
- OpenSpec requirement “Trace display semantics must be explicit and stable” 已有实现与测试证据。
- `add-model-monitor-telemetry-observability` 已具备归档条件，后续查阅应优先使用 canonical spec 和 archive change。
