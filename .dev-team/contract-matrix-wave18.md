# wave-18 契约冻结矩阵（T1 / L2-P 协议特派员）

- 任务：`task-1`（wave-18 ① 凭证清理 + 扫描器补盲 + 看板修正）
- Owner：`protocol-agent`（L2-P）
- 波次 revision：**wave18-r2（冻结；r1 → r2 新增 C5 前端默认零 egress + 两条 Lead 裁定登记，变更原因见 §9）**
- 冻结时间基准：本文件写入时的工作树状态（HEAD 见 §0.3）
- 写域：仅 `.dev-team/`（只读侦察 + 只写 `.dev-team/`，未改任何业务代码/测试）

---

## 0. 证据方法（全部为亲自 read/grep/run 的结果，行号真实）

### 0.1 侦察命令与物理收据

| # | 命令 | Exit | 结论 |
|---|---|---|---|
| E1 | `read crates/pony-agent-core/src/agent/ponysentry/config.rs`（58 行全文） | 0 | 泄漏点确认：`:16` 硬编码 64-hex；`:17` `enabled: true`；`:34` 回落同一 token；`:36-38` `enabled` 默认 true |
| E2 | `grep -n "enabled" crates/pony-agent-core/src/agent/ponysentry/client.rs` | 0 | 命中 `:32`（sender 选择）与 `:91,125,175,193`（四处早退） |
| E3 | `read scripts/check-secrets.mjs`（314 行全文） | 0 | `SECRET_RULES` 7 条（`sk-`/`ghp_`/`gho_`/`AKIA`/`glpat-`/`xox`/`PRIVATE KEY`），**无裸 hex 规则**；`scanTextContent` 对 `check-secrets.spec.ts` 按文件名整体豁免 |
| E4 | `git ls-files \| xargs grep -nIE '[0-9a-fA-F]{64}'` | 0 | 全库 64-hex 面：`Cargo.lock` 662 处 + `src-tauri/Cargo.lock` 470 处（均为 `checksum = "..."`）；非 lockfile 命中见 §0.2 |
| E5 | `node -e 'import("./scripts/check-secrets.mjs").runChecks({mode:"all"})'` | 0 | **`unpatched_gate_all_violations=0`** → 现有门禁对真实泄漏完全失明（补盲根因物理收据） |
| E6 | `node /tmp/w18-rule-probe.mjs {sha,spec,repo,full}` | 0/0/0/0 | 候选规则四项探针全绿（详见 §4.4，脚本 `/tmp/w18-rule-probe.mjs` 为只读探针，未写入仓库） |
| E7 | `ls -la /home/dm/.agents/skills/dev-team/ /home/dm/.claude/skills/dev-team/` | 0 | 两个 skill 包内**只有 `SKILL.md`**（+ 一份 `SKILL.md.bak`），无 `dev-stub-lint.py` / `dev-red-purity.py` → 见 `.dev-team/wave18-tooling-deviation.md` |
| E8 | `grep -rn "PONYSENTRY_CLIENT_TOKEN\|client_token\|CLIENT_TOKEN" --include=*.rs/ts/vue/mjs/json` | 0 | 旧契约消费者全清单 → §8 迁移清单 |
| E9 | `read src/lib/telemetry.ts`（296 行全文） | 0 | C5 证据：`:10-12` INGEST_URL 硬编码回落、`:13-15` CLIENT_TOKEN 硬编码回落、`:266-274` IPC 分支、`:276-291` fetch 分支、`:280-282` token 头 |
| E10 | `cat vitest.config.ts` + `grep -rn acceptance Cargo.toml crates/*/Cargo.toml src-tauri/Cargo.toml` | 0 / **1** | R2 孤儿文件证据：`vitest.config.ts:20` include 仅 `tests/**/*.spec.ts\|*.test.ts`；全仓 Cargo.toml **零** `acceptance` 命中（grep EXIT=1）；`Cargo.toml:2-5` workspace members 仅 `crates/pony-agent-core`、`src-tauri` |
| E11 | `grep -rn "INGEST_URL\|CLIENT_TOKEN\|from .*telemetry" src tests` | 0 | C5 调用方面：外部仅 import 函数 `reportError`/`addBreadcrumb`（`src/stores/runtime.ts:3`、`src/lib/tauri.ts:3`、`src/main.ts:6`、`tests/unit/telemetry-suppression.spec.ts:2`）；`INGEST_URL`/`CLIENT_TOKEN` **无外部导入方**（仅 telemetry.ts 内部 `:280-284` 使用） |
| E12 | `read tests/unit/telemetry-suppression.spec.ts`（68 行全文） | 0 | C5 测试面：`:10-21` 已断言 vitest 下 `fetch`/`safeInvoke` 零调用（由 `:254-260` test-mode 静默守卫达成）→ C5 新测试必须区分机制，见 §5.4 |

### 0.2 全库 64-hex 面（`git ls-files`，排除 `.dev-team/` 与 lockfile）

