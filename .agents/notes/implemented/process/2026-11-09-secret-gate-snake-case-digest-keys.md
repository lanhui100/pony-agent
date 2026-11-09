# Agent Note: 密钥门 64-hex 规则对 snake_case 摘要键的系统性误报收口

Status: implemented

配套决策：[ADR 0111](../../../docs/decisions/0111-secret-gate-snake-case-digest-keys.md)；
契约修订：`.dev-team/contract-matrix-wave18.md` §AMEND-6（正文 §4.2 原文保留并加修订指针）；
上游冻结基线：同文件 §4.2/§4.3（`wave18-r4`）。

## Decision

1. `BENIGN_RE` 的 `\bhash\b` → `(?<![a-z0-9])hash(?![a-z0-9])`；词表追加 `frozen_expected`、`冻结值`、`现值`。
2. 新增豁免 5「结构化摘要槽」：`hexLineFacts` 预计算 `digestSlots`（JSON/对象成员值起始下标 → 键名），
   凭证键（`CRED_KEY_STEM_RE` 正向黑名单）永不入表。
3. 判定顺序不变（`R1′ → 豁免 1 → 豁免 2/3 → 豁免 4 → 豁免 5`）。
4. 红相基线 + 端到端用例锁死：`tests/check-secrets.spec.ts` 新增 `ADR 0111` 描述块 S1–S13 / T1–T6。

## 为什么

`docs/audit/rounds/r1-dim6`、`r2-dim1` 是 trace 审核的历史取证轮，其 `round-state.json` /
`receipts/*.meta.json` 记录的是 `corpus_hash`、`assertion_hashes`、`freeze_impact.before/after`——
摘要收据，不是凭证。272 个未跟踪取证文件在 `npm run secrets:check:all` 下报 65 条违规、**零真阳性**，
直接后果是历史轮无法入库（硬提交会让全量自检变红）。

两条独立根因：

- `_` 属 `\b` 的 word 字符 ⇒ `corpus_hash` 里 `hash` 无词边界 ⇒ 良性词豁免对 snake_case `*_hash` 整类失明。
  §4.3 自己写过这条论证（用来说明 hex 边界为何不用 `\b`），但从未用到良性词边界上。
- 豁免 1–4 只认行内文本特征，不看 JSON 的「键 → 值」结构 ⇒ `"before"/"after"/"detail"/
  "scripts_assertions_dim6.sh"` 这类非摘要键的摘要值全部判违规。任何新收据 schema 都会再误报一轮。

## Alternatives considered

- **只做 `docs/audit/**` 路径级豁免**：落选。误报只是被赶出取证目录；`.dev-team/**` 收据今天靠反引号/单元格
  豁免侥幸没炸，换个 writer 不加反引号即复现。
- **只修词边界**：落选。实测残余 55 条误报，历史轮仍无法入库。
- **豁免 5 用反向判据「键名不匹配 `CRED_WORD_RE`」**：落选。`apikey`/`secretkey`/`passphrase` 不匹配
  `CRED_WORD_RE`（缺分隔符）⇒ 真凭证被当良性键放行，比现状更危险。
- **改写取证 JSON 的摘要格式（加 `sha256:` 前缀）**：落选。取证产物是证据，为迁就门禁改写证据等于伪造收据。
- **改 §4.2 冻结实现正文**：落选。该实现逐字交付过 Executor，改写等于篡改当时的契约事实；
  改为 §AMEND-6 追记（与 §AMEND-5 同惯例）。

## 机器收据

| 检查 | 结果 |
|---|---|
| `npx vitest run tests/check-secrets.spec.ts` | 50 passed（含新增 5 个 `it`） |
| 红相基线（HEAD 版实现跑 S1–S10） | 10/10 命中，修复后 0/10 |
| 取证目录 `docs/audit/rounds/**`（272 文件） | 违规 65 → **0** |
| `node scripts/check-secrets.mjs --all` | Exit 0 |
| 性能（HEAD → 修复） | 1MB 35→39ms；5MB 148→132ms；10MB 258→273ms（判据 2000/5000ms） |
| `node scripts/check-decisions.mjs` | 通过，110 篇 ADR |
