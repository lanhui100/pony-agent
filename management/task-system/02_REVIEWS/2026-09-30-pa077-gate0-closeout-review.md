# PA-077 Gate0 收口对账审核（2026-09-30）

## 结论

**有条件通过 Gate0 文档/状态对账；PA-077 保持 `Review`，closeout gate 为 `Blocked`。**

本轮只校正任务系统与证据引用，不修改 Rust/TypeScript/Tauri 运行时代码、Cargo 依赖、测试、CI 或 PA-103 active change；不提交。PA-077 的实现事实保留为“当前工作树已落地的 Windows best-effort Job Object 生命周期/进程树 containment”，不得表述为严格 containment、完整 SandboxBackend 或已发布集成。

## 三路对抗审核

| Reviewer | 范围 | 结论 | 采纳项 |
|---|---|---|---|
| 架构/治理 reviewer `cad497fb-8613-4a6f-8972-5f66ef725cb8` | 状态枚举、任务板分区、OpenSpec/任务卡引用 | 有条件通过 | 使用既有 `Review` 主状态；将 closeout blocked 作为说明，不引入未定义的 `Validation` 状态；PA-044 保持 Ready/冻结 |
| 测试/交付 reviewer `0eea9b57-58f9-4480-a7db-e713867df1c8` | 可复现门禁、历史证据与当前证据分层 | 有条件通过 | 历史快照曾记录 PA-103 active failure；当前复核为 54/54 passed；保留不存在的 `npm run openspec:check`、fmt 基线差异、target-test ACL 阻断、Linux/macOS 未本机执行 |
| 安全 reviewer `1fe11756-2d5b-431e-8010-fd9371b33d10` | containment 语义、sandbox 边界、平台证据 | PASS WITH FINDINGS | 保留 spawn→assign race 非安全边界、Job 非 SandboxBackend、NoSandboxBackend/enforce_sandbox fail-closed 与跨平台未执行免责声明 |

前置设计与实现审核仍以原记录为准：设计三路审核是有条件通过；实现审核是批准 best-effort 范围内的 conditional pass，不是严格安全批准。

## Gate0 事实

- 当前 Git 基线：`HEAD=4f16b83`，PA-077 runtime 相对 `origin/main@87a6562` 无差异；当前未提交改动为任务系统收口文档修改。
- PA-077 OpenSpec change 的稳定引用已指向 `openspec/changes/archive/2026-09-30-windows-job-object-containment/`；canonical spec 为 `openspec/specs/windows-process-containment/spec.md`。
- `npm run version:check`：Passed（core 0.1.92 / Tauri 0.1.96）。
- `npx openspec validate --all --strict`：Passed，54/54；此前 PA-103 active change failure 已解除并归档，不再作为当前 PA-077 阻塞项。
- `npm run openspec:check`：Not applicable / repository script 不存在；不得把该命令写为通过。
- Scoped PA-077 runtime format check: `rustfmt --edition 2021 --check` over `process.rs`, `process/windows_job.rs`, `sandbox.rs`, and `tools.rs` passed (exit 0; raw `target-pa077-check/pa077-rustfmt-scoped-2026-10-02.txt`). This does not override manifest-level `cargo fmt --check` (exit 1; broad pre-existing diffs, raw `target-pa077-check/pa077-fmt-2026-10-02.txt`).
- Full-core regression: `npm run cargo:test:shared -- --package=pony-agent-core` started 970 tests but stopped at two `agent::runtime::tests::start_turn_stream_*` cases reporting over 60 seconds; the process later exited without final summary or exit code. Raw: `target-pa077-check/pa077-cargo-core-test-2026-10-02.txt`. Status: `Incomplete/Stalled`, not Passed or Failed. `npm run cargo:check:shared` independently passed (raw `target-pa077-check/pa077-cargo-check-2026-10-02.txt`).
- PA-077 targeted process gate: after ACL repair, exact command `npm run cargo:test:shared -- --package=pony-agent-core --lib agent::process::tests` passed; `21 passed / 0 failed / 0 ignored / 949 filtered out`, exit 0, raw `target-pa077-check/pa077-process-rerun-2026-10-02-after-acl.txt`. Earlier exit 101 is historical pre-repair evidence; this is a filtered module gate, not full-core or cross-platform validation.
- Linux/macOS：CI jobs 已配置，当前无 raw CI run/artifact；本机未执行，记录为 `Not run`，不得写成跨平台验证通过。

## 安全语义

PA-077 提供 Windows Job Object 的 best-effort 进程生命周期/进程树 containment，用于纵深防御；它不是严格 containment，也不是文件系统、网络、令牌、AppContainer 或审批沙箱。`Command::spawn()` 到 `AssignProcessToJobObject()` 之间存在受调度影响且无固定时间上界的 race；该窗口不是安全边界，窗口内创建的后代可能未受 Job 约束。PA-077 不消除该 race、不注册 SandboxBackend、不授权 autonomous Run。`NoSandboxBackend` 与 `enforce_sandbox` 保持 fail-closed；缺少真实 sandbox backend 时无人值守 Run 仍拒绝。

## 状态裁决

- PA-077：`Review`；实现已完成，closeout validation gate blocked。不可进入 Done，直到当前精确 diff、定向测试、适用门禁与跨平台证据完成对账。
- PA-044：不启动，保留既有 `Ready` 状态；本轮无实现、无代码影响、无提交，不把它误标 Done。
- Blocked 分区：不把 PA-077 作为“功能未完成”列入；阻塞原因记录在 Review 条目中。

## 采纳/不采纳

- 采纳：使用既有状态枚举；把“实现完成”和“收口验证阻塞”拆开；稳定引用 archive/canonical；准确记录 Passed / Not applicable / Blocked / Not run。
- 不采纳：引入未定义的 `Validation` 状态；修改运行时代码或测试绕过 ACL/fmt；把 CI 配置当执行结果；把 Job Object 升级为 strict containment/Sandbox；启动 PA-044。
- 暂不勾选 archive `tasks.md` 的 3.4：本轮任务卡、Board、Dashboard 与本日志已同步，但当前精确测试/门禁/平台证据仍未闭合，且需后续复核后才能宣称完整归档收口。

## 下一步最小动作

1. 冻结 PA-077 新功能开发，不启动 PA-044、完整 SandboxBackend 或 strict suspended-start 方案。
2. 保留 PA-077 runtime 与 `origin/main@87a6562` 无差异的归属证据；共享 Cargo/CI/治理文件不整文件 reset/restore，不执行提交。
3. ACL 子门禁已关闭：原定向命令已通过 21/21；保留成功 raw 输出与历史 exit 101 失败记录，不再重复 ACL 修复。
4. PA-103 当前门禁已解除并独立归档；本卡不修改其发布流程。无需将 PA-103 作为 PA-077 strict validation 前置条件。
5. 取得跨平台 CI 结果或明确保持 Not run，再启动实现/收口三路复核。

## Resume Hint

下次先打开：

- `management/task-system/03_TASKS/PA-077-windows-job-object-containment.md`
- `management/task-system/02_REVIEWS/2026-09-30-pa077-gate0-closeout-review.md`
- `openspec/changes/archive/2026-09-30-windows-job-object-containment/tasks.md`
- `openspec/specs/windows-process-containment/spec.md`