| 文件 | 处数 | 上下文性质 |
|---|---|---|
| `crates/pony-agent-core/src/agent/ponysentry/config.rs` | 2 | **真实凭证泄漏（硬编码 client token）** |
| `crates/pony-agent-core/tests/ponysentry_telemetry_test.rs` | 1 | **同一 token 的测试断言副本（旧契约）** |
| `src/lib/telemetry.ts` | 1 | **同一 token 的前端回落副本（Lead 侦察未覆盖，本次新发现）** |
| `docs/architecture/ponysentry-integration-report.md` | 1 | `"fingerprint"`（事件指纹，非凭证） |
| `docs/release/2026-10-16-wave17-ime-guard-numeric-jitter/NFR-COMPLIANCE.md` | 1 | `sha256`（截图校验和） |
| `obscura-headless-browser-research.md` | 1 | `images/sha256-...`（Docker Hub SRI） |
| `skills-lock.json` | 1 | `"computedHash"`（技能锁哈希） |
| `Cargo.lock` / `src-tauri/Cargo.lock` | 662 / 470 | `checksum = "..."`（crates.io sha256） |

### 0.3 基线锚点

- 泄漏 token 全库出现文件（`git ls-files` 逐文件 grep 同一字面量）：**仅 3 个文件 4 处** —— `config.rs:16`、`config.rs:34`、`ponysentry_telemetry_test.rs:184`、`src/lib/telemetry.ts:15`。
- 本文件内该 token 一律以掩码 `6aa12e9e…c5d24` 形式引用（64-hex 首 8 位 + 末 5 位），**不得在任何新增产物中写出完整字面量**（否则新规则会自我拦截）。

---

## 1. C1 契约：`PonySentryConfig::default()`

| 字段 | 当前态（证据） | 目标态（冻结） | 不变/变更 |
|---|---|---|---|
| `endpoint` | `config.rs:15` `"https://sentry.ponyjob.top"` | 同左 | **不变** |
| `client_token` | `config.rs:16` `Some("6aa12e9e…c5d24")`（硬编码 64-hex） | `None` | **变更** |
| `enabled` | `config.rs:17` `true` | `false` | **变更** |
| `environment` | `config.rs:18` `"dev"` | 同左 | **不变** |
| `release` | `config.rs:19` `env!("CARGO_PKG_VERSION")` | 同左 | **不变** |

**冻结断言（T2 实施后必须成立）**

1. `PonySentryConfig::default().client_token.is_none()`
2. `PonySentryConfig::default().enabled == false`
3. `endpoint == "https://sentry.ponyjob.top"`、`environment == "dev"`、`release == env!("CARGO_PKG_VERSION")`
4. `config.rs` 内**不得残留任何 64-hex 字面量**（由 C4 新规则机械保证）
5. 结构体字段签名 `pub client_token: Option<String>`（`config.rs:6`）**不变** —— 调用方 `client.rs:364`、`mod.rs:64` 无需改类型

---

## 2. C2 契约：`PonySentryConfig::from_env()`

| 行为 | 当前态（证据） | 目标态（冻结） |
|---|---|---|
| `client_token` 来源 | `config.rs:31-34`：`PONYSENTRY_CLIENT_TOKEN` 未设/空白时 `.or_else(\|\| Some("6aa12e9e…c5d24"))` **回落硬编码** | 仅 `PONYSENTRY_CLIENT_TOKEN`（`.ok().filter(非空白)`）；**未设 => `None`；禁止任何回落** |
| `enabled` 未设时 | `config.rs:38` `.unwrap_or(true)` => **true** | `.unwrap_or(false)` => **false** |
| `enabled` 真值判定 | `config.rs:36-37` `trim().to_lowercase() != "false" && trim() != "0"` | **同左，保留**：仅显式非 `"false"`/`"0"` 才为 `true`（`"true"`/`"1"`/任意其他非空值 => true） |
| `endpoint` | `config.rs:26-29` `PONYSENTRY_INGEST_URL`，空白回落 `https://sentry.ponyjob.top` | **不变** |
| `environment` | `config.rs:40-43` `APP_ENV`，空白回落 `"dev"` | **不变** |
| `release` | `config.rs:45-48` `APP_RELEASE`，空白回落 `env!("CARGO_PKG_VERSION")` | **不变** |

**冻结断言**

1. `PONYSENTRY_CLIENT_TOKEN` 未设时 `from_env().client_token.is_none()`（**反向断言**：不得为 `Some(_)`）
2. `PONYSENTRY_CLIENT_TOKEN=""`（空白）时同样 `is_none()`
3. `PONYSENTRY_ENABLED` 未设时 `from_env().enabled == false`
4. `PONYSENTRY_ENABLED="false"` / `"0"` / `"FALSE"`（含首尾空白）=> `false`
5. `PONYSENTRY_ENABLED="true"` / `"1"` => `true`
6. `PONYSENTRY_CLIENT_TOKEN="test-token-123"` => `Some("test-token-123")`（既有断言保留，`ponysentry_telemetry_test.rs:195`）

---

## 3. C3 契约：`client.rs` 四处 enabled 早退（**沿用，不得改动**）

