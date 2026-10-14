# Meta Retrospective: Sentry 上报契约与 ask_user/ask_answer 消费错误治理

## 1. 通信拓扑与信噪比（Topology & Noise）
- **拓扑结构**：采用 Lead 编排与 DAG 任务拆分（Task-1: Sentry 契约与项目维度补全；Task-2: Ask 消费防并发与状态对齐；Task-3: 全量门禁验证）。
- **信噪比与上下文控制**：保持任务切片正交，避免无状态循环与冗余交互，聚焦物理事实与代码闭环。

## 2. 门禁穿透与误杀率（Gate Penetration / False Negatives & Positives）
- **测试拦截效果**：
  1. `telemetry-suppression.spec.ts` 成功拦截非测试环境下的 Sentry Ingest 契约断言，验证 `project: "pony-agent"` 自动注入；
  2. `ask-store.spec.ts` 覆盖 CAS 失败、重复取消、并发应答等 17 项全路径断言；
  3. `runtime-status` / `workspace-runtime-status` 对齐最新状态机规范，全量 786 项测试 Exit Code 0。
- **构建门禁**：`vue-tsc --noEmit`、`vite build`、`cargo check`（src-tauri & pony-agent-core）全部通过。

## 3. 分工契约与隔离有效性（Contract Isolation）
- **写域正交**：前端 telemetry/tauri/ask-store 与后端 client.rs/lib.rs 分离，单向依赖清晰。
- **边界防御**：在 Sentry 上报层统一追加 `tags.project: "pony-agent"`，满足 `pony-sentry` 服务端提取契约，杜绝 `missing_project` 告警。

## 4. 元协议迭代建议（Self-Evolving Protocol）
- **经验沉淀**：控制平面操作（如 CAS 依赖的 Ask / Plan / Approval）需在前端交互层提供第一道乐观排重锁，并在错误处理中将预期内的状态过期（Replay / Expiry）与真正的系统崩溃（Crash / NetworkFailure）进行差异化分流，防止良性交互竞争污染监控系统。
