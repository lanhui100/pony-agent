# 2026-05-31 Session 52 - PA-025 Closeout

## 背景

- 目标：拆分 `PA-025`，发起合理数量的子智能体并行推进，实现后完成验证并达到可交付状态。
- 范围约束：
  - 只收口 `Build Context` 与 cache-friendly prompt 边界
  - 不把 retrieval observability / telemetry 聚合混回本卡
  - 不接受与 `PA-025` 无关的 trace tool group / tool aggregation UI 偏航功能

## 拆分与委派

- 主智能体负责：
  - 主线边界判断
  - 集成收口
  - 最终验证
  - 任务系统回写
- 子智能体 `Hegel`
  - 负责后端方向勘察与补位
  - 关注 `context.rs / provider.rs / session_regression`
- 子智能体 `Bohr`
  - 负责前端方向勘察与补位
  - 关注 `HomeSidebar.vue / runtime.ts / 前端单测`

实际拆为三路：
1. 后端三层 observation 收口
2. 前端 trace 展示与类型补齐
3. 主线集成验证与任务系统同步

## 本轮收口

### 1. 后端

- `src-tauri/src/agent/context.rs`
  - 为 request observation 明确三层：
    - `stablePrefixText`
    - `semiStableContextText`
    - `volatileInputText`
  - 普通 request 与 provider-native transcript 两条路径都补齐观测构造
  - provider-native transcript 被截断时，observation 继续保留真实发送过的 truncation note
- `src-tauri/src/agent/provider.rs`
  - 暴露 `ProviderRequestObservation`
  - `BuildContextObservation` 承接三层语义
- `src-tauri/src/bin/decision_probe.rs`
  - 补齐 `ProviderRequestObservation::default()` 初始化
- `src-tauri/tests/session_regression.rs`
  - 补齐测试壳层所需模块，使回归测试可编译运行
  - 新增 `buildContextObservation` 持久化 / 回读覆盖，防止 session 序列化边界漂移

### 2. 前端

- `src/types/runtime.ts`
  - `BuildContextObservation` 新增三层字段
- `src/components/HomeSidebar.vue`
  - retrieval 卡片明确为 `Current retrieval state`
  - 每个 turn 下新增 `Sent build context`
  - 展示 request 基础信息、三层文本、request messages 和 tool definitions
  - 确保 retrieval state 与 sent build context 语义分离
- `tests/HomeSidebar.spec.ts`
  - 保留折叠结构测试
  - 补齐 retrieval 与 build context 分离展示测试
- `tests/runtime-store.spec.ts`
  - 补齐 build-context 工厂默认值
  - 验证 `turn:started` / `turn:completed` 写入与覆盖时都保留 clone 语义

### 3. 偏航清理

- 全文搜索确认未残留以下偏航标识：
  - `trace-tool-group`
  - `toolGroup`
  - `toolActivityState`
  - `toggleToolGroup`
  - `activeToolGroupKey`
  - `ToolActivityGroup`

## 验证

```powershell
npm run test:unit -- tests/HomeSidebar.spec.ts tests/runtime-store.spec.ts
cargo test --manifest-path src-tauri/Cargo.toml build_request_records_layered_observation -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml build_context_observation_ -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --test session_regression -- --nocapture
npm run build
```

结果：
- 前端定向单测通过：`2 files / 30 tests passed`
- Rust `build_request_records_layered_observation*` 通过
- Rust `build_context_observation_*` 通过
- `session_regression` 通过：`64 passed`
- `npm run build` 通过
- 构建仅保留 `@vueuse/core` 现存 Rollup 注释 warning，不构成交付阻塞

## 结论

- `PA-025` 已达到可交付状态并完成收口。
- 本轮已按合理粒度完成拆分与并发推进：
  - 后端实现面
  - 前端实现面
  - 主线集成与验证面
- 后续若继续推进：
  - retrieval observability 产品化转入 `PA-024`
  - provider cache hit / miss 指标另行建卡，不回滚 `PA-025` 范围
