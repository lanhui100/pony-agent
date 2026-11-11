# 2026-05-29 Session 48 PA-018 Retrieval UI Boundary

## 本轮目标

- 继续推进 `PA-018`
- 结合当前 retrieval UI 的产品反馈，明确首页工作台的分栏职责
- 防止 retrieval / context / memory 相关信息继续在左栏、中栏、右栏之间漂移

## 本轮结论

- 左侧边栏应只承担菜单、页面切换、会话操作、新建对话、对话历史
- 中间区域应只承担对话历史、输入区和当前消息主舞台
- 右侧边栏应统一承担 `status / trace / tool / retrieval / diagnostics` 等可观测性内容

## 对当前 retrieval UI 的判断

当前前端里有三处 retrieval 相关表达：

1. 左侧边栏“当前上下文”
2. 中间对话区顶部 `Retrieved Context`
3. 右侧状态面板中的 `Retrieval`

产品边界上，这三处不应长期并存为分散表达。

## 收口方向

- 左侧“当前上下文”卡片后续应移出左栏
- 中间顶部 `Retrieved Context` strip 后续应移出中栏
- retrieval 相关表达后续统一收口到右栏

推荐优先形态：

1. 先整合进右栏 trace 面板中的紧凑 retrieval 分区
2. 若信息量和交互独立性后续继续增长，再拆成右栏独立 retrieval 面板

## 语义边界补充

- 状态面板负责表达“本次对话整体状态”
- retrieval 面板负责表达“系统这次实际取用了哪些上下文事实”

不能再把 retrieval 细节混成状态面板的一部分，也不应把 retrieval 当成左栏导航内容。

## 文档回写

- 新增：
  - `docs/guides/frontend-layout-and-observability-boundary.md`

## 当前状态

- 这是 `PA-018` 的产品边界收口记录，不代表 `PA-018` 已完成
- `PA-018` 继续保持 `In Progress`

## 下一步

1. 在不破坏现有稳定交互的前提下，重构三处 retrieval UI 的布局归位
2. 优先尝试把 retrieval 归并进右栏 trace 体系
3. 如果右栏信息层级仍然拥挤，再评估拆分独立 retrieval 面板
