# PA-167: Headless 形态的「工具级授权与交互应答」缺口（wave-59 撤销项承接）

## 状态
- **状态**: Backlog（已登记，待排期）
- **优先级**: P2
- **Complexity**: C
- **创建日期**: 2026-11-02
- **来源**: PA-164 契约 v3/v4 裁定 —— 两项能力因**宿主无接线口**被撤销，不得作为"已支持"对外宣称。

## 缺口事实（物理证据）
1. **工具级 allow-list 无法落地**：策略求值器由 runtime 内部注册（`governed_executor.rs:463-469`），兜底 `governed_default` → `Allow`（`:281-287`）；宿主无法注入自定义 evaluator。CLI 侧自造 `--allow-tools` 判定无人消费 = 静默谎报。
2. **无审批应答方 ⇒ 只能 fail-closed**：审批/ask 触发时 turn 立即返回并只发 `turn:suspended`（`runtime/turn_stream.rs:376-438`），宿主需 `answer_ask`（`ask_control.rs:47-53`）+ `graph_resume_ask`（`ask_plan_commands.rs:229-260`）才能续跑。无头场景无人应答 ⇒ PA-164 收敛为 exit 6。
3. **`host_control_available` 硬编码 `true`**（`runtime/mod.rs:316`），`TurnInput` 无接缝 ⇒ 无法走 `interaction_unavailable`（`dispatcher.rs:1202-1207`）这条 fail-closed 逃生口。
4. **跳数上限无公开接线口**：`runtime/limits.rs:76 tool_hop_limit_override_registry()` 为 `pub(super)`，故 `--max-turns` 撤销。
5. **suspended 不可细分**：wire 只有固定 `control_outcome_pending`，无法区分"用户主动 ask"与"安全审批"，故 envelope 仅能透传 `failure_reason` 原文。

## 目标
给无头/CI 形态补齐**可编程的授权与交互时序**：工具级白名单真实生效、审批请求可被脚本化应答、suspended 的 reason 可区分、跳数预算可配置。

## 交付契约（草案）
- **C1 策略求值接缝**：`AgentRuntimeBuilder` 暴露可注入 `ToolPolicyEvaluator`；宿主可声明 allow-list / deny-list 且**被真实消费**（验收须证明"未命中的工具确实未执行"）。
- **C2 无头应答协议**：`--approval-responder {deny|auto-allow|script:<cmd>}`，请求经子进程/HTTP 回调应答，**超时必须 fail-closed**（默认 deny），禁止挂起。
- **C3 reason 透传**：`turn:suspended` payload 增加 `control_reason` 字段（`ask_user` / `approval_required` / `plan_approval`），向后兼容新增字段。
- **C4 跳数预算**：公开 `max_tool_hops` 配置入口，映射到 `limits.rs` 的 override registry。
- **C5 `host_control_available` 可注入**：`TurnInput` 增字段，使宿主可声明"无交互能力"从而走 `interaction_unavailable`。

## 非目标
不改变 TUI/Tauri 现有交互体验；不引入远程多租户鉴权。

## 验收标准（物理收据）
1. allow-list 生效性：真实二进制 + mock，断言"白名单外工具**未被调用**"（证据：mock 请求记录为空）。
2. 审批脚本化：`script:` responder 应答后 turn 继续并完成（exit 0），不应答 ⇒ exit 6 且无挂起（有 wall-clock 上界断言）。
3. reason 区分：ask 与 approval 两场景 `control_reason` 可区分。
4. 新增字段向后兼容：老客户端解析不失败（serde 默认值 + 快照测试）。

## 当前进展
已登记，未动工。

## 下一步动作
先做 C1+C5（最小可验证闭环），再做 C2；C3/C4 可并行。需先与 PA-165（ACP）对齐——ACP 的 `session/request_permission` 正是 C2 的协议化形态，避免两套应答机制。

## 当前卡点
C1 需要改 runtime 内部注册逻辑，触及 `governed_executor` 装配边界，须先过方案对抗审查（blast-radius 命中）。

## 断点续跑提示
读本卡 §缺口事实即可恢复上下文；关键证据行号已内联。
