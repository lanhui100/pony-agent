# Design

## Decision Summary

1. **输入事件路径审计**：`WorkspaceComposer` / `HomeWorkspace` 的 keydown/click/composition 处理确认同步无阻塞；补 `isComposing` 守卫（修复中文候选确认误发送的既有 bug）。
2. **trace 渲染调度降级**：展开态 trace 更新渲染用 `runLowPriorityTurnWork`（复用 utils.ts:137，rIC + timeout fallback）调度，合并高频更新（pending 标志 latest-wins）。
3. **模板只读 `presentedProjection`**：不订阅最新源快照，idle 调度才能真正延后渲染（依赖 PA-086 投影层）。
4. **draft 写入保持同步**：不延迟；确认持久化时机不被渲染延迟影响。

## Chosen Direction

### 1. 输入事件路径审计 + IME 守卫（HomeWorkspace.vue / WorkspaceComposer.vue）

```ts
// 现状：handleComposerKeydown 对 Enter 无 isComposing 守卫（HomeWorkspace.vue:1279-1298）
// 改为：
function handleComposerKeydown(event: KeyboardEvent) {
  if (event.isComposing || event.keyCode === 229) {
    return; // IME 组合中，不触发提交
  }
  // ...现有逻辑
}
```

- 审计 keydown/click/composition 处理：确认全部同步、无 `await` 阻塞、无 rAF 依赖。
- 若发现任何异步延迟输入处理，改为同步。
- 输入事件监听用原生 `@keydown`/`@click`（Vue 模板），不引入额外调度。

### 2. trace 渲染调度降级（HomeTracePanel.vue）

```ts
// 复用既有 runLowPriorityTurnWork（utils.ts:137，rIC + timeout 200ms fallback）
import { runLowPriorityTurnWork } from "@/lib/utils";

// 展开态下，投影更新触发的渲染调度：
let pendingRender = false;
function scheduleTraceRender() {
  if (pendingRender) return;          // 合并：latest-wins
  pendingRender = true;
  runLowPriorityTurnWork(() => {
    pendingRender = false;
    renderFromPresentedProjection();  // 读最新 presentedProjection，一次渲染
  });
}
```

- 模板/computed 只读 `presentedProjection`（shallowRef/markRaw，由 PA-086 投影层发布），不订阅最新源快照。
- 会话切换：同步清空 pending 标志 + 取消未执行任务（runLowPriorityTurnWork 返回 cancel）。
- 组件卸载：onUnmounted 清理。

### 3. draft 路径

- `draftMessage` 是 store 同步 ref，输入即写，无延迟。
- 持久化（persistHistory / 会话切换）读取最新 draft——确认不被渲染调度延迟（同步执行）。

### 4. 测试

- 顺序代理断言：draft 同步更新（输入后立即断言）+ 渲染延后（fake timers 下断言渲染未立即执行）+ 最终一致（advance timers 后断言最新数据渲染）。
- IME 序列：compositionstart → input → Enter（isComposing=true 不提交）→ compositionend。
- rIC fallback：jsdom 无 rIC，走 setTimeout 分支（runLowPriorityTurnWork 已有）。
- 调度合并：多 revision 一次渲染（pending 标志）。
- 卸载清理：onUnmounted 后无泄漏任务。

## Edge Cases

- `requestIdleCallback` 长时间不触发（主线程持续忙）：`timeout: 200` 保证最终执行（不饿死）。
- 展开面板 + 流式高频更新：pending 标志合并，一次渲染。
- 会话切换 + 调度中：清空 pending + 取消任务，回调读最新投影（新会话），无陈旧渲染。
- 组件卸载时调度回调：cancel 清理。
- IME 快速连续输入：composition 期间不触发提交；compositionend 后正常。
- 点击停止按钮与 idle 回调并发：停止是同步操作，不受调度影响。