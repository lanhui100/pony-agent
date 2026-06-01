# PA-025 收口 Build Context 与 cache-friendly prompt 边界

## 状态
- Status: `Done`
- Priority: `P2`
- Owner: `Codex`

## 目标
在 `PA-018` 已建立统一 retrieval boundary 的基础上，把 `RetrievedContextState -> prompt/request` 的映射边界单独收口，明确：
- 哪些 retrieval 字段属于稳定前缀
- 哪些字段属于半稳定上下文
- 哪些字段必须留在易变输入层
- `Build Context` 作为“本轮实际送给模型的请求观测”应暴露什么

本卡负责处理 prompt 组装稳定性、cache-friendly request 组织以及 `Build Context` 的可解释性，不继续把这些问题留在 `PA-018` 的 retrieval boundary 本体里，也不承接 `PA-024` 的 observability / telemetry 聚合。

## 输出
- `RetrievedContextState -> prompt/request` 第一版三层映射规则
- `BuildContextObservation` 的正式语义说明：它观测的是“本轮实际发给模型的请求”，不是“当前统一上下文状态”
- prompt 稳定前缀 / 半稳定上下文 / 易变输入 的最小边界
- cache-friendly prompt 组装的第一版工程约束
- 对应前端与 Rust 回归测试

## 验收标准
- retrieval 与 build-context 的语义边界清晰分离：
  - retrieval 表达当前统一上下文状态
  - build context 表达本轮最终 request
- `BuildContextObservation` 至少稳定表达：
  - request format
  - message count
  - image count
  - tool count
  - request message preview
  - tool definition preview
- prompt 组装层明确区分稳定前缀、半稳定上下文和易变输入
- 高波动字段不会默认混入稳定前缀
- 本卡不扩展到 retrieval 监控产品化，也不负责 provider 侧长期 telemetry 聚合

## 当前进展
- 已将 `RetrievedContextState -> prompt/request` 收口为三层观测：
  - `stablePrefixText`
  - `semiStableContextText`
  - `volatileInputText`
- `DefaultTurnContextBuilder.build_request()` 已把上述三层写入 `ProviderRequestObservation`
- `BuildContextObservation` 已正式表达“本轮实际发给模型的请求观测”，并与 retrieval state 分离
- `HomeSidebar` 已在 trace 面板中区分展示：
  - `Current retrieval state`
  - `Sent build context`
- 前端与 Rust 回归测试已补齐，覆盖：
  - 普通 request 的三层观测
  - provider-native transcript 的三层观测
  - provider-native transcript 被截断时，observation 仍保留真实发送过的 truncation note
  - `turn:started` / `turn:completed` 事件中的 build-context clone 语义
  - 右侧 trace 面板中 retrieval 与 build context 的分离展示
  - `buildContextObservation` 的 session 持久化与回读

## 并发拆分
- 主智能体负责主线集成、收口判断与最终验证
- 子智能体 `Hegel` 负责后端实现面勘察与补位
- 子智能体 `Bohr` 负责前端展示面勘察与补位
- 实际拆分为三路：
  - 后端三层 observation 收口
  - 前端 trace 展示与类型补齐
  - 主线集成验证与任务系统同步

## 验证

```powershell
npm run test:unit -- tests/HomeSidebar.spec.ts tests/runtime-store.spec.ts
cargo test --manifest-path src-tauri/Cargo.toml build_request_records_layered_observation -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml build_context_observation_ -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --test session_regression -- --nocapture
npm run build
```

结果：
- 前端单测通过：`30 passed`
- Rust `build_request_records_layered_observation*` 通过
- Rust `build_context_observation_*` 通过
- `session_regression` 通过：`64 passed`
- `npm run build` 通过；仅保留 `@vueuse/core` 现存 Rollup 注释 warning

## 结论
- `PA-025` 已达到可交付状态。
- 本卡只收口 build-context request 语义与 cache-friendly prompt 边界，不扩展到 `PA-024` 的 retrieval observability / telemetry 聚合。
- 若后续继续推进观测产品化，应转入 `PA-024`；若要补 provider cache hit / miss 指标，应另行建立后续任务，不再回滚本卡边界。

## 断点续跑提示
如果后续需要复核本卡完成态，优先查看：
- `src-tauri/src/agent/context.rs`
- `src-tauri/src/agent/provider.rs`
- `src/components/HomeSidebar.vue`
- `src/types/runtime.ts`
- `tests/HomeSidebar.spec.ts`
- `tests/runtime-store.spec.ts`
- `src-tauri/tests/session_regression.rs`
- `docs/learning/0015-prompt-caching-as-runtime-design-constraint.md`
