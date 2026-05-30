# 2026-05-30 Session 50 - PA-018 / PA-024 / PA-025 边界拆分

## 背景
- 用户明确提出：
  - retrieval 的观测问题后续放到 `PA-024`
  - `Build Context` 的优化任务不再继续留在 `PA-018`
- 当前 `PA-018` 已经承载 retrieval boundary、runtime 接入、前端 retrieval 消费、cache-friendly 收口与 observability 边界讨论，范围开始过宽。

## 本轮决策
- `PA-018` 继续只聚焦：
  - retrieval boundary 本体
  - runtime / graph 对 retrieval 的消费链路
  - `LongTermMemory` 的稳定事实来源
- retrieval 的观测语义、监控面和 `Trace` 中 retrieval 的展示意义，后续转交 `PA-024`
- `RetrievedContextState -> prompt/request` 映射、`Build Context` 语义与 cache-friendly prompt 收口，后续转交新建任务 `PA-025`

## 本轮修改
- 更新 `PA-018` 任务卡，去掉继续吸收 retrieval 观测和 `Build Context` 优化的趋势，明确后续转交边界
- 更新 `PA-024` 任务卡，使其正式承接 retrieval 可观测性语义与监控面
- 新建 `PA-025` 任务卡，单独承接 `Build Context` 与 cache-friendly prompt 边界
- 更新 `01_TASK_BOARD.md` 与 `00_DASHBOARD.md`，同步新的任务分工与主线说明

## 结果
- retrieval / observability / build-context 三类问题不再都挤在 `PA-018`
- 后续讨论与实现可以更清楚地区分：
  - retrieval boundary 本体
  - retrieval 观测面
  - build-context request 语义与 prompt 组装

## 下一步
1. 继续按新的边界推进 `PA-018`，不要再把 retrieval 监控和 `Build Context` UI 问题混入本卡
2. 后续若继续讨论 retrieval 观测，直接落到 `PA-024`
3. 后续若继续讨论 prompt/request 组织、`Build Context` 解释力与 cache-friendly 收口，直接落到 `PA-025`
