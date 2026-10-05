# 主会话消息排队与插队 (Queue & Steer) 交互与状态契约

## 1. 契约定义与数据模型

### 排队消息条目 (QueuedMessageItem)
```typescript
export type QueueDeliveryMode = "queue" | "steer";

export interface QueuedMessageItem {
  id: string;
  sessionId: string;
  content: string;
  mode: QueueDeliveryMode; // 默认 queue，可由用户提升为 steer
  createdAt: number;
}
```

### 核心行为契约
1. **输入框提交逻辑分流**：
   - 当 `isSubmitting === false`（空闲）：直接触发正常 `submitTurn()`；
   - 当 `isSubmitting === true`（Agent正在思考/工具调用中）：回车提交**不拦截报错**，而是自动包装为 `QueuedMessageItem`，加入当前会话的 `pendingQueuedMessages`，并清空输入框。
2. **气泡位置与视觉设计**：
   - 消息气泡直接位于时间线最底端、输入框最上方。
   - 样式为半透明灰色气泡（`bg-stone-100/90 text-stone-600 border border-stone-200/60`），带有 `⏳ 排队中` 状态标签与序号。
3. **>2 条平滑动效堆叠交互 (Stacked Bubble Animation)**：
   - 当排队消息数 ≤ 2：直接垂直排列展示。
   - 当排队消息数 > 2：
     - **默认收起态**：最早排队的消息在最上方（第 1 条完全展示），后续消息以平滑动效轻微下沉错落堆叠（`translateY` 偏移 + 缩放 `scale(0.98, 0.96...)` + 阴影），呈现立体卡片层叠效果。
     - **Hover 态**：鼠标悬停在堆叠卡片区时，平滑展开全部列表（高度自适应展开，过渡 `cubic-bezier(0.25, 0.46, 0.45, 0.94)`），清晰显示每条内容。
4. **行级操作按钮与图标**：
   - **⚡ 立即插队按钮**：使用与发送按钮完全相同的 **ArrowUp（上箭头）** 图标。点击后将该消息从排队列表中提升为插队（`steer`），优先在下一个 Step 边界被大模型接纳。
     - Tooltip: `"立即插队：在当前回合步骤完成后优先处理"`。
   - **🗑️ 移除按钮**：使用垃圾桶（Trash2）图标。点击后从队列中删除。
     - Tooltip: `"删除排队消息"`。
5. **全局模式切换快捷键**：
   - 设置快捷键 `Alt+S`（或可在输入框内一键切换）：可在“排队模式（Queue）”与“插队模式（Steer）”之间快速切换。发送栏提供轻量状态 pill 指示当前发送模式。

---
## 2. 状态机推进
- 状态：Stage 1 契约冻结完成，进入前端状态与组件落地。
