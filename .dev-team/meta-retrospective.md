# Wave 4 元框架复盘（2026-10-09：PonySentry Agent Trace 上报与生命周期状态机）

> 交付：桌面端 Agent Trace 上报契约、生命周期状态机及 Core 异步非阻塞投递管线全绿接入
> 范围：ADR-0024 制定、NFR 基准落盘、7 个契约/对抗测试通过，原 telemetry 7 个测试零回归；基线 82c86ea → 终态 afa2a79
> 波次检查点：red_anchor=82c86ea, green=afa2a79

## 1. 通信拓扑与信噪比

- 角色分权：Protocol Agent (task-1) 制定 ADR 与 NFR、Test Agent (task-2, task-3, task-4) 编写契约测试、Executor (task-5) 实施 Client 逻辑，Lead 统筹，Roster 严格受控（Roster=4 ≤ 8）。
- 信噪比：交互均基于任务板与结构化指令，无多余过程性讨论；针对敏感词 pre-commit 阻断，精准通过任务拆分（task-3/task-4）在 2 轮内收敛解决。

## 2. 门禁穿透与误杀率

- 红相有效：初始测试因缺少 `capture_agent_trace` 产生真实编译期 Exit Code 101，严禁了假测试与自测放行。
- 敏感词门禁：Node.js 敏感凭据检测拦截了测试用例占位符中的非规范前缀，通过统一对齐 `sk-test-` / `sk-mock-` 白名单规则解除阻断，证明安全扫描门禁具备真实拦截力。
- 绿相有效：Executor 实施后，7 个契约与对抗测试全部通过（Exit Code 0），原有 7 个 telemetry 错误测试亦保持全绿，实现零回归。

## 3. 分工契约与隔离有效性

- 职责边界坚固：Test Agent 仅有测试写入权限，Executor 仅对 `crates/pony-agent-core/src/agent/ponysentry/` 具备写入权限，`tests/` 目录在实施阶段严格保持只读零修改。
- 契约冻结：NFR 商业基线（超时 ≤3000ms、队列有界 ≤1024、零信任脱敏）在 `.dev-team/nfr-baseline.json` 物理锚定并完全通过单测断言。

## 4. 元协议迭代建议（对 dev-team skill 的演进）

1. **测试凭据白名单预检**：在生成涉及鉴权与 Token 的对抗测试用例时，建议 `test-expert` 规范内置常用占位符格式（例如一律以 `sk-test-` 或 `sk-mock-` 开头），减少与静态扫描工具的摩擦耗时。
2. **非阻塞遥测设计模式库**：本次沉淀的 `AgentTracePayload` + `EvalStatus` 6 态状态机可作为后续多客户端（如 Web 版、CLI 版）统一的 Agent 评测轨迹交换格式。

---

# Wave 5 元框架复盘（2026-10-09：Turn 终态 Trace 自动上报接线）

> 交付：桌面端每个 turn 终态（completed/failed/cancelled）自动构造 AgentTracePayload 并异步上报 PonySentry，stream 与 sync 双路径全覆盖
> 基线 13c564d（红相） → eed8ab9（首次绿相） → c2e3358（L3 FAIL 修复收口）

## 1. 通信拓扑与信噪比

- **teammate 连续失效事件**：本波 protocol-agent、executor、test-agent 均出现"运行中但长时间不落盘"或"失败无产出"。Lead 依熔断逻辑两轮触发（换人 + 精简指令 + 提供完整结构定义）仍未恢复，最终按抗震荡原则降级为 **Lead 装配例外**完成（登记 write_scopes、≥2 路 L3 独立审查弥补分权）。**教训**：当多个 teammate 在同会话内连续无产出，应立即评估为环境性失效而非个体问题，尽快降级避免 Token 空耗；本波在此浪费了约 20+ 轮 wait 轮询。
- **Test as Contract 有效**：Test Agent（task-14）成功以红相测试冻结契约（4 用例），成为 Executor 实现的唯一验收基准；这一产物是本波最有价值的协作成果。

## 2. 门禁穿透与误杀率

- **L3 双视角审查捕获致命缺陷（本次门禁最大价值）**：第一路 L3（8f9298c1）给出 **FAIL（3 findings）**——`is_trace_reportable_terminal` 匹配带前缀事件名而运行时 `payload.kind` 是短名 → **生产路径零上报**（冻结测试用带前缀 kind 构造事件掩盖了缺陷，4 测试全绿但功能完全失效）；同步入口经 NoopTurnEventSink 绕过挂载；tool error 字符串 JSON 二次编码。Lead 逐一验证 file:line 反例真实存在后接受 FAIL。
- **修复闭环**：短名+前缀双匹配门控；挂载统一到 `turn_flow::emit_event`（stream/sync 公共底点）并移除 control_plane 冗余挂载防双报；error 取 `Value::String` 裸文本。补 3 个固化用例（短名 kind 判定/短名映射/error 裸文本），复议 L3（0adc537f）PASS。
- **误杀率**：冻结测试本身存在"与运行时形态不符"的盲区（测试用带前缀 kind 掩盖了真实短名），提示 Test-as-Contract 必须附带"真实运行时事件形态"样本校验，否则契约可能冻结错误前提。

## 3. 分工契约与隔离有效性

- 写域隔离有效：测试文件仅 test-agent/Lead 追加，Executor 实现仅改 src/，L3 审查只读；审查 JSON（PASS/FAIL + findings）机器可解析。
- 契约演进：L3 FAIL 驱动的修复属于契约修正（测试与运行时形态对齐），按"测试冻结后由 Lead 一票裁决 + Test Agent 迁移"原则执行，未静默改测试。

## 4. 元协议迭代建议

1. **teammate 失效快速熔断阈值**：连续 2 轮"运行中无落盘产物"即触发降级评估（现行"连败 3 次"适用于测试失败，不适用于无产出型失效）。
2. **Test-as-Contract 必须携带运行时形态样例**：冻结测试应包含一个"从真实代码路径捕获的事件样例"（如运行时短名 kind、真实 session_id 形态），防契约冻结在错误前提上。
3. **L3 双视角审查高价值确认**：独立审查确实捕获了功能测试盲区外的生产失效，纳入 B 级任务的强制门禁。
