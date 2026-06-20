# PA-057 Spec Review

## 审核对象

- [PA-057 任务卡](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-057-build-frontend-flight-recorder-and-stall-diagnostics.md>)
- [proposal.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/build-frontend-flight-recorder-and-stall-diagnostics/proposal.md>)
- [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/build-frontend-flight-recorder-and-stall-diagnostics/design.md>)
- [tasks.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/build-frontend-flight-recorder-and-stall-diagnostics/tasks.md>)
- [spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/build-frontend-flight-recorder-and-stall-diagnostics/specs/frontend-flight-recorder/spec.md>)

## 审核方式

- 使用 `opencode`
- 模型：`opencode/deepseek-v4-flash-free`
- 方式：3 个不同角度并行/补跑的只读 spec 审核，不改文件

## 审核维度

1. 架构与事件模型
2. 持久化、查询与数据治理
3. 落地可行性、验证与运维使用

## 模型结论

- 总体结论：`Conditionally Pass`

## 高优先级问题

1. `spec` 只定义了按 `sessionId + 时间窗口` 查询，落后于设计中的 `sessionId / turnId / 时间窗口` 查询目标。
2. SQLite 落表缺少索引、WAL/连接策略、retention 与分页边界，随着事件累积会反噬诊断查询本身。
3. flush 失败降级路径未定义，存在诊断链路反向阻塞前端主线程的风险。
4. recorder 生命周期、`sessionId` 延迟绑定与 pre-session 事件处理规则缺失，真实实现容易各处散落初始化逻辑。
5. Phase 1 范围偏大，若把 Chrome Trace 导出、retention、关键链路埋点、诊断入口全塞进首期，交付风险过高。

## 中优先级问题

1. `tsPerfMs`、`seq`、Chrome Trace `cat` 映射等语义还不够明确。
2. 缺少 `longtask availability`、`flush duration`、`dropped count` 等 recorder 自观测项。
3. 验证策略偏“人工可观察”，缺少自动化 stall 注入/验收路径。
4. 导出后的消费路径不清晰，若只有底层命令没有最小入口，团队实际使用率会偏低。

## 采纳与调优

本轮采纳以下修改：

1. 在 [proposal.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/build-frontend-flight-recorder-and-stall-diagnostics/proposal.md>) 中补充 recorder 生命周期、可配置阈值、索引/retention/失败降级与分阶段交付范围。
2. 在 [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/build-frontend-flight-recorder-and-stall-diagnostics/design.md>) 中补充 `tsPerfMs`、`seq`、`sessionId` 延迟绑定与 Chrome Trace `cat` 映射规则。
3. 在 [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/build-frontend-flight-recorder-and-stall-diagnostics/design.md>) 中新增 `Recorder Lifecycle` 与 `Recorder Config` 小节。
4. 在 [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/build-frontend-flight-recorder-and-stall-diagnostics/design.md>) 中补充 SQLite 索引、WAL、单 writer、busy timeout 与分页查询边界。
5. 在 [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/build-frontend-flight-recorder-and-stall-diagnostics/design.md>) 中补充 flush 非阻塞降级、FIFO 丢弃、dropped count 与 flush duration 规则。
6. 在 [design.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/build-frontend-flight-recorder-and-stall-diagnostics/design.md>) 中将实施路径拆为 `Phase 1 / Phase 1.5 / Phase 2`。
7. 在 [spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/build-frontend-flight-recorder-and-stall-diagnostics/specs/frontend-flight-recorder/spec.md>) 中补按 `turnId` 查询 requirement、flush 失败不阻塞 requirement 与 persistence scale/retention requirement。
8. 在 [spec.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/build-frontend-flight-recorder-and-stall-diagnostics/specs/frontend-flight-recorder/spec.md>) 中补 recorder lifecycle 与 stall 验证路径 requirement。
9. 在 [tasks.md](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/build-frontend-flight-recorder-and-stall-diagnostics/tasks.md>) 中补连接策略、失败降级、自观测、自动化 stall 验证与交付分阶段任务。
10. 在 [PA-057 任务卡](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-057-build-frontend-flight-recorder-and-stall-diagnostics.md>) 中同步回灌本轮审核结果与下一步动作。

## 未采纳项

- 暂不在本轮引入新的 `error` event kind。
  原因：当前 change 的首要目标是建立可回放的性能/卡顿诊断主链路；逻辑错误可先通过 `instant + data.severity/errorCode` 或后续 change 扩展，避免首轮事件模型膨胀。

- 暂不在本轮强行合并 `counter` 与 `sample`。
  原因：虽然边界存在解释成本，但当前文档已经能表达“规模快照”与“周期采样”的不同意图；先通过判例和埋点规范约束，后续如实际使用证明混淆严重再收敛模型。

- 暂不要求 Phase 1 必须交付完整可视化诊断面板。
  原因：本轮只要求最小可操作消费路径，保留“导出入口或 debug overlay”即可，不把 UI 工具链扩展成首期阻塞项。

## 结果

- 当前版本已从“总体方向正确但实现边界偏粗”收紧为“可直接指导前端 recorder / Tauri persistence 落地”的 spec 包。
- 文档现已覆盖：生命周期、配置、持久化索引、分页查询、retention、flush 降级、分阶段交付与验证路径。
- 下一步可以先跑严格校验，再进入实现阶段。

## 审核证据

- `opencode` session：`PA057 Spec Review Arch V2`
- `opencode` session：`PA057 Spec Review Persistence V2`
- `opencode` session：`PA057 Spec Review Operations V3`