| 站点 | 证据 | 语义 | 冻结 |
|---|---|---|---|
| `capture_error` | `client.rs:91-93` `if !self.config.enabled { return; }` | 禁用时零 egress | **不得改动** |
| `capture_panic` | `client.rs:125-127` 同上 | 同上 | **不得改动** |
| `capture_payload` | `client.rs:175-177` 同上 | 同上 | **不得改动** |
| `capture_agent_trace` | `client.rs:193-195` 同上 | 同上 | **不得改动** |
| （关联）sender 构造 | `client.rs:32-44` `let sender = if config.enabled { ... } else { None }` | `enabled=false` 时不 spawn worker 线程、不建 channel | **不得改动** |
| （关联）worker 侧 token 头 | `client.rs:364-366` `if let Some(ref token) = config.client_token { req.header("X-Client-Token", token) }` | token 为 `None` 时不带鉴权头 | **不得改动** |

**推论（fail-closed 行为变更，T2/验收必须知情）**

- `mod.rs:69-71`：`send_agent_trace_direct` 在 `!enabled` 时返回 `Err("PonySentry is disabled")`。默认 `enabled=false` 后，**手动上传路径默认关闭**，须显式 `PONYSENTRY_ENABLED=true` 才可用 —— 这是本波期望的 fail-closed 语义（默认零 egress）。
- 即使 `enabled=true` 但 token 为 `None`，请求将**不带 `X-Client-Token`** 发出，服务端应拒绝；契约上要求 token 只能来自环境变量。

---

## 4. C4 契约：`scripts/check-secrets.mjs` 新增裸 64-hex 高熵规则

### 4.1 导出面兼容（**不得破坏**，证据 `tests/check-secrets.spec.ts:2-7`）

```ts
import { scanTextContent, checkBlockedFilename, isSafeFilename, SECRET_RULES } from "../scripts/check-secrets.mjs";
```

冻结：`SECRET_RULES`（`check-secrets.mjs:20`）、`BLOCKED_FILE_PATTERNS`（`:87`）、`isSafeFilename`（`:98`）、`checkBlockedFilename`（`:106`）、`scanTextContent`（`:118`）、`runChecks`（`:187`）**导出名与签名一律不变**；新规则以 **append 到 `SECRET_RULES` 末尾** 的方式加入（保持既有 7 条规则的匹配顺序与优先级，避免影响 `spec:33/52/62/69/80` 的 `rule` 文案断言）。

### 4.2 冻结规则实现（逐字交付给 T2）

```js
  {
    name: "Bare 64-Hex High-Entropy Credential (SHA-256 sized client token)",
    regex: /(?<![0-9a-fA-F])([0-9a-fA-F]{64})(?![0-9a-fA-F])/g,
    isSafe: (match, context = "") => {
      // 仅按“当前行”判定；scanTextContent 会以全量内容二次调用 isSafe，
      // 多行上下文一律不参与豁免（防止整文件级放行造成漏报）。
      if (context.includes("\n")) return false;
      const line = context;
      const lower = line.toLowerCase();
      // 1) 已知非凭证哈希上下文关键字（校验和 / 摘要 / 指纹 / SRI）
      if (/(sha-?256|sha-?512|checksum|integrity|digest|computedhash|fingerprint|pristine_proof|\bhash\b|校验和|指纹)/.test(lower)) return true;
      // 2) sha256sum(1) 输出格式：<hex><空白><路径>
      if (/^[0-9a-f]{64}\s+\S/.test(line.trim())) return true;
      // 3) Markdown 表格中的独立摘要单元格：| <hex> |
      if (/\|\s*[0-9a-fA-F]{64}\s*\|/.test(line)) return true;
      // 4) 反引号独占包裹的摘要（证据收据内联哈希）；同行出现凭证语义词时仍判违规
      const credWord = /(client[_-]?token|access[_-]?token|api[_-]?key|secret|password|passwd|credential|authorization|bearer)/;
      if (!credWord.test(lower) && line.includes("`" + match + "`")) return true;
      return false;
    }
  }
```

### 4.3 边界语义（冻结）

- 用 `(?<![0-9a-fA-F])` / `(?![0-9a-fA-F])` 而非 `\b`：既能命中 `token_<64hex>`（`_` 是 `\b` 的 word 字符，`\b` 会漏），又拒绝 65/128 位 hex 长串与 40 位 git SHA。
- 长度窗口严格 64：63/65/128 位一律不命中（见 §4.4 sha 探针）。
- 大小写 hex 均命中（`[0-9a-fA-F]`）。
- `scanTextContent` 对 `check-secrets.spec.ts` 的文件名整体豁免（`check-secrets.mjs:119-123`）**不得改动**；同时本规则自身对该文件内的样例串也零命中（§4.4 spec 探针在 **无文件名** 条件下运行，即最严苛路径）。
- `.env.example` 文件名豁免链路（`isSafeFilename` `:98-104` → `checkBlockedFilename` `:106-116`）**不得改动**。

### 4.4 零误报物理收据（探针 `/tmp/w18-rule-probe.mjs`，只读，未写仓库）

