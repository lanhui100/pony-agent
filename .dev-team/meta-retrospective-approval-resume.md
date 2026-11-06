# wave-65 元复盘：审批可恢复（approval-resume）

> 交付：`approval_resume_e2e.rs` T1~T4（冻结）+ R2 修复 + 2 轮对抗审查。本复盘只诊断**协作元框架**，不评价业务代码细节。

## 1. 通信拓扑与信噪比

- 名册 Lead(1) + Protocol(1) + Test(1) + Executor(1)，审查全部走 Subagent 池（4 + 3 路），未挤占常驻槽位，符合容量守恒。
- 探索阶段 1 路 E2 Explorer 一次性产出 Q1~Q6 事实清单（带 path:line），把 Lead 的文件读取量从"十几处"压到"3 处定点复核"——这是本波最大的 token 节省点；同时它纠正了 Lead 的两处错误假设（`take_pending_ask_injection` 实际不存在；Ask 注入无法零改动复用），避免了方案返工。
- 噪声面：Protocol Agent 的上报里带上"三点裁决请求"是有效信噪（a/c 两条直接影响交付边界）；而 R2 修复轮的回报仍沿用 v1 结构，Lead 需要二次核对行号——建议后续固定"变更块 diff 摘要"格式。
- 重复读取率：并行会话持续改写 `bin/tui.rs` 导致行号漂移，Lead 与审查员都出现同文件二次定位；已在 state.json 记录并行会话风险，属可接受成本。

## 2. 门禁穿透与误杀率

- **红相有效**：T1~T3 在空桩阶段确实失败且失败点落在断言（非编译错误），T4 作为 INV-2 回归绿；冻结 sha256 全程未变。
- **第一轮审查穿透力强**：4 路全部 FAIL，抓出 3 类真实缺陷——(a) 无 path 审批把 callId 当路径授权导致永久拒批；(b) 审批 TTL 60s 无宿主对账导致面板活锁；(c) 契约对非路径审批缺 else 结论导致"批准→续轮→再审批"循环。这三类**都不是**红相测试能覆盖的（e2e 只走 core，不走 TUI 接缝），证明"机器门禁 + 对抗审查"双轨是必要的。
- **误杀/过度**：4 路各自把同一 BLOCKING 重复报（信息冗余但无害）；L3-P 与方案审查各贡献 1 条独立发现（TTL、验收假绿），互补性达标。
- **三轮收敛曲线**：R1（4 路）暴露 3 类真实缺陷 → R2 修 4 项后 A/B 转 PASS，但 L3-P/方案又抽出 1 条**更深**的 BLOCKING（`approvalKind` 按实参 path 派生而非按 `decision_source` 成因派生）→ R3 修复后收敛。教训：审查轮不是"数人头"，而是每轮都必须给审查员**新的可证伪面**（R2 新增的单测与投影字段正是 R3 的抓手）。
- **假绿风险已如实登记**：TUI 正向接线无 e2e（bin 内函数不可被集成测试导入），已写入契约 C5 与 backlog F-2，未用"新增单测"冒充 e2e。

## 3. 分工契约与隔离有效性

- **写域隔离**：Protocol/Test/Executor 三者写域按阶段串行交接（stub → test → impl），未发生交叉覆盖；Lead 只做契约、裁决、git 与门禁，符合 Lead Write Prohibition。
- **最大隔离漏洞是工作区而非写域**：同仓库存在并行会话（thinking-bottom-gap / tui-terminal-restore）持有 `bin/tui.rs`、`docs/decisions/README.md`、`tests/tui_terminal_restore.rs` 的未提交改动。Protocol Agent 首次 `--all-targets` 门禁因此失败（E0425 来自他人文件）。缓解手段是 **hunk 定向暂存**（按关键字切分 diff，`git apply --cached`），代价是每次提交前需重新切分、且行号会漂移。建议后续为本类并行会话提供 worktree 隔离（本仓已有 `pony-agent-worktrees/` 先例）。
- **测试分权**：冻结验收由 Test Agent 唯一编写，Executor 只读；R2 修复中 Executor 需要机械适配 `PermissionRequest` 字段（2 个既有测试字面量补 `approval_path: None`），属派生编译豁免，断言零改动。

## 4. 元协议迭代建议

1. **并行会话感知门禁**：在 `dev-team` 的波次生成阶段先跑 `git status --porcelain`，若存在**非本波**未提交文件，必须显式登记"并行写域"并在验收命令中排除受影响目标（本波 `--all-targets` 失败即此因）。
2. **hunk 定向暂存标准化**：把"按关键字切分 diff + `git apply --cached --recount`"沉淀为脚本（当前是 Lead 临时手写），避免每次重新推导。
3. **审查预算分层**：本波 4+3 路审查共 7 次调用，落在"波次 × 2"上限内；但单轮 4 路存在重复 BLOCKING 报点，建议后续对同一 BLOCKING 采用"首报者定稿、其余只做补位发现"。
4. **验收接缝可测性前置**：方案阶段就应判断"用户接缝是否可被集成测试驱动"，不可测则必须当场登记 backlog（本波 F-2），而不是等到审查轮被指出。
