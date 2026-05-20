# Pony Agent 学习记录索引

## 目的

本目录用于沉淀 Pony Agent 重构过程中的学习内容，特别记录：

- 关键提问
- 系统化梳理
- 概念解释
- 技术取舍理解
- 可复用的内容素材

这些内容既服务项目内部学习，也为未来输出公众号文章、知乎文章提供素材池。

## 使用原则

1. 提问不是聊天碎片，而是知识资产
2. 每条学习记录尽量回答一个清晰问题
3. 不只写结论，要写为什么
4. 有内容传播价值的点，要补“可延展选题”

## 当前条目

- [0001 学习式重构的目标、已完成工作与下一步](C:/Users/HUAWEI/Documents/pony-agent/docs/learning/0001-learning-mode-rebuild-overview.md)
- [0002 为什么前端选 Vue 3 + Pinia，暂不上 vue-router](C:/Users/HUAWEI/Documents/pony-agent/docs/learning/0002-frontend-stack-vue-pinia-no-router.md)
- [0003 Rust 智能体核心调试为什么优先用 Tauri UI 而不是 TUI](C:/Users/HUAWEI/Documents/pony-agent/docs/learning/0003-tauri-ui-over-tui-for-agent-debugging.md)
- [0004 Pony Agent 需要哪些 Rust 开发环境，它们分别做什么](C:/Users/HUAWEI/Documents/pony-agent/docs/learning/0004-rust-environment-basics.md)
- [0005 用 Hello World 学 Rust 最基础语法](C:/Users/HUAWEI/Documents/pony-agent/docs/learning/0005-rust-hello-world-basics.md)
- [0006 如何理解 run_turn()、while loop 和 ReAct 的关系](C:/Users/HUAWEI/Documents/pony-agent/docs/learning/0006-run-turn-while-loop-react.md)
- [0007 Pony Agent 第一版 run_turn() 实现了什么](C:/Users/HUAWEI/Documents/pony-agent/docs/learning/0007-first-run-turn-implementation.md)
- [0008 白屏问题、工作台布局与学习式 UI 的关系](C:/Users/HUAWEI/Documents/pony-agent/docs/learning/0008-blank-screen-and-ui.md)
- [0009 run_turn 与 Claude queryLoop 的关系](C:/Users/HUAWEI/Documents/pony-agent/docs/learning/0009-run-turn-and-claude-query-loop.md)
- [0010 compaction、cache 与上下文治理为什么不能粗暴处理](C:/Users/HUAWEI/Documents/pony-agent/docs/learning/0010-compaction-and-cache.md)
- [0011 Provider 配置、env 策略与模型编辑](C:/Users/HUAWEI/Documents/pony-agent/docs/learning/0011-provider-config-and-env.md)
- [0012 当前 stream runtime、Hermes/Claude 对照与下一步架构路径](C:/Users/HUAWEI/Documents/pony-agent/docs/learning/0012-stream-runtime-and-future-architecture.md)
- [0013 为什么要从最小 tool 闭环切到 OpenAI/Anthropic 原生 tools 协议](C:/Users/HUAWEI/Documents/pony-agent/docs/learning/0013-native-tools-protocols.md)
- [0014 本地 planner、跨轮历史与工作区工具为什么要前置](C:/Users/HUAWEI/Documents/pony-agent/docs/learning/0014-local-planner-and-history-carry.md)
- [0015 为什么要把缓存命中纳入 runtime 重构约束](C:/Users/HUAWEI/Documents/pony-agent/docs/learning/0015-prompt-caching-as-runtime-design-constraint.md)
- [0016 为什么工具调用要先变成 ToolCall / ToolResult，而不是直接调函数](C:/Users/HUAWEI/Documents/pony-agent/docs/learning/0016-tool-call-as-data-not-direct-function-call.md)
- [0017 为什么要先做 SessionStore，再做新对话和历史对话](C:/Users/HUAWEI/Documents/pony-agent/docs/learning/0017-session-store-and-context-boundary.md)
- [0018 对话历史侧栏与会话管理 UI 的设计取舍](C:/Users/HUAWEI/Documents/pony-agent/docs/learning/0018-session-navigation-ui-and-tradeoffs.md)

## 记录模板

- [学习记录模板](C:/Users/HUAWEI/Documents/pony-agent/docs/learning/TEMPLATE.md)