| 探针 | 覆盖 | 真实输出 | Exit |
|---|---|---|---|
| `sha` | git SHA 7/8/12/40 位、63/64/65/128 位、`_` 前缀 64 位、hex 字符前缀、大写 64 位 | `sha_cases_failures=0`（64 位命中、其余 0 命中） | **0** |
| `spec` | 复刻 `tests/check-secrets.spec.ts`（共 9 个 `it` 用例）中的 **8 条内容扫描断言**（sk 真实/mock-draft-test-placeholder、ghp、AKIA EXAMPLE/真实、glpat、私钥/mock 私钥）；第 9 个用例为文件名豁免（`isSafeFilename`/`checkBlockedFilename`），由 §4.3 的"不得改动"条款覆盖，本探针不含 | `spec_assertion_failures=0`（逐条 PASS，计数与 spec 期望完全一致） | **0** |
| `repo` | `git ls-files` 全量 1408 个可读文本文件（≤1MB、非二进制扩展名），仅候选规则 | `candidate_rule_hits=4` —— 精确等于 §0.3 的 4 个真实泄漏站点，**误报 0** | **0** |
| `full` | **端到端**：把候选规则 push 进真实模块的 `SECRET_RULES`，再调用真实 `runChecks({mode:"all"})` 代码路径（含文件名规则、spec 豁免、二进制/大小跳过） | `gate_rules_before=7 gate_rules_after=8`；`patched_gate_all_violations=4`（`config.rs:16`、`config.rs:34`、`ponysentry_telemetry_test.rs:184`、`src/lib/telemetry.ts:15`） | **0** |

**补盲前后对照（同一代码路径）**：E5 未打补丁 `violations=0`（失明） → `full` 打补丁 `violations=4`（全部真阳性，零误报）。

> 字段对齐：`.dev-team/nfr-baseline-wave18.json` 的 `wave18_secret_scan_gate.existing_spec_assertions_preserved = 8` 即上表 `spec` 探针的 8 条内容扫描断言（第 9 个 `it` 用例为文件名豁免）。

### 4.5 已知边界（诚实登记，非隐藏豁免）

- 豁免项 1/2/3/4 均为**行级**上下文判定。若某真实凭证被写在同时含 `sha256`/`hash` 等词的行上，会被豁免（漏报面）。判定优先级上，第 4 项已加"同行凭证语义词 => 仍判违规"的反向守卫，但 1/2/3 项未加。
- 第 4 项（反引号豁免）是本波为达成"当前树零误报"所必需：当前树有 6 处 `.dev-team/**` 证据收据把摘要写成 `` `<64hex>` ``（`contract-matrix-deadlock.md:282`、`wave17-red-receipts.md:16,17,18,19`），无此豁免则 `check-secrets --all` 在干净树上即 FAIL。
- 若 Lead/验收认为该豁免过宽，可收紧为"仅 `.dev-team/**` 路径生效"（把路径信息传入 `isSafe` 需扩签名，**会破坏导出面兼容**，故本波不采用）。

---

## 5. C5 契约：前端默认零 egress（`src/lib/telemetry.ts`）

> 来源：**Lead 裁定（wave18-r2）** + task-3 已生效要求。C5 与 C1/C2 同源同目标：**默认零 egress，凭证仅来自环境变量**。

### 5.1 当前态 → 目标态（逐行证据）

| 字段/分支 | 当前态（证据） | 目标态（冻结） |
|---|---|---|
| `INGEST_URL` | `telemetry.ts:10-12`：`(import.meta.env?.VITE_PONYSENTRY_INGEST_URL as string) \|\| "https://sentry.ponyjob.top"` —— **硬编码端点回落** | 仅取 `import.meta.env.VITE_PONYSENTRY_INGEST_URL`；**未设/空 => `""`**；**禁止任何硬编码回落** |
| `CLIENT_TOKEN` | `telemetry.ts:13-15`：`(import.meta.env?.VITE_PONYSENTRY_CLIENT_TOKEN as string) \|\| "6aa12e9e…c5d24"` —— **硬编码 64-hex 凭证回落（第三处泄漏点，F1）** | 仅取 `import.meta.env.VITE_PONYSENTRY_CLIENT_TOKEN`；**未设/空 => `""`**；**禁止任何硬编码回落** |
| `APP_RELEASE` | `telemetry.ts:16-17` `\|\| packageJson.version \|\| "0.1.118"` | **不变**（非凭证，不在 C5 范围） |
| `APP_ENV` | `telemetry.ts:18-20` `\|\| (import.meta.env?.DEV ? "dev" : "production")` | **不变** |
| `X-Client-Token` 头 | `telemetry.ts:280-282` `if (CLIENT_TOKEN) { headers["X-Client-Token"] = CLIENT_TOKEN; }` | **头名与语义不变**；空 token 时因 §5.2 守卫根本不会到达此处 |
| `fetch` 上报 | `telemetry.ts:284-291` `fetch(\`${INGEST_URL}/api/v1/ingest\`, ...)` | 仅在 §5.2 守卫通过后执行 |
| IPC 降级路径 | `telemetry.ts:266-274`：`isTauriAvailable()` → `safeInvoke("ponysentry_capture", { payload })`，失败 `catch` 后降级 fetch | **不得破坏**：结构、命令名、catch 降级语义一律不变 |

