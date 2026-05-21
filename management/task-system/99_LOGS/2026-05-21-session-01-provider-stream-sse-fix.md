# Session Log 2026-05-21 / 01

## 本次做了什么
- 复盘了真实联调里出现的 provider 失败问题
- 确认错误不在 tool，而在 OpenAI 兼容 provider 的流式返回解析
- 修复了 `provider.rs` 中把 SSE 文本流误当普通 JSON 解析的问题
- 补上了对 `data: ...` chunk 的逐行聚合逻辑
- 重新跑通 `cargo check`
- 重启了 `tauri dev`，让联调吃到新代码

## 这次故障说明了什么
- “live provider 已接入”不等于“stream 主链已经稳定”
- provider adapter 不能把外部协议细节直接漏给 runtime
- SSE 解析属于 provider 层责任，而不是 tool 层责任
- 这类问题很适合沉淀成学习文档，因为它能帮助区分协议问题、runtime 问题和工具问题

## 当前结果
- OpenAI 兼容 provider 的流式 follow-up 主路径已能正确读取 SSE 文本流
- 本轮最关键的误判已被纠正：之前报错不是 tool 失败，而是 provider stream parse 失败
- 当前可继续把关注点放回 agent core 的边界收束，而不是继续在“是不是网络问题”上兜圈子

## 下一步动作
- 更新学习文档，记录“provider stream = 协议适配问题，而非工具问题”
- 在任务系统里把下一步拆成更适合并行推进的两条线：
- `PA-007` 独立接入层
- `PA-008` 工具层补强
