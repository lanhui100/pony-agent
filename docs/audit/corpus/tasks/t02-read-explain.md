---
id: t02-read-explain
timeout_secs: 120
write_access: none
expected_spans: [agent.turn, llm.chat, tool.call]
---

# Prompt

读取 scripts/observability/check-langfuse.sh，说明其退出码 0、2、3 分别代表什么判定，以及为什么缺凭据时不允许判 PASS。只读操作，不要修改任何文件。