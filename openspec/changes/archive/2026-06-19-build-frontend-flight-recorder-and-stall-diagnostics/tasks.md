# Tasks: Build Frontend Flight Recorder And Stall Diagnostics

## 1. Spec And Task-System Alignment

- [x] 1.1 新增 `PA-057` 任务卡，明确前端飞行记录仪与卡顿诊断体系的目标、范围与验收标准
- [x] 1.2 新增 OpenSpec change：`build-frontend-flight-recorder-and-stall-diagnostics`
- [x] 1.3 完成 `proposal / design / tasks / delta spec` 初稿
- [x] 1.4 使用 `opencode / deepseek-v4-flash-free` 进行 3 路并行只读 spec 审核
- [x] 1.5 汇总采纳意见并回灌文档

## 2. Recorder Contract

- [x] 2.1 定义统一 `FrontendTraceEvent` 事件模型
- [x] 2.2 定义 `span / instant / counter / sample / stall / snapshot` 六类事件语义
- [x] 2.3 定义 recorder API、seq 规则、`tsPerfMs` 语义、session/turn 绑定规则
- [x] 2.4 定义 recorder 生命周期：`init / bindSession / bindTurn / flushAndStop`
- [x] 2.5 定义 ring buffer、批量 flush、限流、丢弃策略与截断规则
- [x] 2.6 定义阈值与容量配置项，而不是在实现中硬编码

## 3. Stall Detection

- [x] 3.1 定义 `requestAnimationFrame gap` 检测策略
- [x] 3.2 定义 `setTimeout drift` 检测策略
- [x] 3.3 定义 `PerformanceObserver(longtask)` 的可选增强策略
- [x] 3.4 定义 stall 级别、阈值与强制 flush 行为

## 4. Freeze Snapshot

- [x] 4.1 定义 stall 触发后的轻量现场快照字段
- [x] 4.2 明确哪些大对象或大文本禁止进入 snapshot
- [x] 4.3 定义 snapshot 去重、限流与截断规则

## 5. Tauri Persistence And Query

- [x] 5.1 定义前端诊断事件的 SQLite 落表结构
- [x] 5.2 定义 stall snapshot 的 SQLite 落表结构
- [x] 5.3 定义索引、WAL、busy timeout 与单 writer 连接策略
- [x] 5.4 定义批量写入命令、失败降级与 dropped-count 自观测
- [x] 5.5 定义按 `sessionId / turnId / 时间窗口` 的查询命令与分页边界
- [x] 5.6 定义 JSON 导出能力
- [x] 5.7 定义 Chrome Trace/Perfetto 导出能力与时间精度映射
- [x] 5.8 明确 retention/清理策略

## 6. Critical Path Coverage

- [x] 6.1 定义 `turn completed` 主链路的首批必须埋点
- [x] 6.2 定义 `HomeWorkspace` 展示与滚动链路的首批必须埋点
- [x] 6.3 定义 `MarkdownRenderer` 渲染链路的首批必须埋点
- [x] 6.4 定义消息规模、trace 规模、DOM 规模等 sample/counter 字段

## 7. Verification

- [x] 7.1 为 recorder / stall detector / flush 策略补单元测试或定向测试计划
- [x] 7.2 为 Tauri 侧查询/导出能力补验证计划
- [x] 7.3 为人工制造 stall 与自动化 stall 注入补 smoke 方案
- [x] 7.4 为 longtask availability、flush duration、dropped count 补自观测验证项
- [x] 7.5 设计最小运维消费路径，例如导出入口或 debug overlay
- [x] 7.6 更新任务系统状态与审核记录

## 8. Delivery Phasing

- [x] 8.1 将 Phase 1 收口到 core recorder pipeline + JSON 查询/导出
- [x] 8.2 将 Chrome Trace 导出与 replay-ready 验证收口到 Phase 1.5
- [x] 8.3 将关键链路埋点与最小诊断入口收口到 Phase 2
