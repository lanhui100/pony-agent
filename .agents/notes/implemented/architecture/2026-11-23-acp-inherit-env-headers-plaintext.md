# Agent Note: 把「env 明文」与「headers 原样透传」写进 ACP inherit 的契约文档（B177 ⑩）

Status: implemented

## Problem

复审 r2 的 ADVISORY ⑩：`resolve_mcp_servers` 的文档只写了「env 已完成 `${env:VAR}` 解析」，
**没说解析后是明文**；而显式条目的 `http` / `sse` `headers` 在转换里被原样 `clone()` 透传
（`config.rs` 的两条 `AcpMcpTransportSpec::Http/Sse` 分支），文档的字段映射清单里**一个字都没提**。

后果不是"少写一段文档"，而是**责任边界没划**：

- 调用方看到「已完成解析」，很容易以为"解析过=安全"，把 spec 直接 `{:?}` 打进日志或落盘；
  实际拿到的是密钥**明文值**。
- `headers` 更是典型：写配置的人以为 Pony 会做 header 白名单或脱敏，实际什么都没有——
  把 `Authorization: Bearer <长期密钥>` 塞进去，它就原样发给外部 agent，并可能出现在日志里。

## Decision

在 `resolve_mcp_servers` 的 rustdoc 里新增「明文与 headers 的两条硬事实」小节，明确：

1. **`${env:VAR}` 在加载期就被替换成明文**，转换后的 spec 保存的是值、不保留变量引用；
   本层不做二次屏蔽、也不知道哪个值敏感 ⇒ **任何把 spec 落盘 / 写日志 / 上报的调用方必须自行脱敏**
   （现有实现只对 deny 表命中的键 fail-closed，不覆盖其余明文）。
2. **`http` / `sse` 的 `headers` 原样透传**：不清洗、无 header 白名单、日志不抹除 ⇒ headers 里
   不得放长期凭据；需要密钥时应在外部 agent 侧用它自己的配置引用环境变量。

两条都写明**谁负责**（本层只做 deny 表 fail-closed；其余靠调用方/配置方自律），而不是笼统写"注意安全"。

## Verification

- `bash scripts/cargo-lane.sh check -p pony-agent-core --lib` → Finished（exit 0）
- 文档与实现逐条对照：`${env:VAR}` 的替换发生在加载期（loader 侧），`headers` 在
  `AcpMcpTransportSpec::Http` / `::Sse` 两个分支里以 `headers.clone()` 原样进 wire 结构
- 纯文档改动，零行为变更

## Alternatives considered

- **在运行时做"敏感值脱敏"**：本层根本不知道哪个值是密钥（用户可以用任意变量名），
  猜名字做正则屏蔽既漏又误伤；把责任明确推给"要落盘/打日志的那一方"才成立。
- **只写一句「注意 env 是明文」**：读者知道有风险，但不知道**自己该做什么**；必须写到
  「谁脱敏、在哪脱敏、为什么本层不做」这一层。
- **给 headers 加白名单**：会破坏生态兼容（各家 agent 的 header 需求不同），且白名单本身
  挡不住"用户就是要传密钥"的场景——文档约束 + 配置侧自律才是可行解。
