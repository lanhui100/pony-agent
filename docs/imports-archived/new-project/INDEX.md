# 文档索引

## 文档说明

本目录用于承接 Pony Agent 的学习、决策和工程说明，避免知识只停留在聊天记录里。

## 结构

- `docs/decisions/`：记录关键架构与实现决策，包括背景、约束、权衡和结论
- `docs/learning/`：记录学习性结论、概念梳理、踩坑过程和可复用素材
- `docs/*.md`：补充设计原则、工程说明与专题文档

## 当前文档

### 决策记录

- [0001-前端工作台先采用原生 TypeScript + Vite 壳层](C:/Users/HUAWEI/Documents/New%20project/docs/decisions/0001-frontend-shell.md)
- [0002-第一阶段视觉方向采用方案 A](C:/Users/HUAWEI/Documents/New%20project/docs/decisions/0002-visual-direction.md)
- [0003-API Key 存储先采用环境变量，后续再演进到安全凭证层](C:/Users/HUAWEI/Documents/New%20project/docs/decisions/0003-api-key-evolution.md)

### 设计基础

- [第一阶段视觉基础规范](C:/Users/HUAWEI/Documents/New%20project/docs/design-foundations.md)
- [上下文压缩设计原则](C:/Users/HUAWEI/Documents/New%20project/docs/context-compaction-principles.md)

### 学习记录

- [0001-空白页排查与当前 UI 方向](C:/Users/HUAWEI/Documents/New%20project/docs/learning/0001-blank-screen-and-ui.md)
- [0002-run_turn 与 Claude queryLoop 的关系](C:/Users/HUAWEI/Documents/New%20project/docs/learning/0002-run-turn-and-claude-query-loop.md)
- [0003-上下文压缩与 Cache 命中](C:/Users/HUAWEI/Documents/New%20project/docs/learning/0003-compaction-and-cache.md)
- [0004-provider 配置、env 策略与模型编辑](C:/Users/HUAWEI/Documents/New%20project/docs/learning/0004-provider-config-and-env.md)

### 任务系统

- [任务系统入口](C:/Users/HUAWEI/Documents/New%20project/task-system/README.md)
- [项目总览](C:/Users/HUAWEI/Documents/New%20project/task-system/00_DASHBOARD.md)
- [任务看板](C:/Users/HUAWEI/Documents/New%20project/task-system/01_TASK_BOARD.md)
