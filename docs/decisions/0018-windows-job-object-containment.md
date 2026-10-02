# 0018 Windows Job Object 进程生命周期约束

Status: implemented (decision adopted; PA-077 engineering closeout remains blocked)

## 背景

PA-076 将 ProcessBackend 生命周期与 SandboxBackend 权限隔离分开。直接 Child kill 可能留下子孙进程。PA-077 采纳 Windows Job Object 生命周期管理，保持无人值守 Run fail-closed。状态表示决定已采纳，工程进度与审核证据仍由 PA-077 任务卡记录。

## 候选方案

- **方案 A：std Command spawn 后立即分配私有 Job（选定）**。保留现有参数、环境和管道行为，使用 Child::as_raw_handle 避免按 PID 重开句柄。代价是 spawn 到 assignment 之间存在受调度影响、无固定时间上界的窗口，窗口内已创建的后代可能逃离；不能作为绝对安全边界。极短命令提前退出可能使挂入失败，接受显式 start 错误而不是不受管成功。
- **方案 B：挂起启动/原生 CreateProcessW 或原子 job-list startup（本卡不选）**。能阻止启动初期脱逸，但必须扩展/替换进程启动路径，复核标准流、参数、环境和句柄继承。Rust 1.95 的 std raw-attribute spawn API 仍为 nightly。三路设计审核接受 A 的限定范围，不为此引入 nightly 或扩大原生启动工程；严格无窗口方案为未来独立工作。
- **方案 C：维持 Child::kill 或仅用 taskkill 按 PID 杀树（不选）**。没有 last-owner kill-on-close；外部工具/PID 树清理还有竞态，不提供 Job 内核生命周期保证。

## 决策

每个 Windows managed child 采用无名、不可继承、kill-on-close 且不允许 breakaway 的私有 Job。Job 在 spawn 前配置，spawn 后立即挂入并验证成员关系；失败终止/回收 direct child，不发布成功句柄，即便父进程已经退出也不能据此推断安全。kill/shutdown 显式终止已成功加入 Job 的成员，即便父进程已退出；终止错误可见并使用 Job close/kill-on-close 兜底，shutdown 保持 count API 并记录错误。最终 owner drop 关闭 Job 句柄，终止 Job 内仍存成员；spawn→assign 窗口内产生的未受管后代不在该保证范围内。

这是 best-effort 进程生命周期 containment，不是文件/网络沙箱；不注册 SandboxBackend、不更改现有 fail-closed 门禁、不改变公共序列化协议或非 Windows 行为。

## 影响与验证

增加 Windows-only windows-sys 直接依赖与 FFI RAII。测试使用 start-return 后 stdin 指令创建后代、就绪握手、native SYNCHRONIZE handle 和有界 WaitForSingleObject；故障注入仅限 manager 实例 cfg(test)，不使用全局开关。验证父子树终止、父进程先退出、clone/final-owner、错误清理、flags/breakaway 和 sandbox 门禁。Windows 实测与 Linux/macOS CI 编译证据分别报告，不把待跑 CI 当作已通过。

设计审核与意见采纳见 `management/task-system/02_REVIEWS/2026-09-30-pa077-design-review.md`；工程状态见 `management/task-system/03_TASKS/PA-077-windows-job-object-containment.md`。