### 5.2 冻结实现形态（最小闭包，交付 Executor）

```ts
export const INGEST_URL =
  (import.meta.env?.VITE_PONYSENTRY_INGEST_URL as string) || "";
export const CLIENT_TOKEN =
  (import.meta.env?.VITE_PONYSENTRY_CLIENT_TOKEN as string) || "";
```

上报路径守卫，**必须置于 IPC 分支之后、`fetch` 之前**（即当前 `:274` 与 `:276` 之间）：

```ts
    // 2. 浏览器环境或 IPC 降级：直接 HTTP POST 到 Ingest
    //    C5：ingest/token 未配置（空串）时零 egress —— 不发 fetch、不带 X-Client-Token
    if (!INGEST_URL || !CLIENT_TOKEN) {
      return;
    }
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
    };
    if (CLIENT_TOKEN) {
      headers["X-Client-Token"] = CLIENT_TOKEN;
    }
```

**冻结约束**

1. `INGEST_URL`/`CLIENT_TOKEN` 的声明式回落一律改为 `|| ""`（或等价 `?? ""`）；**文件中不得再出现 `https://sentry.ponyjob.top` 作为这两个常量的回落值**。
2. 守卫条件必须是 `!INGEST_URL || !CLIENT_TOKEN`（**任一为空即 return**）。
3. **不得**把守卫提到 IPC 分支之前 —— 否则 Tauri 桌面端 IPC 上报路径被误杀，违反"IPC 降级路径不得破坏"。
4. `:280-282` 的 `if (CLIENT_TOKEN)` 写法**保留**（守卫后已恒真，冗余但零风险，属最小闭包）；头名 `X-Client-Token` 不得改。
5. 导出符号 `INGEST_URL`/`CLIENT_TOKEN`/`APP_RELEASE`/`APP_ENV` 与全部函数导出（`reportError`/`addBreadcrumb`/`buildIngestPayload`/`sanitizeString`/`deepSanitize`/`parseStackTrace`/`getBreadcrumbs`/`clearBreadcrumbs`/`shouldIgnoreTelemetryError`）**名称、类型、可见性一律不变**；**不新增导出符号**。
6. 既有 test-mode 静默守卫（`:254-260`）与 `shouldIgnoreTelemetryError` 探针过滤（`:249-251`）**不得改动**。

### 5.3 冻结断言

1. `src/lib/telemetry.ts` 源码**不含任何 64-hex 字面量**（由 C4 规则机械保证，§4.4 `repo`/`full` 探针命中点 `src/lib/telemetry.ts:15` 在迁移后必须消失）。
2. `VITE_PONYSENTRY_INGEST_URL` / `VITE_PONYSENTRY_CLIENT_TOKEN` 未设时：`INGEST_URL === ""` 且 `CLIENT_TOKEN === ""`。
3. 两者任一为空时：**`fetch` 调用次数为 0**、**不产生 `X-Client-Token` 头**。
4. IPC 路径回归：`isTauriAvailable() === true` 时 `safeInvoke("ponysentry_capture", ...)` 仍被调用（守卫不得前置）。
5. 迁移后 `node scripts/check-secrets.mjs --all` 与 C4 规则在 `src/lib/telemetry.ts` 上**零命中**。

### 5.4 非空洞测试要求（**关键，防零假测试**）

- **风险事实**：`tests/unit/telemetry-suppression.spec.ts:10-21` 已断言 vitest 下 `fetch` 零调用 —— 该结果由 `telemetry.ts:254-260` 的 **test-mode 静默守卫**独立达成。因此在 vitest 环境下，**"mock fetch 调用次数为 0" 这一条对 C5 是空洞断言**（即使 C5 未实现也会 PASS）。
- **冻结要求**：C5 测试必须包含**至少一条对 C5 具有变异敏感性**的断言，从下列三种任选（推荐全取）：
  1. **导出值断言**（最直接）：未设 env 时 `expect(INGEST_URL).toBe("")` 且 `expect(CLIENT_TOKEN).toBe("")` —— 若有人恢复硬编码回落，此断言必失败。
  2. **源码静态断言**：读取 `src/lib/telemetry.ts` 文本，断言不存在 `/[0-9a-fA-F]{64}/` 命中、且 `INGEST_URL`/`CLIENT_TOKEN` 的初始化表达式中不含 `"https://sentry.ponyjob.top"` —— 不受 test-mode 守卫混淆。
  3. **守卫直测**：在关闭 test-mode 守卫的条件下（例如经 `vi.mock("@/lib/telemetry")` 以外的等价手段，或对守卫提取出的纯谓词做单测）驱动上报路径，断言 `fetch` 零调用。
