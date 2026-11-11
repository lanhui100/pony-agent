# PA-070 统一 Provider Request Retry 与退避边界

## 基本信息
- 编号: PA-070
- 名称: 统一 Provider Request Retry 与退避边界
- 状态: Done
- 优先级: P1
- 创建日期: 2026-06-26
- 更新日期: 2026-06-27

## 目标
- 为 `call model` 建立正式的 core-side request-level retry / phase-level fallback 合同。
- 明确 `pony-agent-core`、`src-tauri`、前端 store 三层各自负责什么，不再混淆 provider request retry 与 whole-turn retry。
- 为指数退避、错误分类、stream 安全边界、预算、telemetry 与 deterministic tests 建立可实施 spec。

## 输出
- 一套 OpenSpec change：`unify-provider-retry-and-backoff-boundary`
- 一份 proposal / design / tasks / delta spec
- 一张清晰的任务卡，能指导后续实现与审核

## 范围
- `crates/pony-agent-core/src/agent/provider.rs`
- `crates/pony-agent-core/src/agent/turn_flow.rs`
- `crates/pony-agent-core/src/agent/runtime/mod.rs`
- `src/stores/runtime.ts`
- `src-tauri/src/tauri_adapter.rs`

## 非目标
- 本卡不直接实现完整 retry 重构代码
- 本卡不重做前端 UI 视觉
- 本卡不把所有 tool retry 与 provider retry 强行收敛成一套总框架
- 本卡不把 turn-level retry 静默迁移为与 request-level retry 相同的机制

## 验收标准
1. 明确 request-level retry、phase-level fallback、turn-level retry 三层语义与预算归属。
2. 明确 provider retry 必须落在 `pony-agent-core`，而 `src-tauri` 不负责退避判定。
3. 明确前端现有 whole-turn 自动重试将退场；后续如需 turn-level retry，只允许显式 control-plane action。
4. 明确 stream 安全边界：何时允许 `stream -> retry`、何时允许 `stream -> sync`、何时必须停止自动重试。
5. 明确 deterministic test 策略与最小可测抽象，避免真实时间 sleep 测试成为主验证路径。
6. 明确与 `turn lifecycle / trace persistence / monitor telemetry / run-control audit` 四类既有合同的边界对齐。

## 当前进展
- 已完成代码勘察，确认 provider timeout retry 现位于 `crates/pony-agent-core/src/agent/provider.rs`。
- 已完成 3 路并行对抗式审核，并采纳了分层、预算、安全边界与测试策略方面的关键意见。
- 已建立正式 OpenSpec change，并已根据 3 路 spec 对抗式审核补入 escalation contract、structured failure、budget/`Retry-After` 与 stream/tool-call 边界。
- 已完成实现、阶段性严格代码审核、最终 2 路严格对抗审核与一轮收尾调优。
- 已同步 canonical spec、归档 OpenSpec change，并更新本地架构文档口径。

## 采纳结论摘要
- provider request retry 应继续放在 `pony-agent-core`
- `src-tauri` 保持桥接角色，不持有 retry policy
- 前端 whole-turn 自动重试退场，不再作为第二层自动恢复机制保留
- stream 不能用“任意 delta”这个过粗信号决定是否还能自动重试，应至少区分 reasoning 与 visible text
- `Retry-After`、budget、fallback source 与 telemetry 必须写入正式合同

## OpenSpec
- 已归档：
  `openspec/changes/archive/2026-06-27-unify-provider-retry-and-backoff-boundary/`
- Canonical Spec:
  `openspec/specs/provider-retry-and-backoff-boundary/spec.md`
- Spec 状态: validated and archived

## 下一步动作
已完成，无后续动作；后续若继续扩展显式 turn-level retry，应以新 change 承接。

## 当前卡点
- 暂无。当前已完成归档与收口。

## Resume Hint
- 下次继续时先读 `management/task-system/03_TASKS/PA-070-unify-provider-retry-and-backoff-boundary.md`。
- 再读 `openspec/specs/provider-retry-and-backoff-boundary/spec.md`，如需继续推进显式 turn-level retry，则以新 change 启动。
