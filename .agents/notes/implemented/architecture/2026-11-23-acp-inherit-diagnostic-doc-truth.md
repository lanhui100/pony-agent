# Agent Note: ACP inherit MCP 诊断文档与实现对齐（B177 ADVISORY ①）

Status: implemented

## Problem

`crates/pony-agent-core/src/acp/config.rs` 的 `resolve_mcp_servers` 文档写着：

> 文件级致命诊断（`invalid_json` / `invalid_root` / `unsupported_schema_version` /
> **`invalid_mcp_servers`** 等，`server == None`）直接 fail-closed 报错

但同一函数的实现里，致命判定是：

```rust
let fatal_code = matches!(
    diagnostic.code.as_str(),
    "invalid_json" | "invalid_root" | "unsupported_schema_version"
);
```

`invalid_mcp_servers`（`mcpServers` 不是对象）**不在**致命集合里——它只是 push 一条诊断然后把
server 列表降级为空。文档把「降级」说成「致命」，是复审 r2 的 ADVISORY ①。

这类错误的危害不在于行为，而在于**方向性误导**：读文档的人会以为"继承的 MCP 配置一旦形状不对，
整个 delegate 就会失败"，于是要么在别处加多余的防御，要么反过来把实现改成"真的致命"来"对齐文档"。

## Decision

**按实现真值改正文档，不改任何行为**：

1. 致命集合在文档里收敛为三个 code（`invalid_json` / `invalid_root` / `unsupported_schema_version`），
   并显式写出「其余一律忽略 + 记账，降级为空列表继续」；
2. 补上**理由**（不只写结论）：`.mcp.json` 随仓库分发，把它升级为致命 ⇒ 一份结构不合规的
   `.mcp.json` 会让**所有** `acp_delegate` 一起失败；
3. 留一行**历史注记**说明这条曾误列 `invalid_mcp_servers`（B177 ①），避免下一个人以为文档从没变过。

## Verification

- `bash scripts/cargo-lane.sh check -p pony-agent-core --lib` → Finished（exit 0）
- 文档列举的三个致命 code 与实现 `fatal_code` 的 `matches!` 分支逐项一致；
  `invalid_mcp_servers` / `workspace_mcp_config_untrusted` 明确归入「降级」一侧

## B177 剩余清单（本 PR 只收第 ① 条）

② `acp-inbound` 分发线程不 join；③ 入站分发仍单线程串行（terminal/output 被 wait 压后）；④ h4「不饿死」
断言结构性不可观测；⑤ h3 仅覆盖 fs、`install_*` 拒绝无返回值断言；⑥ `read_slice` 的 `NotUtf8` 死分支；
⑦ `canonicalize → metadata → open` 的 TOCTOU 残留；⑧ wait 预算硬编码与 `set_turn_budget` 解耦；
⑨ `ManagedTerminalHandler::new` 无 workspace 绑定；⑩ inherit 的明文 env 与 http/sse headers 未文档化。

## Alternatives considered

- **改实现，让 `invalid_mcp_servers` 真变成致命**：会让一份坏 `.mcp.json` 否决整条 ACP 能力，
  是复审里明确否掉的方向（B2 的既有裁决）。
- **只删掉 `invalid_mcp_servers` 四个字、不写理由**：删完读者仍不知道"为什么它不致命"，
  下一个人很可能凭直觉把它加回致命列表——结论要让位给理由。
- **把这条 ADVISORY 标成「文档问题，不值得记录」直接跳过**：它的成本只有 20 分钟，
  而留在账本里的每一条 ADVISORY 都会在下一次复审里被重新提出一次。
