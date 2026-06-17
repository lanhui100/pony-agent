# 2026-06-17 PA-056 Workspace Mode Closeout

## 本次做了什么

- 为 `PA-056` 补齐显式 `Coding / Work` 配置闭环
- 新增独立全局设置模型 `AppSettings`
- 新增 Tauri `load_app_settings / save_app_settings`
- 新增前端 `settings` 类型、store 与 `SettingsPanel`
- 在左侧边栏尾部新增设置入口
- 将 `workspaceMode` 从前端提交透传到 `TurnInput -> TurnContext -> domain profile`
- 将 `domain profile` 选择改成“显式配置优先，启发式兜底”
- 补充最小回归测试，覆盖 settings store、App 页面切换、sidebar 设置入口与 `workspaceMode` 提交链路

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

## 本次验证

- `npm run cargo:check -- -p pony-agent-core --lib`
- `npx vitest run tests/settings.store.spec.ts`
- `npx vitest run tests/App.spec.ts tests/HomeSessionSidebar.spec.ts tests/settings.store.spec.ts`
- `npx vitest run tests/runtime-store.spec.ts -t "forwards workspace mode to start_graph_run_stream input payload"`

结果：

- Rust core 编译通过
- 前端设置 store 回归通过
- App / Sidebar 设置入口与 settings 页面切换回归通过
- `workspaceMode` 提交链路回归通过

## 当前结果

- `Coding / Work` 已不再只是前端设置项
- 当前选择会真实进入上下文构建主路径，并决定 `domain profile prompt`
- 这条配置主链后续可继续承载更多全栈配置项，而不必重复改造 runtime/context 主路径

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
