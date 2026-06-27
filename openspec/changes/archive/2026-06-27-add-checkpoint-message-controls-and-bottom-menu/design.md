# Design: Checkpoint Message Controls And Bottom Menu

## 背景

当前实现里，checkpoint / history-control 的主交互主要集中在 [HomeSessionSidebar.vue](C:/Users/HUAWEI/Documents/pony-agent/src/components/HomeSessionSidebar.vue)：

- history graph 状态、history action evidence、current context、restore / fork / branch switch 入口都在侧边栏
- [HomeWorkspace.vue](C:/Users/HUAWEI/Documents/pony-agent/src/components/HomeWorkspace.vue) 负责消息流与 composer，但没有 checkpoint 级回退 affordance
- 用户阅读一条旧 agent 回复时，无法直接就地回到这条回复对应的 checkpoint

这导致两个问题：

1. 控制入口离消息语境过远，用户要在“阅读对话”和“操作历史图”之间频繁切换。
2. 现有 session control surface 更像内部审计面，而不是主对话流里的自然操作层。

## 设计目标

1. 把最常用的 checkpoint 回退动作拉回主消息流
2. 让“从哪个 checkpoint 继续”在 composer 附近有统一入口
3. 让 fork 历史变得可发现，但不把主界面重新做成复杂分支图
4. 保持既有 truth-source / audit-summary / history-state boundary 不变
5. 避免前端通过消息顺序、数组下标或视觉位置去猜测 checkpoint 归属

## 非目标

- 不新增新的恢复模式或 rollback 语义
- 不把 fork 摘要弹层扩展成完整 branch explorer
- 不要求在本卡里重做整个 sidebar；只要求它不再承担 checkpoint 主入口职责
- 不要求在 v1 中展示文件 diff、workspace snapshot 细节或多分支比较

## 当前实现约束

### 1. 消息流没有 checkpoint 锚点

`HomeWorkspace.vue` 当前以 `turnId` 组织 `TurnBucket`，但 [src/types/runtime.ts](C:/Users/HUAWEI/Documents/pony-agent/src/types/runtime.ts) 里的 `HistoryNode` 尚未暴露 `turnId`。这意味着：

- 前端不能安全地通过 `messages[]` 顺序去猜“哪条 agent 消息对应哪个 checkpoint”
- 消息级 checkpoint affordance 需要显式 mapping，而不是隐式推断

### 2. history-control truth-source 已经稳定

现有类型和 store 已经提供：

- `HistoryNode`
- `HistoryBranch`
- `HistoryCheckoutResult / HistoryRestoreResult / HistoryForkResult / HistoryBranchSwitchResult`
- `HistoryStateAuditSummary`

因此本次更适合新增“前端消费视图”和“稳定映射”，而不是重新定义控制命令。

### 3. composer 底部已具备扩展空间

`HomeWorkspace.vue` 当前 composer 底部 bar 已经承载 provider/model、reasoning 和主发送按钮，适合补一个轻量 checkpoint picker trigger，而不是再新增一整块大卡片。

### 4. runtime state schema 需要前置补齐

当前 `HistoryNode` 虽然提供 `nodeId / branchId / workspaceRef / summary`，但不包含消息级 affordance 必需的稳定映射字段：

- `turnId`
- 明确的 workspace rollback 能力位，或可无歧义投影到按钮能力判断的等价字段

因此本 change 的实现前提不是“纯 UI 改稿”，而是先把 runtime state schema 补到足以支撑消息级 checkpoint surface。

## 方案概览

本次改动拆成三个前端 surface：

1. 消息级 checkpoint affordance
2. 底部 checkpoint picker
3. fork 摘要弹层

三者共用同一套 checkpoint read model。

## 1. 消息级 Checkpoint Affordance

### 展示规则

- 仅对“有稳定 checkpoint 映射的 agent 消息”展示
- 最新一条 agent 消息不展示
- user 消息和 tool 消息不展示
- affordance 放在该 agent 消息卡片底部，正文之后、卡片边界之内

### 交互动作

每个符合条件的消息底部展示两个纯图标按钮：