- **保留项**：`telemetry-suppression.spec.ts:10-21` 的既有断言**不得删改**（作为 test-mode 守卫的回归护栏，其证明对象是 `:254-260` 而非 C5）。
- 若 Test Agent 判定第 3 条在当前 Vitest 配置下不可稳定达成（`import.meta.env` 静态替换与 `vi.stubEnv` 的交互需实测），**必须**采用第 1+2 条并显式记录第 3 条"未采用 + 原因"，不得静默降级。

---

## 6. 本次新增发现（Lead 侦察未覆盖，需并入本波范围裁定）

| # | 位置 | 事实 | 影响 |
|---|---|---|---|
| **F1** | `src/lib/telemetry.ts:13-15` | 前端 `CLIENT_TOKEN` 同样硬编码同一 64-hex，作为 `VITE_PONYSENTRY_CLIENT_TOKEN` 未设时的回落；`:280-282` 把它写进 `X-Client-Token` 头；`:284` 浏览器侧 `fetch` 直发 `/api/v1/ingest` | **第三处泄漏点**。**Lead 已裁定纳入本波** → 冻结为 **C5（§5）**；C4 新规则已能命中（§4.4） |
| **F2** | `crates/pony-agent-core/tests/ponysentry_manual_upload_test.rs:189` | 明文数据库凭证：`postgresql://job_copilot:<已掩码>@127.0.0.1:30543/ponysentry` | 既有硬编码凭证（非 64-hex，C4 规则不覆盖）。**不在 task-1 写域**，登记移交 Lead 分诊 |
| **F3** | `tests/acceptance/ponysentry_telemetry_test.rs:177-198` | 与 `crates/pony-agent-core/tests/ponysentry_telemetry_test.rs:176-201` 的 `test_config_contract` **近乎重复的副本**（差异：副本无 token 断言、有 `assert!(default_cfg.enabled)` @`:182`） | 迁移必须**双份同步**，否则副本成为新的旧契约残留 |
| **F4** | `crates/pony-agent-core/tests/ponysentry_telemetry_test.rs:257` | `init(PonySentryConfig::default())` 位于 `test_breadcrumb_ring_buffer_limit` 末尾 | 默认 `enabled=false` 后该调用变为"禁用态 init"；该测试仅验证模块级函数不 panic（`:258-259`），语义仍成立，但断言强度下降，需 T2 评估是否补断言 |
| **F5** | `scripts/check-secrets.mjs` 长度窗口 | 已冻结规则为**严格 64 位**窗口（§4.3），未覆盖 32-hex（MD5 尺寸）凭证 | **Lead 裁定：32-hex 不作"必须命中"要求** → 登记为后续 backlog，见 §6.1 R1 |
| **F6** | `tests/acceptance/ponysentry_telemetry_test.rs` | 该 `.rs` 文件**既不被 cargo 编译**（全仓 Cargo.toml 零 `acceptance` 命中，grep EXIT=1；workspace members 仅 `crates/pony-agent-core`+`src-tauri`，`Cargo.toml:2-5`），**也不被 vitest 收集**（`vitest.config.ts:20` include 仅 `tests/**/*.spec.ts`/`*.test.ts`） | **孤儿死文件**（旧契约的未编译副本）。仍须迁移断言（防误导读者的陈旧契约，M4/M5），并登记为 dead-file backlog，见 §6.1 R2 |

### 6.1 Lead 裁定登记（wave18-r2）

**R1 — 32-hex 不作"必须命中"要求（Lead 裁定）**

- 裁定内容：C4 规则维持**严格 64 位窗口**（§4.3），**不**新增 32-hex 必命中规则。
- 理由（Lead）：补 32-hex 规则会与**零误报**目标冲突 —— MD5 尺寸/短哈希/UUID 片段在仓库中误报面大；而"零误报"是**绑定约束**（该门禁跑在 `npm run verify` 上，任何误报即阻塞全量自检）。
- 处置：登记为**后续 backlog 事项**，标注"**待独立评估长度窗口与豁免口径**"（须先给出仓库级 32-hex 误报面实测与豁免设计，再决定是否引入；不得在本波就地放宽 C4）。
- 本波状态：**C4 不变**（§4.2/§4.3 保持冻结，无需修改）。

**R2 — 孤儿死文件事实（Lead 已复核，本特派员独立复验）**

- 事实（双路证据）：
  1. **不被 cargo 编译**：`grep -rn "acceptance" Cargo.toml crates/*/Cargo.toml src-tauri/Cargo.toml` → **EXIT=1（零命中）**；`Cargo.toml:2-5` workspace `members = ["crates/pony-agent-core", "src-tauri"]`，仓库根 `tests/` 不是 package 目录，故 `tests/acceptance/ponysentry_telemetry_test.rs` 不进入任何编译目标。
  2. **不被 vitest 收集**：`vitest.config.ts:20` `include: ["tests/**/*.spec.ts", "tests/**/*.test.ts"]`；该文件扩展名为 `.rs`，不在收集面内。
