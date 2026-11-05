# 0064 TUI 会话韧性：DSML 折叠、错误摘要、中断检查点与退出回收

Status: implemented

## 背景

上游把原始 DSML 工具调用协议文本放进 SSE `delta.content`，TUI 曾原样渲染、原样持久化并回灌 provider；终态错误把 400 字符响应体直接铺在消息区；中断轮次只留占位文案；主循环退出后在飞 turn 线程仍持有 tty，进程不真正退出。本波按冻结契约 C1–C8 修复这四条链路。

## 候选方案

- **A（采纳）**：provider 层一个纯函数折叠 DSML 块，TUI 流式增量与历史回放共用；错误详情只留运行日志与既有 `capture_turn_failure` issue 通道，用户可见面收敛为一句话摘要；中断检查点以 additive 文本进入既有会话历史；退出经既有 `stop_turn` + 有界等待 + `restore_terminal_modes` + `print_resume_hint` + `process::exit`。不新增依赖、网络端点、环境变量、持久化 schema 或重试预算。
- **B（落选）**：在 TUI 状态机里做跨 chunk 缓冲与重试进度事件、新增独立 checkpoint 表/字段、TUI 自带重试循环。改动跨多个公共契约，且与 provider 既有 5 次/30s 预算叠加会制造重试风暴；对已落盘的旧会话还会引入字段迁移成本。
- **C（落选）**：只修消息区渲染，不改 SSE 解析与持久化。历史回放仍会再次回灌 DSML 原文，根因未闭合。

## 决策

保持 provider 层既有重试预算不叠加；DSML 过滤收敛为单一纯函数并在 SSE 解析/累计终态/TUI 历史回放三处复用；终态错误默认折叠；中断轮次以 additive 文本保存已落盘工具步骤；退出按 `cancel → wait → restore → hint → exit` 顺序收口，等待上限 3000ms。

## 影响

- 用户不再看到 DSML 协议原文、半截标记或 400 字符响应体；可重试瞬态错误显示「重试中（n/N）」口径的简短状态。
- 旧会话无检查点文本时逐字回放既有历史；`pony resume <id>` 命令形状不变。
- 退出后进程真正结束，不留持有 `/dev/tty` 的在飞 turn 线程；`docs/observability.yaml` 仅作归属登记，四维归属不变。