1. `transcript_only`
   - 语义：仅回退对话历史
   - hover tooltip：`回到此 checkpoint（仅对话）`
2. `transcript_and_workspace`
   - 语义：回退对话历史并尝试恢复文件改动
   - hover tooltip：
     - 支持时：`回到此 checkpoint（对话 + 文件）`
     - 不支持时：`该 checkpoint 不支持文件回退，将仅恢复对话`

图标点击后仍然派发到既有 `checkoutHistoryNode(nodeId, mode)` 边界，不新增新命令。

### 状态反馈

- 点击后若成功，沿用既有 history-control feedback 语义
- 若降级为 transcript-only，必须继续明确展示“仅恢复对话，未恢复工作区”
- 若当前处于 `isSubmitting` 或 `sessionOperation`，按钮 disabled，并给出 tooltip reason

## 2. Fork 摘要图标与弹层

### 展示规则

- 当某个 checkpoint 对应节点已经产生 fork 对话轨迹时，消息底部额外展示 fork 图标
- 没有 fork 时不展示该图标

“已产生 fork” 的判断以既有 `HistoryBranch.forkedFromNodeId` / `HistoryNode.nodeId` 为准，不允许前端自行猜测。

### 弹层内容

点击 fork 图标后弹出轻量摘要面板，列出所有从该 checkpoint 分叉出来的候选对话轨迹，每项至少显示：

- fork 标签或分支名
- 该 fork 对应 checkpoint / 节点摘要
- 是否为当前活跃分支

每项都提供“转到该 checkpoint”的菜单动作，派发到既有：

- 直接切到 fork 分支头节点时：`switchHistoryBranch(branchId)`
- 若需精确切到某个分叉节点时：`checkoutHistoryNode(nodeId, mode)`

v1 中弹层只负责“摘要 + 跳转”，不负责完整分支管理。

## 3. 底部 Checkpoint Picker

### 替代关系

现有笨重的 session control / history graph 卡片不再作为 checkpoint 主选择面。新的主入口改为 composer 底部 bar 的一个功能键：

- 位置：输入框底部 bar，与 provider / reasoning / send 同区
- 作用：打开 checkpoint picker 菜单
- 菜单项：每个 checkpoint 的摘要截取
- 点击项：回退到该 checkpoint

这意味着：

- sidebar 仍可保留审计与次级管理信息
- 但 checkpoint 主导航应从 sidebar 下沉到 workspace 底部

### 菜单内容

每个 checkpoint 菜单项至少展示：

- 摘要截取
- 基本上下文：如分支标签、时间、是否当前可见节点
- 恢复模式提示：仅对话 / 对话 + 文件 / 文件回退不支持

默认菜单按“离当前最近的可回退 checkpoint 优先”排序，并且最新消息对应的当前 checkpoint 不作为主回退目标重复展示。

### 快捷键

新增一个全局快捷键打开该菜单，建议：

- Windows / Linux：`Ctrl+K`
- macOS：`Cmd+K`

若该组合与现有全局行为冲突，则可以在实现时退让到 `Ctrl+Shift+K / Cmd+Shift+K`，但必须满足：

- 有显式 spec 记录
- 有可测试行为
- 不抢占文本输入的基础编辑快捷键

## 4. 新的前端 Read Model

为了避免消息级 affordance 靠数组顺序猜测，v1 需要一个显式 read model。建议在 runtime store 新增派生结构，例如：

```ts
type ConversationCheckpointEntry = {
  nodeId: string;
  turnId: string;
  branchId: string;
  summary: string;
  createdAtMs: number;
  isLatest: boolean;
  workspaceRollbackCapable: boolean;
  availableModes: Array<"transcript_only" | "transcript_and_workspace">;
  forkTargets: Array<{
    branchId: string;
    nodeId: string;
    label: string;
    summary: string;
    isActive: boolean;
  }>;
};
```

关键约束：

- `turnId` 与 `nodeId` 的映射必须显式提供
- 前端 SHALL NOT 通过“第 N 条 assistant 消息对应第 N 个 history node”这类启发式推导
- 若某条消息缺少稳定 mapping，则该消息不显示 checkpoint affordance

