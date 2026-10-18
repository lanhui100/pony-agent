# wave-37 元复盘增量（Meta-Retrospective Delta）

## 1. 通信拓扑与信噪比
- 四角色（protocol/test/exec-a/exec-b）并行推进，板面 revision 为唯一唤醒信号，wait_agent 长超时轮询 6 次收齐全部回执，无空转；
- 一次越界（test-agent 提前改 src 基准区）由 Lead 复核 diff 时发现，定向指令还原，来回仅 2 条消息，未扩散。

## 2. 门禁穿透与误杀率
- **拦截 1 起**：protocol 的 NFR 基线缺 `trace_id` 字段，dev-nfr-lint 机械拦截（EXIT=1），Lead 补字段复跑 EXIT=0——脚本门禁有效，且暴露 protocol 未自跑 lint 的流程缺口（已在验收消息中要求下波自跑）。
- **红相真实性**：test-agent 如实记录红→绿时间线（首跑 3 failed），未伪造；exec-a 曾声称 9/9 全绿但其观测时点 T4 已并行落地，收据时序歧义由 Lead 亲自复跑澄清——并行波次下"收据时点"应强制写明 git 快照。
- **误杀 0**；**穿透 0**（wave36_adversarial 8/8 无回归证明隔离有效）。

## 3. 分工契约与隔离有效性
- 写域正交成立：tools.rs（exec-a）与 subagent_runtime/orchestration（exec-b）零冲突；test-agent 对 src 只读。
- **缺陷 1 起**：exec-a 的 write_file 观察门插入位置吞掉了 `overwrite=false` → `file_exists` 既有契约，lib 回归暴露后 Lead 在 barrier 中修补（6 行）——单写者跨既有契约的回归门禁（lib 全量而非新增测试）应在 executor 自检清单中强制。
- wave-36 在飞未提交变更与 wave-37 共享工作区，stash 定向隔离 + 归因对照（stash 前后各跑一次）成功判定 lib 唯一红为 wave-36 PA-141 红相——共享工作区的波次隔离模式可行，但 stash push/pop 与 commit 的时序窗口有竞态风险，建议下波改用 `git worktree`。

## 4. 元协议迭代建议（对 dev-team skill）
1. **收据时序强制**：物理收据必须记录"基于哪个 commit/工作树快照"，防并行写入导致的时点歧义。
2. **lint 自跑前置**：protocol agent 落盘 NFR/契约产物后必须自跑对应 dev-*-lint.py，不得依赖 Lead 复核兜底。
3. **Executor 自检含全量 lib 回归**：触碰既有函数行为时，新增测试绿之外必须跑该模块全量 lib 测试，防既有契约被新逻辑吞掉。
4. **共享工作区并行下 wave 间变更需 worktree**：stash 定向隔离在 commit 时序上有窗口竞态，强隔离模式（k≥2）建议默认 worktree 化。
