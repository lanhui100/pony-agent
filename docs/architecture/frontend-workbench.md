# 前端工作台架构

## 目标

Pony Agent 第一阶段前端不是产品官网，也不是普通聊天页，而是用于承接 Rust 智能体核心的调试工作台。

## 页面架构

`App.vue` 作为根容器，通过 `currentPage` 管理多页切换（无 vue-router，用条件渲染实现）：

- `home`：主工作台（三栏布局）
- `providers`：Provider 配置页
- `model-monitor`：模型监控页
- `settings`：设置面板

## 三栏布局（home 页面）

### 左侧导航栏 `HomeSessionSidebar.vue`

固定结构（冻结表面，未被任务明确邀请不允许修改）：

1. 顶部品牌入口：`Pony Agent`（点击返回对话主页）
2. 主操作：`新对话`
3. 折叠菜单：`对话历史`（分页加载，每页 5 条）
4. 折叠菜单：`模型管理`
   - `模型配置`（切换到 providers 页面）
   - `模型监控`（切换到 model-monitor 页面）

### 中间对话工作区 `HomeWorkspace.vue`

核心对话主舞台，职责：

- 消息流展示（按 `TurnBucket` 分组：user + assistant + tools）
- 用户输入区（composer），支持多行输入、提交
- 流式消息实时展示（通过 `useStreamingPresentationState`）
- checkpoint/history 控制（checkout、restore、fork）
- tool call 交互展示
- 超时重试 pending 气泡

### 右侧可观测性面板 `HomeSidebar.vue`

可观测性区域，承载运行态信息：

- **Status**：当前运行阶段、provider、session 状态
- **Tools**：工具调用记录与观察
- **Trace**：Turn 执行轨迹、phase 变化、timeline
- **Retrieval**：本轮取用的上下文事实（history / attachment / memory）
- **Diagnostics**：诊断信息（调试面板）

右侧面板可通过 `rightSidebarOpen` 切换开闭，宽度可拖拽调整。

## 独立页面

### `ProviderConfigPage.vue`

Provider/Model 配置管理：
- 增删改 provider、model
- 能力声明（capabilities）
- API Key 管理（通过 SecretStore）
- 模型选择与策略配置

### `ModelMonitorPage.vue`

模型监控与 telemetry 聚合：
- 全量监控摘要（model_monitor_summary）
- 会话级 drilldown（model_monitor_session_drilldown）
- 工具调用统计、trace timeline、cache 命中
- 能力来源与能力注册表查看

### `SettingsPanel.vue`

应用设置面板。

## 组件清单

### 核心业务组件

| 组件 | 职责 |
|------|------|
| `HomeWorkspace.vue` | 对话主工作区 |
| `HomeSessionSidebar.vue` | 左侧导航与会话列表 |
| `HomeSidebar.vue` | 右侧可观测性面板 |
| `ProviderConfigPage.vue` | Provider/Model 配置 |
| `ModelMonitorPage.vue` | 模型监控与 telemetry |
| `SettingsPanel.vue` | 应用设置 |
| `AttachmentCenterPanel.vue` | 附件中心（查询/清理） |
| `MarkdownRenderer.vue` | Markdown 渲染 |
| `DebugPanel.vue` | 调试面板（事件日志） |
| `StrategyPanel.vue` | 策略面板（学习指引） |
| `CollapsiblePanel.vue` | 可折叠面板容器 |
| `InfoTip.vue` | 信息提示组件 |
| `PonyBrandIcon.vue` | 品牌图标组件 |

### UI 基础组件（shadcn-vue）

| 组件 | 用途 |
|------|------|
| `ui/Badge.vue` | 徽标 |
| `ui/Button.vue` | 按钮 |
| `ui/Card.vue` | 卡片容器 |
| `ui/ConfirmPopover.vue` | 确认弹层 |
| `ui/Input.vue` | 输入框 |
| `ui/ScrollArea.vue` | 滚动区域 |
| `ui/ScrollBar.vue` | 滚动条 |
| `ui/Separator.vue` | 分隔线 |
| `ui/Switch.vue` | 开关 |
| `ui/Tooltip.vue` | 工具提示 |

## 状态管理（Pinia Stores）

### `stores/runtime.ts`

核心运行时状态，承接：

- 当前会话（sessionId, messages, turnTraceHistory）
- 运行阶段（phase, isSubmitting, isStreaming）
- 流式消息状态与 presentation state
- checkpoint/history 控制（cursor, branch, node）
- 工具调用记录（toolActivities）
- trace 与 buildContext 观测
- 前端飞行记录仪（flight recorder）
- 会话列表管理

### `stores/providers.ts`

Provider 配置管理：

- provider/model CRUD
- 能力目录（capability catalog）
- 模型选择与策略
- 密钥存储接口

### `stores/settings.ts`

应用设置管理。

## 关键 lib

| 文件 | 职责 |
|------|------|
| `lib/tauri.ts` | Tauri invoke/event 桥接封装 |
| `lib/error-utils.ts` | 错误提取工具 |
| `lib/markdown.ts` | Markdown 渲染工具 |
| `lib/flight-recorder.ts` | 前端飞行记录仪 |
| `lib/useStreamingPresentationState.ts` | 流式展示状态管理 |
| `lib/useTimelineAutoScroll.ts` | 时间线自动滚动 |
| `lib/utils.ts` | 通用工具函数 |

## 当前 UI 风格约束

- 极简
- 中文优先
- 微圆角（4px - 8px）
- 布局紧凑
- 黑白灰中性色为主，暖橙色为强调色
- 层次依赖背景明度、字重、字号和留白，不依赖明显边框和重阴影

## 最佳实践

- 用组件边界映射领域边界
- 用 store 管理领域状态
- 前后端通信统一通过 Tauri command / event 边界
- 全局状态进入 Pinia，本地展示状态留在组件内部
- UI 先服务于调试与观察，不急于做复杂产品包装