- 性质：它是 `crates/pony-agent-core/tests/ponysentry_telemetry_test.rs` 旧契约的**未编译副本**（F3），既无编译期也无测试期的强制力，但会被读者误当现行契约。
- 处置：
  1. **本波仍须迁移其断言**（M4/M5）—— 目的不是让它变绿（它根本不运行），而是**消除陈旧契约的误导源**；
  2. 登记为 **dead-file backlog**：建议后续**删除**或**纳入真实编译目标**（二选一，需 Lead 决策；不得仅靠注释标注"未编译"了事）。
- 本波状态：不删除（不在 task-1/task-3 写域内，且删除须 Lead 决策）。

**R3 — 泄漏面复核（Lead `git ls-files` 复核，与本矩阵一致）**

- Lead 复核结论：泄漏 token 在**跟踪文件中恰好 3 个文件 4 处** —— `config.rs:16`、`config.rs:34`、`ponysentry_telemetry_test.rs:184`、`src/lib/telemetry.ts:15`，与 §0.3 一致。
- `.dev-team/` 内**不含**该 token；§4.5 豁免项 4 涉及的是**其他 sha256 摘要**（6 处证据收据行），豁免理由成立。

---

## 7. 影响面 / 非目标

- **非目标（不得改动）**：`client.rs` 全部早退与 sender 构造；`sanitizer.rs` 脱敏管道；`check-secrets.mjs` 既有 7 条规则、`BLOCKED_FILE_PATTERNS`、`isSafeFilename`、`scanTextContent` 的豁免分支、CLI 入口与退出码语义；`telemetry.ts` 的 IPC 分支（`:266-274`）、test-mode 静默守卫（`:254-260`）、探针过滤（`:249-251`）、脱敏管道（`:54-108`）、堆栈解析（`:143-191`）。
- **本波不引入**：结构化日志、新依赖、**新导出符号**、新环境变量名。
- **向后兼容**：`Option<String>` 与 `bool` 类型不变；`endpoint/environment/release` 取值与回落链不变；`from_env` 的真值解析表达式不变；前端全部导出符号名称/类型/可见性不变（C5 §5.2 约束 5）。
- **导出值变更（有意，C5）**：`INGEST_URL` 由 `"https://sentry.ponyjob.top"` 默认值改为 `""`；`CLIENT_TOKEN` 由硬编码 64-hex 默认值改为 `""`。**无外部导入方**（E11），故无调用方破坏面；前端行为变为"未配置 => 零 egress"。
- **运行时语义变更（有意）**：默认零 egress —— 后端 `enabled=false` 且无 token（C1/C2）；前端 `INGEST_URL`/`CLIENT_TOKEN` 未配置时不发 `fetch`（C5）。显式配置 `PONYSENTRY_ENABLED=true` + `PONYSENTRY_CLIENT_TOKEN`（后端）与 `VITE_PONYSENTRY_INGEST_URL` + `VITE_PONYSENTRY_CLIENT_TOKEN`（前端）才恢复上报。

---

## 8. 旧契约迁移清单（**移交 T2 执行**，逐条）

> 规则：T2 只能"迁移断言以匹配 §1/§2/§3/§5 冻结契约"，**不得放宽为恒真断言、不得删除测试、不得跳过（`#[ignore]`）**。

