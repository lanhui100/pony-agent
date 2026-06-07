# 2026-06-05 PA-038 / PA-039 / PA-040 Alignment Review

## 审核方式

- 独立子智能体只读复核 `PA-038 / PA-039 / PA-040` 的任务卡、OpenSpec 文档与架构母文档
- 主线程根据审阅意见判断哪些措辞已经滞后于真实实现，哪些范围词仍然过宽

## 采纳的核心意见

1. `PA-038` 需要把“runtime view 已推进、session control 前端消费仍待继续”明确分开
2. `PA-038` 当前阶段不应把“更正式 persisted trace”写成已收口的硬验收口径
3. `PA-038` 当前范围应优先收敛到 `submission-plan / stop / resume / wait_user`，不再悬空强调 `checkpoint selection`
4. `PA-039` 需要把“session truth-source 已完成”与“checkpoint/runtime-view 更细粒度 trace 投影仍未决”明确分层
5. 架构母文档需要增加状态注记，避免被误读成仍在指导 `PA-031 ~ PA-034` 之后的具体执行顺序

## 已执行的修订

- 更新 `PA-038` 任务卡：
  - 收紧目标与验收标准措辞
  - 把“下一步动作”改为 `session control` 前端消费、persisted trace 升格判断、reload/read-plane closeout
- 更新 `PA-038` OpenSpec：
  - 把 persisted evidence/read-plane 的阶段性完成口径写清
- 更新 `PA-039` 任务卡：
  - 把“进入 persisted trace”改写为“进入 session truth-source，并向 recovery 判定链投影必要 evidence”
  - 显式区分“已完成层”和“未决层”
- 更新架构母文档：
  - 新增 foundation 与 post-foundation 的状态注记
- 启动 `PA-040`：
  - 将任务板与任务卡状态从 `Ready` 推进到 `In Progress`
  - 对应 OpenSpec tasks 把 contract definition 三项标记为已开始落地

## 结论

本轮对齐后，`PA-038 / PA-039 / PA-040` 的任务卡、OpenSpec 与架构母文档已经重新贴近当前真实实现状态，后续可以在不放大验收口径的前提下继续推进 `PA-040` 实现与 `PA-038 / PA-039` 收口。
