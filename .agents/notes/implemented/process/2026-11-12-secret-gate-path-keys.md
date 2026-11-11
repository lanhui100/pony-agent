# Agent Note: 密钥门 64-hex 规则对「文件路径键」摘要值的系统性误报收口

Status: implemented

上游决策：[`2026-11-09-secret-gate-snake-case-digest-keys.md`](2026-11-09-secret-gate-snake-case-digest-keys.md)（豁免 5「结构化摘要槽」）；
缺陷账本：`backlog` B082。

## Problem

`node scripts/check-secrets.mjs --all` 在干净 HEAD 上恒红，2 条违规全部来自已入仓的 L2 验收收据
`.dev-team/verify-b064b-1794391769.json`：

```text
"crates/pony-agent-core/src/agent/sqlite_session.rs": "60848a8f...eb66"
"crates/pony-agent-core/src/agent/session/store.rs":  "30a56b1a...1296"
```

这是 `files_sha256` —— 「文件路径 → sha256」完整性映射，收据哈希是证据不是凭据，既不可轮换也不构成泄漏。
后果不止门禁本身：

- `tests/check-secrets.spec.ts` 的「全仓 0 违规」用例恒红（52 例中 1 例）；
- `tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts` 的 V10-3（实跑全量单测 exit 0）随之恒红；
- 判据面分裂：`npm run verify` 走 `--staged` 只判暂存面，本地可绿而全仓扫描红。

根因在 ADR 0111 的豁免 5 实现：`CRED_KEY_STEM_RE` 对**键名**做凭证词干黑名单，
`.../agent/session/store.rs` 因路径段含 `session` 被判定为凭证键、逐出摘要槽。
路径段撞词纯属目录名巧合，不构成「键名声明此处存放凭证」的语义。

## Decision

新增**豁免 6（路径键值槽）**，在 64-hex 规则的判定链中位于 `credIdx` 守卫**之前**：

1. 常量 `PATH_LIKE_KEY_RE = /[\\/][^\\/]*\.[A-Za-z0-9]+$/`——键必须**同时**含路径分隔符
   且末段带扩展名，才判为路径键。`hexLineFacts` 预计算 `pathKeySlots`（路径键的值槽下标集合）。
2. 判定顺序变为 `豁免 6 → R1′ credIdx 守卫 → 豁免 1 → 豁免 2/3 → 豁免 4 → 豁免 5`。
   必须先于 `credIdx`：路径段撞词会让凭证语义词落在 hex 之前，正是 B082 的现场。
3. 只放宽**路径键**，不放宽非路径凭证键：`"session"` / `"client_token"` / `"api_key"` /
   `"apikey"` / `"passphrase"` / `"signature"` 仍判违规；仅含 `/` 而无扩展名的键
   （`a/client_token`、`https://host/v1/token`）不视为路径键，仍判违规。
4. 收据本身**一字不改**——为迁就门禁改写证据等于伪造收据（与 ADR 0111 同一立场）。

## Alternatives considered

- **给 `.dev-team/verify-*.json` 加路径级豁免**（扩展 `isExemptGeneratedReceipt` 的匹配式）：落选。
  误报只是被赶出这一个文件名前缀；下一个 writer 用别的收据名（`l2at-*` 已先例）即复现，
  且路径级豁免会让该类文件里**真凭证**永久失明。
- **给收据里的哈希加 `sha256:` 前缀或改为截断哈希**：落选。收据是验收证据，格式由验收脚本决定，
  为过门禁改证据格式是本末倒置；截断哈希还会让完整性校验失去意义。
- **把 `session` 从 `CRED_KEY_STEM_RE` 词表里删掉**：落选。`"session": "<64hex>"` 是真实凭证形态
  （会话 token），删词会让真阳性漏报，用词表打补丁修的是症状不是病因。
- **仅按「键含 `/`」判路径键**：落选。`https://host/v1/token`、`v1/token` 也含 `/`，
  放行后这些 URL/路径式**凭证键**会被豁免，比现状更危险；故追加「末段带扩展名」的收窄条件。

## Consequences

- 已知边界（诚实登记）：文件名本身含凭证词干的路径键（`src/secret.rs`）同样被豁免。
  这类键的 64-hex 值在收据语义下仍是摘要而非凭据，风险可接受；若将来出现「路径键下挂真凭证」
  的形态，需在本条追加判据而非回退豁免 6。
- 新增用例 `tests/check-secrets.spec.ts` 的「路径键不是凭证键」块（P1–P9）：
  P1–P4 锁定路径键 0 命中（含 Windows 反斜杠），P5–P8 锁定防过度豁免，P9 端到端锁定 `.dev-team` 收据 0 违规。
- 红相基线：移除 `pathKeySlots` 放行行后 P1–P4 / P9 与「全仓 0 违规」共 3 例转红，恢复后 55 例全绿。
