# Proposal: Asyncify provider IO and streaming boundaries

## Why

当前 provider 层仍使用同步阻塞 HTTP client，这意味着即使 runtime ownership 被拆开，网络 IO 仍会占用线程并限制真正的多对话并发收益。

## Scope

- provider 网络路径从 blocking 改为 async（provider.rs + tools.rs）
- followup stream / sync fallback / retry / timeout 语义转到 async 模型
- `pony-agent-core/Cargo.toml` `reqwest` feature 调整
- 保持现有 turn event 合同稳定
- 清理：移除 deprecated blocking 代码，更新相关测试

## Non-goals

- 本 change 不负责拆 runtime ownership
- 本 change 不负责最终 per-session task 编排
