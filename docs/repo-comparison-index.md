# 外部源码索引

用于并排研究 `reasonix`、`codex`、`claude code` 的源码与缓存/上下文构造策略。

## 目录

- `codex-openai`
  - 来源: `https://github.com/openai/codex.git`
  - 当前分支: `main`
  - 当前提交: `6111791d0b`
- `reasonix-esengine`
  - 来源: `https://github.com/esengine/DeepSeek-Reasonix.git`
  - 当前分支: `main-v2`
  - 当前提交: `a704fa7`

## 现有本地快照

- `claude-code-sourcemap`
  - 这是本地已有的解包/还原源码快照，不是 Git 仓库。
  - 适合和 `claude-code-anthropic` 官方仓库交叉比对。

## 建议的下一步

优先从以下主题横向对比：

1. 请求前缀如何分层：稳定前缀、半稳定上下文、易变输入是否分开。
2. 工具调用后的 follow-up 如何压缩 tool result，是否避免破坏可缓存前缀。
3. 会话摘要是否放在请求靠前位置，以及摘要字段是否频繁变化。
4. 历史消息和 transcript 的截断策略，是否会导致前缀边界频繁跳变。
