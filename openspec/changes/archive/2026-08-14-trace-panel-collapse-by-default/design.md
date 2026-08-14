# Design

## Decision Summary

1. `HomeSidebar.vue` 的 `activePanel` 默认值从 `"trace"` 改为 `""`：应用启动即 trace 折叠。
2. `HomeTracePanel.vue`：**toggle header 常驻**，`collapsible-body` 用 `v-if="open"` 懒挂载——折叠时 body 完全卸载（computed/watch/事件监听释放），展开时才挂载。
3. 顺手修复 `liveTraceTurn` 折叠态 stamp 问题（HomeSidebar.vue:123）。

## Chosen Direction

### 1. 默认折叠（HomeSidebar.vue）

```ts
// 现状
const activePanel = ref<"tools" | "trace" | "plan" | "debug" | "">("trace");
// 改为
const activePanel = ref<"tools" | "trace" | "plan" | "debug" | "">("");
```

- 影响：启动后 trace 面板折叠；其余面板（tools/plan/debug）默认仍折叠（现状即 `""` 或各自处理）。
- `HomeStatusPanel` 不受影响（始终渲染，体积小、无 trace 依赖）。

### 2. header 常驻 + body 懒挂载（HomeTracePanel.vue）

```html
<!-- 现状：整个组件常驻，:data-open 控制折叠 -->
<section class="collapsible-shell ..." :data-open="open">
  <button data-testid="trace-panel-toggle" @click="emit('toggle')">...header...</button>
  <div class="collapsible-body">
    ...timeline 内容（turns v-for / timeline 条目 / 详情行）...
  </div>
</section>

<!-- 改为：body 用 v-if 懒挂载 -->
<section class="collapsible-shell ..." :data-open="open">
  <button data-testid="trace-panel-toggle" @click="emit('toggle')">...header...</button>
  <div v-if="open" class="collapsible-body">
    ...timeline 内容...
  </div>
</section>
```

- 折叠时 body 不挂载：`turns` v-for、timeline 投影、详情行全部不渲染，相关 computed 不执行。
- 展开时全新挂载：渲染当前 `turnTraceHistory` / `traceTimeline`（store 数据始终最新，无需本地缓存恢复）。
- `props.open` 在 body 挂载后恒为 `true`，`collapsible-body` 的样式逻辑保留（无害）。

### 3. liveTraceTurn stamp 修复（HomeSidebar.vue）

```ts
// 现状：每次计算 stamp updatedAt: Date.now()，导致 memo 缓存（key=ref+updatedAt）必 miss
// 改为：折叠态（activePanel !== "trace"）不 stamp；展开态保持现状
```

- 折叠时 `liveTraceTurn` 返回 null（不 stamp）；展开时保持现状逻辑。
- 消除流式期间折叠态下每 200ms 的缓存失效与全量重算。

### 4. 测试

- `HomeSidebar.spec.ts`：默认 `activePanel === ""`（trace body 未挂载）；`togglePanel('trace')` 后挂载并渲染。
- 新增：折叠时 store 更新 trace 数据，trace body 无 DOM；展开后渲染最新数据。
- 既有"trace 面板内容渲染"用例改为先展开再断言。

## Edge Cases

- 展开瞬间 trace 数据量巨大（长会话）：首次挂载渲染可能耗时——属 PA-085（虚拟滚动）范围，本卡不处理。
- 折叠期间新 turn 完成：store 的 `turnTraceHistory` 持续累积，展开时渲染全量——正确（数据不丢）。
- 会话切换：`activePanel` 为 `""` 时切换会话无 trace 开销；展开态切换会话仍渲染新会话 trace（与现状一致）。
- 浏览器模式/无 Tauri：无影响（trace 数据源一致）。
- 交互状态重置：body 卸载清空 activeTurnId/detail 展开——与现状折叠行为一致，非回归。