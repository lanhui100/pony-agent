# PA-024 构建模型监控面与 telemetry 聚合边界

## 状态
- Status: `Done`
- Priority: `P2`
- Owner: `Codex`

## OpenSpec Change
- [openspec/changes/archive/2026-06-01-add-model-monitor-telemetry-observability](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive/2026-06-01-add-model-monitor-telemetry-observability)

## Canonical Spec
- [model-monitor-telemetry/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/specs/model-monitor-telemetry/spec.md)

## Spec 状态
- `proposal ready`
- `spec ready`
- `design ready`
- `tasks completed`

## 目标
将现有占位态 `ModelMonitorPage` 升级为真实监控读面，形成围绕 provider / model / tool / session 的最小 telemetry 聚合能力，并把 retrieval / build-context / trace 证据以稳定 contract 暴露给前端消费。

## 本卡输出
- `HostControlPlane` summary 聚合读面：overview / provider / model / tool / session
- `HostControlPlane` session drill-down 读面：session metrics + runtime view
- Tauri command 暴露：`load_model_monitor_summary`、`load_model_monitor_session_drilldown`
- 前端 `ModelMonitorPage` 真实化：overview 卡片、聚合区、session 下钻、trace timeline、build-context 摘要
- Rust 与前端定向测试补齐

## 完成情况
- `src-tauri/src/agent/control_plane.rs`
  已新增 monitor summary / drill-down contract、聚合 helper、排序逻辑与定向测试。
- `src-tauri/src/lib.rs`
  已新增并注册 monitor 相关 Tauri commands。
- `src/types/runtime.ts`
  已补齐 monitor overview / dimension / tool / session / drill-down 类型。
- `src/components/ModelMonitorPage.vue`
  已从占位页升级为真实监控页，不依赖 runtime store 重算聚合。
- `src-tauri/src/agent/runtime.rs` / `src-tauri/src/agent/session.rs`
  已将 trace timeline `kind` 收敛为 `prepare_retrieval / build_context / call_model / call_tool / return_result`，并兼容历史旧值归一化。
- `src/stores/runtime.ts` / `src/components/HomeSidebar.vue`
  已统一消费 canonical trace semantics，避免 monitor drill-down 与主 trace UI 的语义口径继续漂移。
- `tests/ModelMonitorPage.spec.ts`
  已覆盖摘要加载、自动下钻、切换 session、错误态、非 Tauri 态。

## 依赖边界
- `PA-025` 继续提供 build-context / retrieval explanation 输入。
- `PA-029` 继续提供 cache telemetry 与 request-kind 输入。
- `PA-024` 只消费这些输入并形成 monitor read-plane，不重写 runtime / provider 执行逻辑。

## 验证
- 前端：
  `cmd /c npm run test:unit -- tests/ModelMonitorPage.spec.ts tests/HomeSidebar.spec.ts tests/HomeSessionSidebar.spec.ts tests/runtime-store.spec.ts`
- Rust：
  `cargo test load_model_monitor --manifest-path src-tauri/Cargo.toml`
  `cargo test persisted_trace_timeline_uses_canonical_monitor_semantics --manifest-path src-tauri/Cargo.toml`

## 后续
- 无。后续若扩展过滤器、趋势图、告警或跨会话分析，应另开增量任务卡。
