# 证据：TUI /resume + 归档 PTY 冒烟（B264）

- 二进制：`target-test/debug/pony`（HEAD `469d4c0c`，`cargo-lane build` Exit 0）
- 驱动：`pty-smoke.py`（隔离 HOME=/tmp/pony-tui-smoke-home，100×30 pty）
- 原始终端输出：`resume-panel-pty-2026-11-21.txt`

## 观测量（真实渲染文本，非断言伪造）
- 面板标题：`会话 · 共 0 条`
- 操作提示：`↑↓ 选择 · ↵ 确认 · a 显示归档 · d 归档 · u 取消归档 · esc 取消`
- 空态：`暂无历史会话（当前工作区 0 条 · 全部 0 条）`
- 按 `a` 后：状态行 `已显示归档会话`，标题追加 `· 含归档`
- 进程无 `panicked`，`/quit` 正常退出（exit 0）

## 局限
空库场景只覆盖「0 条」形态；非空列表（跨区前缀、短 id、归档行 dim）由冻结测试
`tests/tui_resume_archive_acceptance.rs`（渲染级 TestBackend）覆盖。