### 数据来源优先级

1. 优先读取既有 runtime state / snapshot / runtime view 提供的显式映射
2. 若当前 state 缺少关键字段，本 change 应先扩展现有 runtime state schema，而不是新增独立 checkpoint endpoint
3. store 负责把既有 `HistoryNode[] / HistoryBranch[] / turnId mapping` 归一化为 read model
4. 若映射不完整，前端只能降级隐藏按钮，不能猜测性展示

## 5. 视觉与交互原则

- 图标按钮采用纯 icon 样式，降低消息卡片噪音
- hover / focus 时出现 tooltip，默认态保持克制
- 底部 picker trigger 的视觉风格应复用 composer bar 现有暖色、圆角、低对比语言
- fork 弹层不使用沉重抽屉；优先轻量 popover / context menu / dialog
- keyboard focus 必须可见，保证无鼠标也能完成 checkpoint 选择

## 6. 对现有组件的落点

### HomeWorkspace.vue

- 为 assistant turn 增加 checkpoint action row
- 在 composer 底部 bar 增加 checkpoint picker trigger
- 承担快捷键监听、picker 开关和 fork 摘要弹层容器

### HomeSessionSidebar.vue

- 保留次级审计信息与 branch 管理能力
- 不再作为 checkpoint 主入口
- 现有大块 session control / history graph card 应弱化为 explainability surface，而非首要操作层

迁移清单：

- 保留：
  - history graph 相关 explainability 信息
  - branch switch / fork 等分支管理动作
  - `HistoryStateAuditSummary` 和 current context 展示
- 降级：
  - 原有“从侧边栏选择 checkpoint”能力若保留，只能作为次级入口，不得继续是主 CTA
- 移除或弱化：
  - 任何与 workspace 底部 checkpoint picker 重复竞争的主回退入口视觉

### runtime store

- 新增 conversation-checkpoint 派生 read model
- 统一消息级 affordance、底部 picker、fork 弹层的读面

## 7. 验证策略

至少补以下验收场景：

- 非最新 assistant 消息显示两个 checkpoint 图标
- 最新 assistant 消息不显示 checkpoint 图标
- 没有稳定 checkpoint 映射的消息不显示图标
- transcript-only / transcript+workspace tooltip 文案正确
- 文件回退不支持时展示降级 tooltip
- 存在 fork 时出现 fork 图标；无 fork 时不出现
- 点击 fork 图标后能看到摘要列表与跳转动作
- 点击 fork 项后弹层会关闭，并派发到既有 history-control action
- composer 底部 trigger 能打开 checkpoint picker
- 快捷键能打开 checkpoint picker
- 选择 picker 项会派发到既有 history-control store action
- 运行中 / sessionOperation 中 disabled reason 正确
- truth-source non-regression：UI 只消费 read model，不新增私有仲裁

## 8. 风险与收敛

### 风险 1：缺少稳定 turnId <-> nodeId mapping

这是本次最大的实现风险。若没有显式映射，消息级 affordance 很容易出现“按钮挂错消息”的隐性 bug。

收敛要求：

- 将“显式映射”写入 spec
- 若后端当前未提供，必须先补 read-plane 合同或 store 归一化层

### 风险 2：底部 bar 过度拥挤与快捷键冲突

composer bar 目前已有 provider / reasoning / send，新增 picker 后可能显得拥挤；同时 `Ctrl+K / Cmd+K` 也是常见命令面板快捷键，存在冲突风险。

收敛要求：

- checkpoint trigger 必须采用低占宽设计
- 在窄屏下允许折叠成单 icon + tooltip
- 实现前必须审计现有全局快捷键占用
- 最终快捷键必须在实现和测试中显式固定，而不是运行时临时猜测

### 风险 3：fork 弹层过度产品化

如果把 fork 摘要做成完整树形浏览器，范围会迅速膨胀。

收敛要求：

- v1 仅交付“摘要列表 + 跳转”
- 不做可视化 branch graph、不做 diff
