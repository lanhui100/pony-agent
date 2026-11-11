# 2026-06-17 PA-056 Workspace Mode And Prompt Tightening Closeout

## 本次做了什么

- 为 `PA-056` 补齐显式 `Coding / Work` 配置闭环
- 新增独立全局设置模型 `AppSettings`
- 新增 Tauri `load_app_settings / save_app_settings`
- 新增前端 `settings` 类型、store 与 `SettingsPanel`
- 在左侧边栏尾部新增设置入口
- 将 `workspaceMode` 从前端提交透传到 `TurnInput -> TurnContext -> domain profile`
- 将 `domain profile` 选择改成“显式配置优先，启发式兜底”
- 根据 3 轮 `opencode / deepseek-v4-flash` follow-up 审核再调优一轮代码
- 恢复 `BASE_SYSTEM_PROMPT` 中默认中文回复语义
- 压缩 `BASE_SYSTEM_PROMPT / coding / work` profile 文本体积
- 为小上下文窗口新增 domain profile 跳过逻辑
- 为未知 `workspace_mode` 增加显式 fallback 提示
- 将 turn 主链、graph handoff 与 persistence 的 `workspace_mode` 透传补齐到真实运行路径
- 修复 `runtime.ts` 在测试 teardown 后仍访问 `window` 的异步错误
- 补充回归测试，覆盖 settings、domain profile、小窗口 skip、runtime teardown 稳定性与上下文路径验证

## 关键改动文件

- `crates/pony-agent-core/src/agent/app_settings.rs`
- `crates/pony-agent-core/src/agent/context.rs`
- `crates/pony-agent-core/src/agent/runtime/mod.rs`
- `src-tauri/src/lib.rs`
- `src/stores/settings.ts`
- `src/components/SettingsPanel.vue`
- `src/components/HomeSessionSidebar.vue`
- `src/App.vue`
- `src/stores/runtime.ts`
- `src/types/settings.ts`
- `src/types/runtime.ts`
- `tests/settings.store.spec.ts`
- `tests/App.spec.ts`
- `tests/HomeSessionSidebar.spec.ts`
- `tests/runtime-store.spec.ts`
- `tests/HomeWorkspace.spec.ts`

## 本次验证

- `cargo check -p pony-agent-core --lib`
- `cargo test -p pony-agent-core build_request_ --message-format short -- --nocapture`
- `cargo test -p pony-agent-core retrieve_context_state --message-format short -- --nocapture`
- `cargo run -p pony-agent-core --bin non_tauri_harness`
- `npm exec vitest -- run tests/runtime-store.spec.ts tests/HomeSidebar.spec.ts tests/HomeWorkspace.spec.ts tests/HomeSessionSidebar.spec.ts tests/settings.store.spec.ts`
- `npm run test:tauri:smoke`

结果：

- Rust core 编译通过
- Rust `build_request_` 相关 `12` 个测试通过
- Rust `retrieve_context_state` 相关 `2` 个测试通过
- `non_tauri_harness` 通过
- 前端 `145` 个测试通过
- `window is not defined` 的 unhandled teardown error 已消失
- `tauri smoke` 通过，preview 端口改为动态分配，已避免 `4176` 固定端口占用冲突

## 当前结果

- `Coding / Work` 已不再只是前端设置项
- 当前选择会真实进入上下文构建主路径，并决定 `domain profile prompt`
- `BASE_SYSTEM_PROMPT` 已恢复中文默认语义，不再对现有中文工作流产生静默回归
- 小上下文窗口模型不会再被无条件 domain profile 注入压缩有效历史窗口
- runtime 低优先级异步任务在测试/teardown 环境下不再访问失效的浏览器对象
- 这条配置主链后续可继续承载更多全栈配置项，而不必重复改造 runtime/context 主路径

## 审核记录

- 首轮 code review：
  [2026-06-16-pa056-code-review.md](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-16-pa056-code-review.md>)
- follow-up code review：
  [2026-06-17-pa056-code-review-followup.md](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-17-pa056-code-review-followup.md>)

## 下一步最小动作

1. 若继续扩配置项，优先沿 `AppSettings -> settings store -> submitTurn/runtime pass-through -> context` 这条链新增字段
2. 若继续推进 `PA-056` 深化项，优先做 `instruction_scope_sources` 真 source 枚举与 diff 检测
3. 若继续做缓存命中优化，优先推进 provider continuation / compaction 的真实策略切换

## Resume Hint

继续前先看：

- `management/task-system/03_TASKS/PA-056-redesign-context-assembly-and-cache-strategy.md`
- `openspec/specs/context-assembly-and-cache-strategy/spec.md`
- `crates/pony-agent-core/src/agent/context.rs`
- `crates/pony-agent-core/src/agent/app_settings.rs`
- `src/components/SettingsPanel.vue`
