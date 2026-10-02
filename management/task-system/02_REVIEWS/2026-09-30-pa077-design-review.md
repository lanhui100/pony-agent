# PA-077 设计对抗审核（2026-09-30）

## 当前审核状态

- Security/correctness reviewer `cb529cff-29b6-4730-8c11-839232a056db`：PASS WITH FINDINGS，仅限 best-effort 进程树生命周期 containment。
- 首轮 architect `066a66ef-f578-4417-a053-987ff0b4ab59` 和 consultant `13fa813d-1abe-4237-9530-a0a096831bb9` 在最终回执前失败；不计作完整审核。consultant 的早期意见已用于修订设计，但不算通过。
- 独立 architect `3b5f4470-5464-492f-b0a6-7f812b5798ad`：条件接受 best-effort 范围；要求明确短命令提前退出时仍 fail-closed，现已补入 design/spec/task。
- 独立 consultant `b7190660-7a5e-4687-a270-29fbf55c429b`：CONDITIONAL PASS / implementation may start；没有额外设计 blocker，实施与测试仍需落实所有硬门禁。
- 汇总结论：三路设计审核有条件通过，开始实现；不是代码通过或验收完成。

## 已采纳意见

1. P1：显式终止 Job 必须在父 Child 已 Exited 的分支也执行，不靠 map removal / RAII 的引用计数时机保障 kill/shutdown。
2. P1：start 事务化。创建/配置 Job 在 spawn 前，spawn 后借用 Child::as_raw_handle 立即 assign/verify；所有错误（含管道接管）结束 child、关闭 Job、不发布 process handle。kill 返回终止错误；shutdown 原 count API 保留但记录错误并释放 owner。
3. P2：窗口受调度影响，没有固定时间上界；删除“毫秒级”表述。保持纵深防御定位，不提升为 SandboxBackend availability。
4. P2：Job 句柄唯一 RAII owner，Child 的 process raw handle 仅借用，不转移/重复关闭。明确 clone、timer 强引用与最终 owner 定义。
5. P2：测试先等 start 返回，再通过 stdin 指令创建后代，握手后打开 native wait handle；以 WaitForSingleObject 验证退出，不能仅靠 sleep 或 PID 查找。
6. P2：失败注入按 manager 实例、cfg(test) 隔离，不使用全局可变开关。覆盖设置/挂入/验证/终止失败及不发布记录。
7. P2：查询实际 flags、non-inheritable 属性、nested Job 与 breakaway 拒绝，保留现有跨会话隔离与 poll/stdin 回归。
8. P3：SandboxBackend fail-closed 独立回归；非 Windows cfg/编译验证按实际运行证据报告。
9. P1（architect）：短命令若在 assign/verify 前退出，也不据此推断安全。任何挂入/验证失败返回错误且不发布 handle；成功则照常保留终态 polling。兼容性代价明确记录，要求确定性 exit-before-assign 与自然 echo 回归。

## 设计依据校正

PA-076 旧评估称 std Child 不暴露 HANDLE；Rust Windows Child 实际实现 AsRawHandle，本卡直接借用已持有句柄，避免 PID reopen。Rust 1.95 的 spawn_with_attributes / main_thread_handle 仍为 nightly API；不引入 nightly 来扩大本卡实现边界。

## 已运行基线

- Windows process lifecycle tests：13 passed / 0 failed（baseline，尚未修改 Rust）。
- Historical design-stage command: `openspec validate add-windows-job-object-containment --strict` passed for the then-active change; the stable archived change is now `openspec/changes/archive/2026-09-30-windows-job-object-containment/` and the current Gate0 strict evidence is recorded separately.
- `verify-decisions.ps1` / `npm run version:check`：通过。
- core sandbox authorization baseline：6 passed / 0 failed。先前 `-p` 传参被包装器误解、0 matching tests 的运行不计有效证据；使用 `--package=pony-agent-core` 后核对为 core 目标 6 项。

## CI 增量审核

- Architect 独立审核最终 ACCEPT。最初误认为 src-tauri manifest 不在 workspace、package selector 无效；根 Cargo.toml 与 cargo metadata 及实际 core 6 项通过反证，reviewer 已撤回该 P1。不改无必要的构建脚本。
- Linux/macOS 使用 hosted-runner pwsh 是明确的 runner 依赖；Ubuntu libdbus/openssl/pkg-config 安装覆盖 core 原生依赖。本机未实际跑非 Windows 编译。
