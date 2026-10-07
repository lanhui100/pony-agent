# Wave 1 元框架复盘（2026-10-07：CI core 测试遗留失败处理）

> 交付：main CI 全绿（verify 含 "Core 单元与集成测试" 1003/1003 + 集成二进制全过，ubuntu/macos 交叉检查通过）
> 范围：14 个 lib 失败 + 4 个红相契约二进制 + 并发遗留 WIP 处置；基线 805541e → 终态 97b301b
> 波次检查点：red_anchor=20d31be, green=2f9eae0, stabilize=90de197, closure=97b301b

## 1. 通信拓扑与信噪比

- 拓扑合规：Test Agent（T1+T3+T5）与 Executor（T2+T4）两常驻槽位 + Lead，Roster=3 ≤ 8；两路 L3 审查走 Subagent 池不占槽位。
- 信噪比良好：队友汇报均为结构化进展/根因/证据，无套话；审查走严苛 JSON（verdict+findings），机器可解析。
- 缺陷记录：Test Agent 交付期间出现一次外部 git 覆盖（分支被并发会话切到 hygiene-cleanup-1791356420，首版修改丢失后重做）——根因是共享工作区 + 多会话并发，无 git 写锁（写域排他公理已知的物理限制）。教训：Lead 每轮交互后应复查 `git branch --show-current`，防漂移；多会话并发时优先 worktree 隔离。

## 2. 门禁穿透与误杀率

- 红相有效：3 条 stale-expectation 修正由 Lead 冻结（20d31be），11 条 product-bug 保持红相交 Executor；红相锚定提交保证测试不丢。
- 绿相有效：Executor 本地证据（-D warnings check + 模块级测试）与 Lead 复核 diff 一致；两路 L3 审查（tools.rs 路径/逃逸、provider 空名收窄）均 PASS，其中 P1 级观察项（name=""+非对象 JSON 参数的空洞判定缺口）经 Lead 裁决为"相对 46040ae 窗口回归、相对更早基线无回归、无测试/已知网关触发"→ 记录 follow-up 不阻塞。
- 误杀率：首次发版失败（PCSTR 类型错误）暴露"Windows-only 代码无法本地编译验证"的盲区——本波改进：引用 vendored windows-sys 签名核对 + CI 复跑作为唯一 Windows 门禁；后续建议引入 `cargo check --target x86_64-pc-windows-msvc` 交叉检查（需 rustup target）。
- 关键发现：cargo test 默认在**第一个失败二进制处 fail-fast**，导致 4 个刻意冻结的红相契约测试（default_workspace_red_phase）长期被 lib 失败掩盖、从未在 CI 运行——CI 绿后首次暴露。教训：红相契约测试须显式标注/隔离，或 CI 使用 --no-fail-fast 以暴露全部失败。

## 3. 分工契约与隔离有效性

- 分工有效：Test Agent 只改测试、Executor 只改产品（写域 tools.rs/provider/process 串行化，T2 blocked_by T1 显式声明）；两处越界（provider/openai_sse.rs、T4 的 process.rs close_stdin）均经 Lead 批准扩展写域后执行。
- 契约隔离：分诊矩阵（14 条带 git 提交依据）作为 Test Agent→Executor 的唯一契约载体，Executor 未回写测试；审查者按 diff+矩阵窄视角，无历史噪声。
- 滚动账本：.dev-team/state.json 按 wave×task 记录 red/green/blocked，配合提交哈希可断点续传；本次无熔断触发。
- 并行度：全程 WIP=1 串行（写域重叠），符合"串行是默认保底"；波次 ROI 判定正确——同文件内联单测结构下并行无收益。

## 4. 元协议迭代建议（对 dev-team skill 的演进）

1. **共享工作区并发护栏**：多会话共享同一 checkout 时，Lead 应在每次派工/收口时校验 `git branch --show-current` 与 HEAD；或将写域任务强制 worktree 隔离。建议 skill 增加"并发会话共存检测"步骤。
2. **红相契约的 CI 可见性**：刻意冻结的红相验收测试不应静默躲在 fail-fast 之后；建议冻结时同时注册"预期失败"清单（如 .dev-team/expected-red.json），CI 绿后自动触发其完结审查，或 CI 用 --no-fail-fast 并显式允许已知红相清单。
3. **Windows-only 代码验证盲区**：建议把"对照 vendored windows-sys 签名 + CI 复跑"固化为 Windows 分支代码的标准验证步骤，并推动 rustup target 交叉检查能力。
4. **测试稳定化授权**：CI 环境负载方差导致的间歇超时，Test Agent 应获授权直接上调 timeoutMs 并在注释中记录缘由（本轮已实践，效果好）。
5. **P1 观察项裁决标准化**：对"相对某个提交窗口的回归、相对更早基线无回归"类观察项，明确裁决模板（基线对比 + 触发面 + 是否阻塞），减少人工裁量成本。

## 遗留 follow-ups（已记录于 state.json，均不影响 CI 绿）
- P1: name="" + 非对象 JSON 参数（[] 等）判非空洞 → normalize Err 整轮失败；建议 is_empty/is_hollow 公共判定纳入非对象可解析值（相对 46040ae 窗口）。
- 本地 Linux 全量挂起：start_turn_stream_completes_after_multi_hop_followup_stream（MockHttpServer 超量请求无兜底 + 跨测试污染触发 provider 重试），建议 mock fail-fast 或确定性时钟。
- P2: extract/partial/is_hollow 直接单测覆盖缺口（建议 name=""×参数形态矩阵）。
- P2: lexically_within_root 在 junction/8.3 短名根下假阳性 requires_authorization（fail-closed 方向）。
- P3: cmd /C 元字符 fail-closed 清单缺 `%`。
- ④(b): followup-sync 收到 SSE 包装响应时 post_openai_json 纯 JSON 解析失败（graceful 降级不崩溃）；建议 SSE 载荷重建，需独立评估。
