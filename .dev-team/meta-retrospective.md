# Meta-Retrospective —— Run 工具 Windows 引号损坏 + 失败误分类修复

## 1. 通信拓扑与信噪比（Topology & Noise）
- 采用 Lead + Test Agent + Executor + Reviewer 四人团队；消息总数约 15 条，均含明确动作指令（实现规格、评审裁决、紧急提醒），无过程性废话；Reviewer 全程遵守 JSON 零废话协议（review-design.json / review-final.json 均为纯 JSON）。
- 信号增强点：Lead 将评审 10 项发现逐条裁决后以单一实现规格消息下发，避免了多轮往返；test-agent 的收工报告含测试行号、红相证据、commit hash 三项可复核事实，信噪比高。
- 噪声源：并发会话（PA-114/PA-118）在同一工作区 git 提交与 reset，导致本团队多次误判现场（实现被清、提交串入他人 hunk），消耗 2~3 轮排查；属于共享工作区多会话的固有噪声，本团队无法消除，只能靠"尽早提交"缓解。

## 2. 门禁穿透与误杀率（Gate Penetration）
- 红相有效：Bug B 红相实测 `left: Some(InvocationFailed), right: None`（真断言失败）；Bug A 红相为编译失败（E0425，函数不存在）——弱于断言红相，但因 Test Agent 契约先行 + 实现签名冻结，未造成假绿。
- 绿相机器门禁：`-D warnings cargo check --lib` PASS；靶向子集 B1-B5(5/5)、A1-A3(3/3)、run_command_(9/9)、tool_error(4/4) 全绿。全套 `--lib` 被沙箱网络测试挂起（环境问题），采用靶向子集 + 评审采信，属降级验证，已显式标注。
- 误杀/穿透：Reviewer 阶段 1 正确拦截（FAIL，10 项发现全部有效），阶段 2 PASS 且 4 条 minor 均不阻塞；无假放行。
- 漏检风险：A4/A5（Windows 集成）本机无法执行，仅靠 CI windows-latest —— 存在"引号形态在真实 Windows 上仍有残留差异"的理论风险，已由 design review 的 F1 论证覆盖（cmd 规则 1 保留引号路径），接受该残余风险。

## 3. 分工契约与隔离有效性（Contract Isolation）
- Test Agent 冻结契约（A1-A5/B1-B5）→ Executor 按契约实现 → Reviewer 按契约核验，三权分离成立；Executor 未改任何测试代码，Test Agent 未改任何实现函数体。
- 写域声明（任务看板）与实际写域基本吻合；但 tools.rs 为共享文件（Test Agent 测试区与 Executor 实现区同在），发生一次同文件并发编辑，靠 git 顺次提交化解，未造成内容丢失。
- 契约冻结有效：Reviewer 的 F1-F10 逐条对应实现或测试断言，未发现契约与实现语义背离。
- 隔离漏洞：并发会话 git reset 两次清掉本团队未提交实现（types.rs 修复、tools.rs 实现）——写域隔离无法防御跨会话 git 操作，教训：共享仓库中所有实现必须"编译通过即提交"，不得积压。

## 4. 元协议迭代建议（Self-Evolving Protocol）
1. 【dev-team skill 增补】"并发会话共享工作区"场景：Lead 首次发现外部会话在跑时，应立即在团队内宣布 git 安全纪律（禁止 reset --hard / checkout 他人文件；实现完成即提交；提交前 git status 检查是否夹带他人 hunk，夹带时用 `git add -p` 精确暂存），而非等被清两次后才下发。
2. 【dev-team skill 增补】红相容忍度分级：编译失败红相（E0425）应作为"弱红相"显式标注并要求契约签名先行冻结，避免把编译失败误当断言红相验收。
3. 【环境】全套 `cargo test --lib` 在沙箱被网络用例挂起：建议维护"靶向测试过滤清单"（如 bug_b_/windows_batch_script/run_command_/tool_error）作为快速回归基线，避免每次全量等待。
4. 【提交卫生】Reviewer 发现"提交夹带他人 hunk"（77fee3d 混入 PA-114 注册表契约测试改动）——建议团队约定：同文件多会话并发时，提交前执行 `git diff --cached --stat` 检查提交面，或用 `git add -p`。
5. 【ADR 跟进（minor）】新错误码 `batch_script_write_failed` 与 `chcp 65001` 编码行为变化应按 error-contract C7.2 补 ADR/注释登记。
