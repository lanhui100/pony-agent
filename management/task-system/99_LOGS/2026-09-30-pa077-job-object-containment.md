# PA-077 Windows Job Object containment（2026-09-30）

## 请求与范围

用户要求继续，按上一轮首推方向启动 PA-077。采用 dev-team C 级流程，复用现有 task-system/OpenSpec，维护 ADR 0018；只做 Windows best-effort 生命周期 containment，不提供完整文件/网络沙箱，不更改 fail-closed 审批，不擅自提交。

## 本轮已完成

- 创建 PA-077 任务卡和 `add-windows-job-object-containment` proposal/design/tasks/delta spec。
- 创建 ADR 0018，三路设计审核后由 proposed 路由 implemented（决定采纳，不表示工程完成）。
- 三路独立设计审核条件通过；采纳：父已退出仍显式 Job termination、start 事务清理、实际 owner/drop 语义、native wait-handle 握手测试、per-manager failure injection、无固定上界的 race、短命令提前退出 fail-closed。
- 旧 PA-076 评估补充：Child 实际支持 AsRawHandle；不再承诺毫秒级窗口；raw-attribute spawn 仍 nightly。
- CI 补入 Windows core 测试和 Linux/macOS core cfg compile-check jobs（当前为配置变更，远端未运行）。
- 实现委派 engineer `a79ed1a4-ca54-4101-adea-8c3a05cf589d`：仅 core Cargo/process/Windows module/tests；主会话负责 docs/CI。

## 基线与当前证据

- Windows baseline process lifecycle：13 passed / 0 failed。
- Windows baseline sandbox authorization：6 passed / 0 failed（有效命令 npm cargo:test:shared -- --package=pony-agent-core --lib agent::sandbox::tests）。
- 初次通过 `-p` 传包名的 npm 包装器实际跑宿主且 0 matching tests，不计通过证据；后续统一 `--package=pony-agent-core` 并核对 crate/count。
- OpenSpec strict：53 canonical specs + 1 active change all valid；ADR lifecycle/version gates passed。
- 前端回归：38 files / 568 passed / 10 skipped；vue-tsc + Vite build exit 0。既有 motion directive / duplicate keys / await assertions / chunk-size warnings保留，不扩大本卡范围。
- CI reviewer 曾称 --package 无法选择 core；不采纳：根 Cargo.toml workspace members 包含 core/tauri，cargo metadata --manifest-path src-tauri/Cargo.toml 证实根 workspace，两次实际 core tests 输出可证包选择成功。reviewer 已更正并撤回 P1，最终 CI ACCEPT。CI YAML 使用 yaml parser 成功解析两项 jobs（verify/core-platform-check）。

## 实现回改与终审前状态

- 基础实现首次 Windows 定向测试为 20/20；两路独立实现/安全审核发现 P1：assign/verify 与 pipe rollback 忽略 kill/reap 失败，测试只证明 map 为空；另有 deterministic exit-before-assign、exited-parent kill、noninherit 等证据缺口。
- 工程回改增加 Windows `kill_and_reap` 有界回收与错误诊断、失败启动 DuplicateHandle + `WaitForSingleObject` 观察、确定性 wait-before-assign seam、pipe rollback 观察、OwnedHandle Job + 互斥 close fallback；定向 process tests 21/21 通过，core 全量 `970 passed / 0 failed / 2 ignored`，tools tests 111/111，shared check 通过。
- 后续针对安全 reviewer 指出的显式 `TerminateJobObject` 失败泄漏风险，在 exited kill、running terminate 的 reap-error/normal 分支统一 `Job::close()` kill-on-close fallback；workspace command timeout/normal/poll cleanup 不再静默丢弃 kill 错误；timer cleanup 改为记录错误。
- 新增/验证：Job handle `HANDLE_FLAG_INHERIT=0`、assign/verify/pipe 失败的真实 child native wait、提前退出后 fail-closed、父退出后显式 kill 分支。当前仍明确不宣称严格 containment：spawn→assign scheduler-dependent window 可逃逸；nested host Job/breakaway、timer+outstanding Arc 的额外集成证据未补，作为 best-effort 限制/后续测试项记录。
- 独立 fixed-snapshot reviewer：correctness conditional pass，无 P0/P1 lifecycle blocker；security reviewer仍将 strict containment race 视为不可接受，但与已批准 best-effort设计一致。最终采纳口径为生命周期纵深防御，不是安全边界。

## 当前状态与下一步

实现与本机验证已完成，尚待：重新运行 version/OpenSpec/ADR gates、同步 canonical spec/归档 active change、任务板/Dashboard/日志收口，并准备但不执行 commit。Linux/macOS CI 只配置未本机实测。


## Gate0 对账追加（2026-09-30）

- 三路独立 Gate0 reviewer 均有条件通过：治理 reviewer 要求沿用既有 `Review` 状态、不得引入未定义 `Validation`；测试 reviewer 要求拆分 canonical/active OpenSpec 结果并记录 ACL/fmt/平台证据边界；安全 reviewer 要求维持 best-effort/non-sandbox/fail-closed 免责声明。
- PA-077 任务卡、任务板、Dashboard 已改为一致语义：实现已落地；当前处于 `Review`，closeout gate blocked；不代表功能实现未完成，也不代表可发布/已提交。
- 稳定引用已指向 `openspec/changes/archive/2026-09-30-windows-job-object-containment/` 与 `openspec/specs/windows-process-containment/spec.md`。archive tasks 3.4 暂不勾选，待当前精确 diff、验证和平台证据闭合后再复核。
- 真实门禁分层：version check Passed；canonical strict 54 passed；active PA-103 failed；`npm run openspec:check` 不存在；cargo fmt 存在预存差异；PA-077 定向测试受 `target-test\\debug\\.cargo-lock` ACL 阻断；Linux/macOS CI 已配置但未本机执行。
- 本轮未修改运行时代码、Cargo、测试、CI 或 PA-103；未提交、未重置、未删除混合工作树改动；PA-044 保持 Ready/冻结，不启动。
- 安全口径保持：Job 是 best-effort 生命周期/进程树 containment，不是严格 containment、SandboxBackend 或文件/网络/令牌/AppContainer/审批边界；spawn→assign race 非安全边界；NoSandboxBackend/enforce_sandbox 继续 fail-closed。

## Gate0 Resume Hint

下次先打开 `management/task-system/02_REVIEWS/2026-09-30-pa077-gate0-closeout-review.md`，核对 PA-077 最小 diff 归属与 target-test ACL 解除情况；不要启动 PA-044 或 PA-077 新功能实现。
