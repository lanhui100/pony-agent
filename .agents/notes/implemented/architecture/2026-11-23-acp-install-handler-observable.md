# Agent Note: 能力闸门拒绝装配要可观测（install_* 返回 bool）+ h3 断言到错误码（B177 ⑤）

Status: implemented

## Problem

复审 r2 的 ADVISORY ⑤ 指出：`h3` 只覆盖 fs 一侧，且**只断言「有 error」不校验 `error.data.code`**。
顺着这条线看代码，问题比复审写的更具体：`install_fs_handler` / `install_terminal_handler` 的
闸门分支是

```rust
if !slots.declared.fs {
    log_json("warn", "acp 拒绝装配 FsHandler：…（fail-closed，不追认）", None);
    return;          // ← 签名是 ()，调用方拿不到任何信号
}
```

也就是说：**装配被拒是静默的**。唯一的痕迹是一条 stderr 警告；调用方（以及测试）无法区分
「装上了」和「被悄悄拒了」。这让 h3 那类"未声明能力不得追认"的契约只能靠"事后发请求看有没有 error"
间接验证，红相（真被追认了长什么样）与绿相之间的差别不够锐利。

## Decision

1. **`install_fs_handler` / `install_terminal_handler` 返回 `bool`**（`client.rs` 与 `session.rs`
   两层同时改，session 层是透传）：`true` = 装配成功，`false` = 被闸门拒绝。生产侧调用点
   （`session.rs` 的三处装配）无需改动即可编译——但语义从"无声"变成"可判断"。
2. **h3 用例把断言提到错误码层**，并直接用上返回值：
   - `assert!(!session.install_fs_handler(...))` —— 未声明 fs 时装配必须被拒
   - `response["error"]["code"] == -32601` —— 必须以 JSON-RPC `method not found` 拒绝
   - `response["error"]["data"]["code"] == "acp_unavailable"` —— 负载必须带可降级的 code
   （这三条都在既有"有 error / 无 result / 不泄漏 TOP-SECRET"之外新增，属加强而非替换。）

## Verification

- `bash scripts/cargo-lane.sh test -p pony-agent-core --test acp_config_regressions`
  → **8 passed / 0 failed**（含强化后的 `h3_installed_handler_cannot_enable_undeclared_capability`）
- 未跑全量 lib 单测：当前会话测试约束只允许 `--test <单个解耦测试>`；本改动触及的两个函数
  由该集成文件与 `acp_capabilities_contract` 覆盖。

## 未收口的部分（诚实标注）

复审 ⑤ 还要求 **terminal 侧的对偶反例**（`install_terminal_handler` 被拒 + 入站 `terminal/create`
回 `-32601`）。这需要 fake agent 夹具新增一条"未声明 terminal 时仍发 terminal/create"的探针，
属改 `fixtures/acp_fake_agent.sh` + 新增用例的独立工作，本 PR 未做——已在 B177 账本里留明确待办。

## Alternatives considered

- **只加错误码断言、不改签名**：h3 能变强，但"装配被静默拒绝"这个事实仍然不可观测；
  下一个人排查"为什么 handler 没生效"仍然只能去翻 stderr 日志。
- **把 `install_*` 改成返回 `Result<(), AcpConfigError>`**：调用方要处理一个"预期内"的拒绝，
  多数场景会写 `let _ =`，反而更啰嗦；`bool` 对"成没成"这个二元事实已经够用。
- **给返回值加 `#[must_use]`**：会迫使所有生产调用点显式处理，而生产路径（握手已声明才装配）
  本来就不会拒绝，加 must_use 只是制造噪声。
