---
id: t03-search-locate
timeout_secs: 120
write_access: none
expected_spans: [agent.turn, llm.chat, tool.call]
---

# Prompt

在 crates/pony-agent-core/src 目录内搜索字符串 exported_trace_id，报告命中总数以及前 5 处命中的文件路径与行号。只读操作，不要修改任何文件。