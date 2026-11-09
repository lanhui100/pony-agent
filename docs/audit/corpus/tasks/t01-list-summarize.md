---
id: t01-list-summarize
timeout_secs: 120
write_access: none
expected_spans: [agent.turn, llm.chat, tool.call]
---

# Prompt

列出目录 docs/audit/corpus 直属的 markdown 文件（不含子目录），并对每个文件用一句话说明其用途。只读操作，不要修改任何文件。