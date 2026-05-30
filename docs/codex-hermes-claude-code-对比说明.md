# Codex / Hermes / Claude Code 对比说明

更新时间：2026-05-28

## 1. 当前目录位置

- `C:\Users\HUAWEI\Documents\pony-agent\codex-openai`
- `C:\Users\HUAWEI\Documents\pony-agent\hermes`
- `C:\Users\HUAWEI\Documents\pony-agent\claude-code-sourcemap`

三者现在已经处于同级目录，方便并排对照。

## 2. 开源性质

### Codex

- 官方开源的是 `Codex CLI / 本地 agent 实现` 这一层。
- 本地仓库来源：`https://github.com/openai/codex`
- 当前克隆提交：`6111791d0b`
- 许可证：`Apache-2.0`
- 注意：云端 `Codex Web / Codex App / 模型服务` 并不是完整开源；仓库主要对应本地运行的 agent、CLI、SDK 和相关工具链。

建议先看：

- `codex-cli/`
- `codex-rs/`
- `sdk/`
- `docs/`

### Hermes

- `Hermes Agent` 是完整开源 agent 框架，偏“通用 agent 平台”。
- 从当前目录结构看，重点模块包括：
  - `agent/`
  - `gateway/`
  - `providers/`
  - `plugins/`
  - `hermes_cli/`

更适合学习：

- 多平台入口
- 长期记忆/技能系统
- provider 抽象
- gateway 架构

### Claude Code

- 当前本地的 `claude-code-sourcemap` 不是 Anthropic 官方源码仓库。
- 它是基于 npm 包和 source map 还原出来的研究版目录。
- 更适合拿来观察：
  - CLI 组织方式
  - tools/commands/services 分层
  - 前端交互结构
- 不适合作为“官方真实工程结构”的唯一依据。

## 3. 三者最值得对照的维度

### Agent 核心循环

- Codex：更偏本地安全执行、审批流、可靠编码代理。
- Hermes：更偏长期运行、记忆、技能沉淀、多入口。
- Claude Code：更偏产品化 CLI 交互与工具编排。

### 工具系统

- Codex：看 `codex-rs/` 与 CLI 的工具调用边界。
- Hermes：看 `plugins/`、`providers/`、`agent/` 的扩展方式。
- Claude Code：看 `restored-src/src/tools/` 和 `commands/`。

### 产品形态

- Codex：本地 CLI + IDE + App + Cloud 的多端协同，但开源主要集中在本地 CLI 层。
- Hermes：开源 agent 平台，系统边界相对完整。
- Claude Code：公开可见更多是发布包产物，不是完整官方研发仓库。

## 4. 建议阅读顺序

1. `codex-openai/README.md` 和 `docs/`
2. `codex-openai/codex-cli/`、`codex-openai/codex-rs/`
3. `hermes/README.md` 和 `agent/`
4. `hermes/providers/`、`gateway/`、`plugins/`
5. `claude-code-sourcemap/README.md`
6. `claude-code-sourcemap/restored-src/src/tools/`、`commands/`、`services/`

## 5. 一个实用判断

如果你的目标是“学习 OpenAI 这类 coding agent 在本地是怎么搭起来的”，优先读 `codex-openai`。

如果你的目标是“学习一个更完整、更通用、可长期运行的 agent 平台”，优先读 `hermes`。

如果你的目标是“逆向理解成熟 CLI 产品如何分层与组织交互”，`claude-code-sourcemap` 最有参考价值。