| # | 文件:行 | 现有旧契约断言 | 迁移方向（目标态） | 优先级 |
|---|---|---|---|---|
| M1 | `crates/pony-agent-core/tests/ponysentry_telemetry_test.rs:184-185` | `assert_eq!(default_cfg.client_token.as_deref(), Some("6aa12e9e…c5d24"))`（硬编码 token，同时是 C4 新规则的命中点） | 改为 `assert!(default_cfg.client_token.is_none())`；**删除硬编码字面量** | P0（阻塞：迁移前 `check-secrets --all` 必 FAIL） |
| M2 | `crates/pony-agent-core/tests/ponysentry_telemetry_test.rs:186` | `assert!(default_cfg.enabled)` | 改为 `assert!(!default_cfg.enabled)` | P0 |
| M3 | `crates/pony-agent-core/tests/ponysentry_telemetry_test.rs:176-201` | `test_config_contract` 未覆盖"未设 env 的默认路径"（测试内先 set 再断言，无未设分支） | 新增反向断言：清空 `PONYSENTRY_CLIENT_TOKEN`/`PONYSENTRY_ENABLED` 后 `from_env()` 得 `client_token.is_none()` + `enabled == false`；并补 `"1"`/`"true"`/`"0"`/`"FALSE"` 真值表断言 | P1 |
| M4 | `tests/acceptance/ponysentry_telemetry_test.rs:182` | `assert!(default_cfg.enabled)`（副本） | 同 M2；并评估与 M1/M2 的双份同步机制（F3） | P0 |
| M5 | `tests/acceptance/ponysentry_telemetry_test.rs:177-198` | 副本 `test_config_contract` 与主文件漂移（无 token 断言） | 与主文件对齐到同一目标契约（含 M3 的反向断言） | P1 |
| M6 | `crates/pony-agent-core/tests/ponysentry_manual_upload_test.rs:148` | `assert!(config.client_token.is_some(), "Default config must provide client token for sentry auth")` | 删除该断言；改为显式构造带 token 的 config（或 `PONYSENTRY_CLIENT_TOKEN` 注入）——**默认配置不再提供 token** | P0 |
| M7 | `crates/pony-agent-core/tests/ponysentry_manual_upload_test.rs:143-186`（`test_upload_trace_to_live_sentry_contract`） | 依赖 `default()` 的 token + 打**生产端点** `sentry.ponyjob.top`，断言 `201 CREATED`，并在 `:189` 内嵌明文 psql 凭证做清理 | 必须**去 live 网络化**：改为 local/mock ingest 或 `#[ignore]` + 显式 env 门控（**不得**默认打生产）；清理路径中的明文凭证（F2）随迁移一并上报 Lead | P0 |
| M8 | `crates/pony-agent-core/tests/ponysentry_telemetry_test.rs:257` | `init(PonySentryConfig::default())`（F4） | 保持不 panic 语义；如需保留"模块级函数可用"强度，改为 `init` 一个显式 `enabled: true` 的 config | P2 |
| M9 | `tests/check-secrets.spec.ts`（全 94 行） | 现有 8 条断言 + 文件名豁免断言 | **不改既有断言**；仅**新增**用例：命中 64-hex 字面量；不命中 7/8/12/40 位 git SHA；不命中 `checksum`/`sha256`/`computedHash`/`fingerprint` 上下文；`sk-draft/sk-test/sk-mock` 与 `.env.example` 豁免保持 | P1 |
| M10 | `tests/unit/telemetry-suppression.spec.ts:10-21` | 断言 vitest 下 `fetch`/`safeInvoke` 零调用（由 test-mode 守卫达成，**对 C5 空洞**，§5.4） | **不得删改既有断言**；**新增** C5 变异敏感断言：`INGEST_URL === ""` + `CLIENT_TOKEN === ""`（未设 env），并按 §5.4 第 2 条做源码静态断言（无 64-hex、无端点硬编码回落） | P0（C5 验收核心） |
| M11 | `tests/acceptance/ponysentry_telemetry_test.rs`（孤儿死文件，F6/R2） | 不被 cargo 编译、不被 vitest 收集；承载旧契约（`:182` `assert!(default_cfg.enabled)`） | 先按 M4/M5 迁移其断言（消除陈旧契约误导源）；再登记 **dead-file backlog**（删除 或 纳入真实编译目标，待 Lead 决策）。**不得**以"反正不编译"为由跳过迁移 | P1 |

**迁移验收硬要求（T2）**：
1. `npm test`（含 `tests/check-secrets.spec.ts`、`tests/unit/telemetry-suppression.spec.ts` + C5 新增断言）Exit 0；
2. `cargo test -p pony-agent-core --test ponysentry_telemetry_test`、`--test ponysentry_manual_upload_test` Exit 0；
3. `node scripts/check-secrets.mjs --all` 在迁移后 Exit 0（4 处泄漏清空后零命中）；
4. C5 变异敏感断言**必须存在**，且能被"恢复硬编码回落"这一反例证伪（Test Agent 需给出该反例的实测收据）。

---

## 9. 冻结签名与 revision 变更记录

### 9.1 冻结签名

- 本矩阵为 wave-18 的**唯一契约基线**（当前 revision `wave18-r2`）。
- 任何偏离（新增豁免、改变规则长度窗口、改 `client.rs` 早退、改导出面、改 C5 守卫位置）都必须走 Lead 裁定并升 revision，不得由实施者就地放宽。
- 配套产物：`.dev-team/nfr-baseline-wave18.json`、`.dev-team/nfr-baseline.schema.json`（additive-only）、`.dev-team/wave18-tooling-deviation.md`。

### 9.2 revision 变更记录

| revision | 变更 | 原因（Lead 裁定 / 依据） |
|---|---|---|
| `wave18-r1` | 冻结 C1（`default()`）、C2（`from_env()`）、C3（`client.rs` 四处早退沿用）、C4（`check-secrets.mjs` 裸 64-hex 规则）；旧契约迁移清单 M1–M9（r1 编号 §7，r2 顺延为 §8）；§0 证据 E1–E8 | task-1 初始交付 |
| `wave18-r2` | **新增 C5（§5，前端 `telemetry.ts` 默认零 egress）**；F1 由"待裁定"改判为"已纳入本波 → C5"；新增 §5.4 非空洞测试要求；新增 F5/F6 与 §6.1 裁定登记 R1（32-hex 不入必命中）/R2（孤儿死文件）/R3（泄漏面复核）；迁移清单新增 M10/M11 并强化验收（含反例可证伪要求）；新增证据 E9–E12；因插入 C5 顺延原 §5–§8 → §6–§9 | **Lead 裁定（wave18-r2）**：F1 第三处泄漏点纳入本波并冻结为 C5；登记 32-hex backlog 与孤儿死文件事实；同步 task-3 已生效要求 |
| — | **C4 规则本体（§4.2/§4.3）在 r2 中未做任何修改** | R1：零误报为绑定约束，不得为覆盖 32-hex 而放宽长度窗口 |
