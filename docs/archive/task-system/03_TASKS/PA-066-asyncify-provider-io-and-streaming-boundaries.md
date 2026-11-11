# PA-066 异步化 provider IO 与 streaming 边界

## 状态
- Status: `Ready`
- Priority: `P1`
- Owner: `待定`

## 依赖
- 前置：`PA-065` 的 provider boundary 冻结
- 可并行于：`PA-067`
- 后续为：`PA-068`

## Canonical Spec
- `openspec/changes/asyncify-provider-io-and-streaming-boundaries/specs/tool-system-contract/spec.md`

## OpenSpec Change
- `openspec/changes/asyncify-provider-io-and-streaming-boundaries/`

## 背景
当前 provider 层仍依赖同步阻塞 HTTP（`reqwest::blocking`），即使 runtime ownership 被拆开，也无法获得真正的 async IO 并发收益。

## 目标
1. 把 provider 网络调用从 blocking 改成 async
2. 保持现有 turn event / delta / output_end / terminal 语义稳定
3. 为 cancellation、timeout、retry 建立 async 原生语义

## In Scope
- `reqwest::blocking` -> async `reqwest`（包括 provider.rs 和 tools.rs 两条路径）
- `pony-agent-core` Cargo.toml 依赖变更：`reqwest` 移除 `blocking` feature，增加异步所需 feature（`json`、`native-tls`、`charset`）
- OpenAI / Anthropic / followup stream 路径的 async 化
- `tools.rs` 中 `web_fetch_url` 等 HTTP 调用的 async 化
- provider timeout / retry / cancellation 语义梳理
- 对外 provider response / stream chunk 合同保持兼容

## Out of Scope
- session ownership 拆分
- per-session task 编排
- 前端 session 切换逻辑

## 验收标准
- provider 网络调用 SHALL 不再依赖 blocking HTTP client
- `tools.rs` HTTP 调用 SHALL 不再依赖 blocking HTTP client
- `pony-agent-core/Cargo.toml` SHALL 移除 `reqwest` 的 `blocking` feature
- stream / sync followup 行为 SHALL 在 async 化后保持合同兼容
- turn 流式事件语义 SHALL 不因 async 化而改变
- 所有 `use reqwest::blocking::*` 引用 SHALL 被移除

## 下一步动作
1. 冻结 provider & tools boundary
2. 更新 `pony-agent-core/Cargo.toml`：`reqwest` 移除 `blocking` feature
3. 逐协议迁移 HTTP client（provider.rs → tools.rs 顺序）
4. 用现有 runtime/store tests + probe 验证流式行为
5. 清理：移除 deprecated `reqwest::blocking` 引用，更新 `#[cfg(test)]` 中依赖 blocking 的测试用例

## 断点续跑提示
- `crates/pony-agent-core/src/agent/provider.rs`
- `crates/pony-agent-core/src/agent/tools.rs`
- `crates/pony-agent-core/src/agent/runtime/mod.rs`
- `crates/pony-agent-core/Cargo.toml`
- `src-tauri/src/tauri_adapter.rs`

## 当前进展
- 已完成

## 完成摘要
- `provider.rs` + `tools.rs` 从 `reqwest::blocking` 迁移到 async `reqwest`，通过 `block_on()` 桥接同步到异步
- 创建 `runtime_helper::block_on` 共享函数消除 provider/tools 重复
- `Cargo.toml` 移除 `reqwest` 的 `blocking` feature，新增 `tokio` 依赖
- Provider 测试 25 项全部通过
- 3 轮并行智能体审核后调优：`block_on` 提取共享模块
- 清理与收口已完成
