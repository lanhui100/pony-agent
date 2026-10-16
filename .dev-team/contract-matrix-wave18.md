# wave-18 契约冻结矩阵（T1 / L2-P 协议特派员）

- 任务：`task-1`（wave-18 ① 凭证清理 + 扫描器补盲 + 看板修正）
- Owner：`protocol-agent`（L2-P）
- 波次 revision：**wave18-r3（冻结；r1 → r2 新增 C5；r2 → r3 修订 C2/C4 + 新增 C6/C7/C8 + 12 项 findings 裁定，变更原因见 §12）**
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
| E8 | `grep -rn "PONYSENTRY_CLIENT_TOKEN\|client_token\|CLIENT_TOKEN" --include=*.rs/ts/vue/mjs/json` | 0 | 旧契约消费者全清单 → §11 迁移清单 |
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
| `enabled` 未设时 | r1 冻结为 `.unwrap_or(false)` => **false**（已由 Executor 落地，现 `config.rs:41-43`） | `.unwrap_or(false)` => **false**（**保留**） |
| `enabled` 真值判定 | `config.rs:41-43` `map(\|s\| s.trim().to_lowercase() != "false" && s.trim() != "0").unwrap_or(false)`：**`Some("")`/`Some("   ")` 落进 `map` 闭包 => `true`（fail-open 缺口）**；`.unwrap_or(false)` 只兜住 unset。实测反例见 §9.1 A9（`review-l3-contract` finding#4） | **r3 修订**：set-but-empty / 纯空白必须 => **false**；即 `""`、`"   "`、`"\t"` 与 unset 同义（与 C5 前端"空串按未设处理"对称） |
| `endpoint` | `config.rs:26-29` `PONYSENTRY_INGEST_URL`，空白回落 `https://sentry.ponyjob.top` | **不变** |
| `environment` | `config.rs:40-43` `APP_ENV`，空白回落 `"dev"` | **不变** |
| `release` | `config.rs:45-48` `APP_RELEASE`，空白回落 `env!("CARGO_PKG_VERSION")` | **不变** |

**冻结断言**

1. `PONYSENTRY_CLIENT_TOKEN` 未设时 `from_env().client_token.is_none()`（**反向断言**：不得为 `Some(_)`）
2. `PONYSENTRY_CLIENT_TOKEN=""`（空白）时同样 `is_none()`
3. `PONYSENTRY_ENABLED` 未设时 `from_env().enabled == false`
4. `PONYSENTRY_ENABLED="false"` / `"0"` / `"FALSE"` / `"False"` / `" 0 "`（含首尾空白与大小写）=> `false`
5. `PONYSENTRY_ENABLED="true"` / `"1"` / `"TRUE"` / `" true "` => `true`
6. `PONYSENTRY_CLIENT_TOKEN="test-token-123"` => `Some("test-token-123")`（既有断言保留，`ponysentry_telemetry_test.rs:195`）
7. **【r3 新增，fail-closed 边界】** `PONYSENTRY_ENABLED=""` => **`false`**；`PONYSENTRY_ENABLED="   "`（纯空白）=> **`false`**；`PONYSENTRY_ENABLED="\t"` => **`false`**
8. **【r3 新增】** `PONYSENTRY_ENABLED="  0  "` => `false`；`PONYSENTRY_ENABLED=" false "` => `false`（先 `trim` 再判，空白不得改变语义）
9. **【r3 新增，非空洞要求】** 断言 7/8 必须为**变异敏感**：若实现退回 `.map(...).unwrap_or(false)`（即 set-but-empty => true），断言 7 必须失败。

**冻结实现形态（r3，最小闭包）**

```rust
        // wave-18 C2（r3）：未设 / set-but-empty / 纯空白 ⇒ false（fail-closed 零 egress）；
        // 仅显式非 "false"/"0"（含大小写与首尾空白）才为 true。
        let enabled = std::env::var("PONYSENTRY_ENABLED")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_lowercase() != "false" && s != "0")
            .unwrap_or(false);
```

> 等价写法（任选其一，语义必须一致）：`std::env::var("PONYSENTRY_ENABLED").ok().map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).map(|s| s != "false" && s != "0").unwrap_or(false)`。
> **禁止**在 `filter` 之前返回 `true`；**禁止**用 `.unwrap_or(true)`。

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

### 4.2 冻结规则实现（**wave18-r3 修订版**，逐字交付给 Executor）

> r3 相对 r2 的三处修订：**R1** 豁免 1/2/3 加 `credWord` 守卫；**R2** 豁免 2/3 只豁免该 hex 本身（位置锚定），不得豁免整行；**R3** 消除 `O(matches × 行长)` 二次方（按行记忆化，详见 §4.6）。同时因 **C7** 把 `isSafe` 的 context 语义改为有界窗口（§7），本规则改为取窗口最后一行。

**模块级辅助（新增，非导出）**

```js
const HEX64 = "[0-9a-fA-F]{64}";
const CRED_WORD_RE = /(client[_-]?token|access[_-]?token|api[_-]?key|secret|password|passwd|credential|authorization|bearer)/;
const BENIGN_RE = /(sha-?256|sha-?512|checksum|integrity|digest|computedhash|fingerprint|pristine_proof|\bhash\b|校验和|指纹)/;
const SUMPREFIX_RE = new RegExp(`^\\s*(${HEX64})\\s+\\S`);
const CELL_RE = new RegExp(`\\|\\s*(${HEX64})\\s*\\|`);
const BACKTICK_RE = new RegExp("`(" + HEX64 + ")`", "g");

// R3：按「行」记忆化全部与 match 无关的上下文判定；有界缓存（≤4 条）防无界增长。
const _hexFactsCache = new Map();
function hexLineFacts(line) {
  const hit = _hexFactsCache.get(line);
  if (hit) return hit;
  const lower = line.toLowerCase();
  const facts = {
    credWord: CRED_WORD_RE.test(lower),
    benignKeyword: BENIGN_RE.test(lower),
    prefixHex: (SUMPREFIX_RE.exec(line) || [null, null])[1]?.toLowerCase() ?? null,
    cellHex: (CELL_RE.exec(line) || [null, null])[1]?.toLowerCase() ?? null,
    backtickHexes: new Set([...line.matchAll(BACKTICK_RE)].map((x) => x[1].toLowerCase())),
  };
  if (_hexFactsCache.size >= 4) _hexFactsCache.clear();
  _hexFactsCache.set(line, facts);
  return facts;
}
```

**规则本体（append 到 `SECRET_RULES` 末尾）**

```js
  {
    name: "Bare 64-Hex High-Entropy Credential (SHA-256 sized client token)",
    regex: /(?<![0-9a-fA-F])([0-9a-fA-F]{64})(?![0-9a-fA-F])/g,
    isSafe: (match, context = "") => {
      // C7：context 为「当前行 + 最多 3 行紧邻上文」的有界窗口；行级判定取最后一行。
      const line = context.split("\n").pop();
      const f = hexLineFacts(line); // R3：每行只算一次，之后 O(1)
      // R1：同行出现凭证语义词 => 一律判违规（优先于豁免 1/2/3/4）
      if (f.credWord) return false;
      if (f.benignKeyword) return true; // 豁免 1（已被 R1 守卫）
      const m = match.toLowerCase();
      if (f.prefixHex === m) return true;      // 豁免 2：只豁免 sha256sum 前缀位置的那个 hex（R2）
      if (f.cellHex === m) return true;        // 豁免 3：只豁免 Markdown 独立单元格内的那个 hex（R2）
      if (f.backtickHexes.has(m)) return true; // 豁免 4
      return false;
    }
  }
```

### 4.3 边界语义（冻结）

- 用 `(?<![0-9a-fA-F])` / `(?![0-9a-fA-F])` 而非 `\b`：既能命中 `token_<64hex>`（`_` 是 `\b` 的 word 字符，`\b` 会漏），又拒绝 65/128 位 hex 长串与 40 位 git SHA。
- 长度窗口严格 64：63/65/128 位一律不命中（见 §4.4 sha 探针）。**R1（r3）**：32-hex 仍不覆盖（§9.1 R1 backlog）。
- 大小写 hex 均命中（`[0-9a-fA-F]`）。
- **【r3 新增 R1】凭证语义词优先**：同一行（窗口最后一行）出现 `client_token`/`access_token`/`api_key`/`secret`/`password`/`passwd`/`credential`/`authorization`/`bearer` 任一（大小写不敏感）时，**豁免 1/2/3/4 全部失效**，该行所有 64-hex 一律判违规。
- **【r3 新增 R2】豁免 2/3 位置锚定**：豁免 2 仅对"该行首个 64-hex 且其后紧跟空白 + 非空白字符"的那个 hex 生效；豁免 3 仅对"被 `|` 包裹的独立单元格内"的那个 hex 生效。**一行内其他位置的 64-hex 不继承豁免**（防止第二个具名凭证被吞）。
- **【r3 新增】豁免 4** 保留 r2 语义（反引号独占包裹 + 同行无凭证语义词），改为 `Set` 成员判定（R3）。
- `scanTextContent` 对 `check-secrets.spec.ts` 的文件名整体豁免（`check-secrets.mjs:119-123`）**不得改动**；同时本规则自身对该文件内的样例串也零命中（§4.4 spec 探针在 **无文件名** 条件下运行，即最严苛路径）。
- `.env.example` 文件名豁免链路（`isSafeFilename` `:98-104` → `checkBlockedFilename` `:106-116`）**不得改动**；**C6** 新增的 `.env.example` 正是依赖该链路才不被文件名规则拦截（但其**内容**仍受 C4 规则约束，见 §6）。
- **【r3 修订】** `isSafe(match, context)` 的 `context` 语义由"当前行 或 全量文件内容"改为**"当前行 + 最多 3 行紧邻上文"的有界窗口**（C7，§7）。所有规则**禁止**依赖整文件内容做放行判定。

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

- **【r3 更新】** 豁免项 1/2/3/4 现**全部**受 `credWord` 守卫（R1）约束：同行出现凭证语义词即一律判违规，故 r2 记录的"1/2/3 项无守卫导致整行放行"缺口已在 r3 关闭（对应 findings #5/#6，见 §9.1 A1/A2）。
- 残余边界（**r3 仍存在，必须如实登记**）：若真实凭证所在行**不含**任何凭证语义词、却含 `sha256`/`hash` 等良性关键词，仍会被豁免 1 放行（例：`const k = "<64hex>" // checksum`）。这是"零误报优先"（门禁跑在 `npm run verify`）与"零漏报"之间的显式取舍；收紧需引入路径/语义白名单，属独立议题（与 R1 backlog 同批评估）。
- 第 4 项（反引号豁免）是本波为达成"当前树零误报"所必需：当前树有 6 处 `.dev-team/**` 证据收据把摘要写成 `` `<64hex>` ``（`contract-matrix-deadlock.md:282`、`wave17-red-receipts.md:16,17,18,19`），无此豁免则 `check-secrets --all` 在干净树上即 FAIL。
- 若 Lead/验收认为该豁免过宽，可收紧为"仅 `.dev-team/**` 路径生效"（把路径信息传入 `isSafe` 需扩签名，**会破坏导出面兼容**，故本波不采用）。

### 4.6 性能硬约束（**wave18-r3 新增，对应 finding#8**）

- **约束**：整行处理必须为 **O(行长)**；`isSafe` 中**一切与 match 无关**的上下文判定（`credWord`、良性关键词正则、sha256sum 前缀、Markdown 单元格、反引号集合）必须按行**预计算/记忆化一次**（§4.2 的 `hexLineFacts` + 有界缓存）。
- **禁止**：在 `isSafe` 内对**每个 match** 重复执行 `line.toLowerCase()`、`regex.test(lower)`、`line.includes(...)`、`line.trim()` 等整行级运算。
- **实测回归证据（r2 实现，独立复现）**：单行 4000 × 64-hex（259,999 B）= **2072.4 ms**；同样数据按行拆分为 4000 行 = **19.7 ms** → **105×** 差距（`/tmp/w18b-probe.mjs`，Exit 1）。评审路实测 1MB 单行 = **133,411.9 ms**（15000 命中）。
- **验收判据**：修复后同一 4000 × 64-hex 单行用例必须 ≤ **100 ms**（相对 19.7 ms 基线留 5× 余量），且命中数与拆行版本一致；CI secrets 步骤（`ci.yml:44` `--all`，无 step timeout）不得因长行停滞。
- **缓存有界性**：缓存必须**有界**（≤4 条并清空重填），禁止以行长/内容为键的无界 `Map` 造成内存泄漏。

### 4.7 r3 反例回归用例（**必须命中**，对应 findings #5/#6）

以下 6 条在 r2 实现下**全部 0 命中**（已独立复现，`/tmp/w18b-probe.mjs` Exit 1），r3 修复后**必须 ≥1 命中**；同时 §4.4 的 `sha`/`spec` 探针必须保持全绿（零误报不得回退）：

| # | 输入形态（`H` = 64-hex 字面量） | r2 实测 | r3 要求 |
|---|---|---|---|
| G1 | ``| PONYSENTRY_CLIENT_TOKEN | H |`` | 0 | ≥1（R1 守卫豁免 3） |
| G2 | ``| client_token | H |`` | 0 | ≥1（R1） |
| G3 | `{"client_token":"H","integrity":"sha512-x"}` | 0 | ≥1（R1 守卫豁免 1） |
| G4 | `{"client_token":"H","checksum":"abc"}` | 0 | ≥1（R1） |
| G5 | `client_token=H // hash` | 0 | ≥1（R1） |
| G6 | `H  PONYSENTRY_CLIENT_TOKEN=H` | 0 | ≥1（R1 + R2：第二个 hex 不得被 sha256sum 前缀豁免吞掉） |

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

## 6. C6 契约：部署配置模板 `.env.example`（**wave18-r3 新增**）

> 来源：`review-l3p-wave18.json` finding#2（Q5 配置与自检）。证据：`ls .env.example` → `No such file or directory`（EXIT=2）；`git ls-files | grep -E '^[.]env'` → 零命中（EXIT=1）；全仓 `PONYSENTRY_ENABLED` 26 行命中**全部落在消费方与测试**，零条面向部署方。

### 6.1 冻结要求

1. **新增仓库根 `.env.example`**（被跟踪文件）。文件名落在既有豁免链路 `isSafeFilename`（`check-secrets.mjs:98-104`）内，故不会被 `checkBlockedFilename` 拦截。
2. 必须逐项记录以下 5 个变量的**用途 / 默认值（未设 => fail-closed 零 egress）/ 示例占位值**：

| 变量 | 用途 | 未设时行为（默认值） | 归属契约 |
|---|---|---|---|
| `PONYSENTRY_INGEST_URL` | 后端 ingest 端点 | 回落 `https://sentry.ponyjob.top`（非凭证，允许保留） | C2 |
| `PONYSENTRY_CLIENT_TOKEN` | 后端鉴权 token | **`None`**（不发 `X-Client-Token`） | C2 |
| `PONYSENTRY_ENABLED` | 后端上报开关 | **`false`**（含 set-but-empty / 纯空白） | C2（r3 修订） |
| `VITE_PONYSENTRY_INGEST_URL` | 前端 ingest 端点 | **`""`**（不发 fetch） | C5 |
| `VITE_PONYSENTRY_CLIENT_TOKEN` | 前端鉴权 token | **`""`**（不发 fetch、不带头） | C5 |

3. 必须写明**恢复上报的最小配置**：后端 `PONYSENTRY_ENABLED=true` + `PONYSENTRY_CLIENT_TOKEN=<...>`；前端 `VITE_PONYSENTRY_INGEST_URL=<...>` + `VITE_PONYSENTRY_CLIENT_TOKEN=<...>`。
4. 必须写明**自检方式**：`node scripts/check-secrets.mjs --all`（并在文件内注明"该命令必须 Exit 0"）。
5. **模板内容硬约束（与 C4 交互）**：不得包含任何 64-hex 字面量；示例值一律用 `<...>` 占位或 `sk-test-`/`sk-mock-` 前缀（既有安全豁免），**不得**写真实凭证或形似真实的 64-hex。
6. 不得把 `.env.example` 加入 `.gitignore`；不得改 `isSafeFilename` 白名单。

### 6.2 冻结断言

1. `.env.example` 存在且被 `git ls-files` 跟踪。
2. 文件内含上表 5 个变量名（`grep -c` 各 ≥1）。
3. 文件内**零** 64-hex 字面量；且 `node scripts/check-secrets.mjs --all` Exit 0。
4. 文件内含字符串 `node scripts/check-secrets.mjs --all`（自检方式可被机械检索）。
5. 文件内含 `PONYSENTRY_ENABLED` 的默认值说明（`false`）与"必须显式置 true 才恢复上报"的表述（防部署方误判）。

---

## 7. C7 契约：既有门禁失明修复（**wave18-r3 新增**，含反熵边界裁定理由）

> 来源：`review-l3-scanner-wave18.json` finding#3（FATAL-FN，**既有缺陷**，本波纳入）。

### 7.1 缺陷机理（当前态证据）

- 调用点 `check-secrets.mjs:162`：`if (rule.isSafe && (rule.isSafe(secretVal, line) || rule.isSafe(secretVal, content)))` —— 第二实参是**整文件内容**。
- `Private Key Block` 规则（`:77-83`，`isSafe` 在 `:79-82`）在 context 上做 `upper.includes("MOCK"|"TEST"|"DUMMY"|"EXAMPLE")`。
- 后果：只要**同文件任意位置**出现 `test`/`mock`/`dummy`/`example` 字样，真实私钥块即被 100% 放行。
- **独立复现（本特派员，`/tmp/w18b-probe.mjs`）**：`scanTextContent(pk)` 命中 1；`scanTextContent(pk + "// unit test fixture")` 命中 **0**（`finding#3` PASS，即失明确认）。直接 API：`isSafe(BEGIN, BEGIN + "\nMIIEow\n-----END RSA PRIVATE KEY-----\n// unit test fixture")` => **`true`**（放行）。

### 7.2 反熵边界裁定理由（**Lead 裁定纳入本波，§12 记录**）

1. **同文件**：缺陷位于 `scripts/check-secrets.mjs`，与本波 C4 修改同一文件、同一函数 `scanTextContent`。
2. **同失效类别**：与 C4 补盲同属"**门禁失明**"（false negative）——本波使命即"让门禁不再失明"，修 C4 而留 C7 等于只堵一半。
3. **最小必要闭包**：根因是 `:162` 一处调用点的**上下文实参语义**，改动收敛于同一函数内的 1 处调用点 + 2 条规则对 context 的读取方式；不触碰导出面、不改规则条数、不改 CLI。
4. **为何不是字面"1 行删除"（必须如实说明）**：直接删除 `|| rule.isSafe(secretVal, content)`（即只传 `line`）会使 `tests/check-secrets.spec.ts:83-92` 的 **mock 私钥豁免**（`// MOCK TEST KEY` 位于 BEGIN 的**上一行**）失效 → 该既有断言从 0 违规变为 1 违规 → **破坏冻结的既有 spec**（§4.1/M9 禁止改既有断言）。**实测证据**：`isSafe(BEGIN, BEGIN)`（仅行）=> `false`（即会误报），`isSafe(BEGIN, "// MOCK TEST KEY\n"+BEGIN)` => `true`。故"最小**必要**闭包"= 有界窗口，而非最小行数。

### 7.3 冻结实现形态（`scanTextContent` 内，替换 `:162`）

```js
    // C7（r3）：context 收窄为「当前行 + 最多 3 行紧邻上文」的有界窗口。
    // 禁止把整文件内容交给 isSafe（既有 Private Key Block 规则会因此 100% 失明）。
    // 窗口不含 BEGIN 行之后的内容：私钥 base64 体可能偶然含 "TEST"/"MOCK" 字样。
    const contextWindow = lines.slice(Math.max(0, lineNum - 4), lineNum).join("\n");
    if (rule.isSafe && rule.isSafe(secretVal, contextWindow)) {
      continue;
    }
```

**冻结约束**

1. **不得**再向 `isSafe` 传入整文件内容（禁止 `content` 作为第二实参）。
2. 窗口**只向上**取（当前行 + 最多 3 行紧邻上文），**不得**包含当前行之后的内容。
3. `Private Key Block::isSafe`（`:79-82`）**本体不改**：它接收有界窗口后，`MOCK|TEST|DUMMY|EXAMPLE` 判定自然变为块级。
4. 其余 6 条前缀规则（`sk-`/`ghp_`/`gho_`/`AKIA`/`glpat-`/`xox`）仅使用 `match`，**不受影响、不得改动**。
5. 64-hex 规则按 §4.2 r3 版读取窗口最后一行（`.split("\n").pop()`）。
6. 窗口构造必须**每行一次**（O(行长)），不得在每个 match 内重复构造（与 §4.6 一致）。

### 7.4 冻结断言

1. 真实私钥块 + 文件任意远端出现 `test`/`mock`/`dummy`/`example` => **仍判违规**（≥1 命中）。
2. `tests/check-secrets.spec.ts:83-92` 的 mock 私钥（`// MOCK TEST KEY` 紧邻上一行）=> **仍 0 违规**（既有断言不得破坏）。
3. 私钥 base64 体中偶然含 `TEST` 字样 => **不得**因此豁免。
4. 多私钥块文件（前一块 mock、后一块真实）=> 真实块必须被命中（窗口按 match 所在行计算，无"首块歧义"）。
5. `node scripts/check-secrets.mjs --all` 在当前树仍 Exit 0（既有 4 处 64-hex 泄漏清理后）。

---

## 8. C8 契约：前端 ingest 硬超时（**wave18-r3 新增**）

> 来源：`review-l3p-wave18.json` finding#1（Q1 外部调用超时 ≤3000ms）。当前证据：`grep -nE "AbortController|AbortSignal|setTimeout|signal" src/lib/telemetry.ts` → **零命中（EXIT=1）**；`fetch`（现 `:289`）init 仅 `method/headers/body/keepalive`。达标对照：`client.rs:14` `REQUEST_TIMEOUT_SECS = 3` → 3000ms。

### 8.1 冻结实现形态（`reportError` 的 fetch 分支）

```ts
const INGEST_TIMEOUT_MS = 3000; // ≤ NFR external_call_timeout_ms
```

```ts
    const controller = new AbortController();
    const timeoutId = setTimeout(() => controller.abort(), INGEST_TIMEOUT_MS);
    fetch(`${INGEST_URL}/api/v1/ingest`, {
      method: "POST",
      headers,
      body: JSON.stringify(payload),
      keepalive: true,
      signal: controller.signal,
    })
      .catch((fetchErr) => {
        console.warn("[ponysentry] fetch ingest silent fail:", fetchErr);
      })
      .finally(() => clearTimeout(timeoutId));
```

**冻结约束**

1. 超时上限 **≤ 3000 ms**（对齐 NFR `external_call_timeout_ms`，见 §12 与 `.dev-team/nfr-baseline-wave18.json` 的 `wave18b_external_call_timeout_landing`）。
2. 必须用 `AbortController` + `setTimeout` + `signal`；**不得**使用 `AbortSignal.timeout`（旧 WebView 兼容风险，Lead 指定）。
3. `clearTimeout` 必须挂在 fetch **settle 之后**（`.finally`）——**禁止**在 `try/finally` 中于发起 fetch 后立即清除（那会使超时永不生效）。
4. 既有静默语义保留：`.catch` 内 `console.warn` + 不抛出；`reportError` 仍为 fire-and-forget（不 await fetch、不阻塞）。
5. `INGEST_TIMEOUT_MS` 为**非导出**模块常量，不得新增导出符号。
6. 守卫（§5.2，现 `:279`）位置不变：空配置时**不进入**该分支，即不创建 timer。

### 8.2 冻结断言

1. `src/lib/telemetry.ts` 内 `fetch` 的 init 含 `signal:`；且存在 `new AbortController()`、`setTimeout(`、`clearTimeout(`。
2. 超时常量值 ≤ 3000。
3. **变异敏感（零假测试）**：删除 `signal` 或删除 `setTimeout` 任一，断言必须失败。**注意**：vitest 下 `reportError` 在 `:254-260` test-mode 守卫处即返回，**无法**通过"调用 reportError 后推进 fake timer"来触达该分支 —— 该路径对 C8 是空洞的（同 §5.4 混淆问题）。故测试必须至少包含**源码静态断言**（检索 `signal:`/`AbortController`/`setTimeout`/`clearTimeout` 与常量值），并可辅以"在关闭 test-mode 守卫条件下"的行为测试；若行为测试不可稳定达成，Test Agent **必须显式记录未采用 + 原因**，不得静默降级。

## 9. 本次新增发现与裁定登记（r2 保留 + r3 新增）

### 9.1 findings 与裁定（wave18-r3）

> 完整逐条裁定表见 **`.dev-team/wave18b-adjudication.md`**（覆盖 4 路审查全部 12 项 findings）。本节登记裁定结论与欠账。

| # | 来源 finding | 裁定 | 落点 |
|---|---|---|---|
| A1 | `review-l3-scanner` #1（豁免 3 整行放行） | **FIX** | C4-R1（§4.2/§4.3）+ G1/G2（§4.7） |
| A2 | `review-l3-scanner` #2（豁免 1/2 整行放行） | **FIX** | C4-R1/R2 + G3–G6（§4.7） |
| A3 | `review-l3-scanner` #3（私钥规则整文件失明） | **FIX（既有缺陷纳入本波）** | **C7**（§7，含反熵理由 §7.2） |
| A4 | `review-l3-scanner` #4（二次方 DoS） | **FIX** | C4-R3（§4.6 性能硬约束） |
| A5 | `review-l3-scanner` #5（`verify` 链 `--staged` 假绿） | **WAIVE（登记欠账 R4）** | §9.2 R4 |
| A6 | `review-l3-contract` #1（零断言假测试） | **FIX（移交 T2）** | §9.2 R5 / §11 M12 |
| A7 | `review-l3-contract` #2（live 用例静默 skip 以 ok 上报） | **FIX（移交 T2）** | §9.2 R6 / §11 M13 |
| A8 | `review-l3-contract` #3 + `review-l3-security` #1（看板断言为假，自反证） | **FIX（既有缺陷纳入本波）** | §9.2 R7 |
| A9 | `review-l3-contract` #4（C2 set-but-empty fail-open） | **FIX** | C2 r3 修订（§2，断言 7/8/9） |
| A10 | `review-l3p` #1（前端 fetch 无硬超时） | **FIX** | **C8**（§8） |
| A11 | `review-l3p` #2（无 `.env.example`） | **FIX** | **C6**（§6） |
| — | R1/R2/R3（r2 既有登记） | **保留** | §9.2 R1/R2/R3 |

**计数核对**：12 项 findings → **FIX 8 项**（A1+A2 合并为"C4 豁免守卫/位置锚定"一项修复；A3、A4、A6、A7、A8、A9、A10、A11）· **WAIVE 1 项**（A5）· **既有缺陷纳入本波 1 项**（A8，由两路独立复现同一缺陷）· **登记 3 项**（R1/R2/R3，非 findings）。

### 9.2 裁定与欠账登记

**R1 — 32-hex 不作"必须命中"要求（Lead 裁定，r2 保留）**

- 裁定内容：C4 规则维持**严格 64 位窗口**（§4.3），**不**新增 32-hex 必命中规则。
- 理由（Lead）：补 32-hex 规则会与**零误报**目标冲突 —— MD5 尺寸/短哈希/UUID 片段在仓库中误报面大；而"零误报"是**绑定约束**（该门禁跑在 `npm run verify` 上，任何误报即阻塞全量自检）。
- 处置：登记为**后续 backlog 事项**，标注"**待独立评估长度窗口与豁免口径**"（须先给出仓库级 32-hex 误报面实测与豁免设计，再决定是否引入；不得在本波就地放宽 C4）。
- 本波状态：**C4 长度窗口不变**（r3 只改豁免守卫/位置锚定/性能，见 §4.2/§4.3/§4.6）。

**R2 — 孤儿死文件事实（Lead 复核 + 本特派员独立复验，r2 保留）**

- 事实（双路证据）：
  1. **不被 cargo 编译**：`grep -rn "acceptance" Cargo.toml crates/*/Cargo.toml src-tauri/Cargo.toml` → **EXIT=1（零命中）**；`Cargo.toml:2-5` workspace `members = ["crates/pony-agent-core", "src-tauri"]`，仓库根 `tests/` 不是 package 目录。
  2. **不被 vitest 收集**：`vitest.config.ts:20` `include: ["tests/**/*.spec.ts", "tests/**/*.test.ts"]`；该文件扩展名为 `.rs`。
- 性质：`crates/pony-agent-core/tests/ponysentry_telemetry_test.rs` 旧契约的**未编译副本**，无编译期/测试期强制力，但会误导读者。
- 处置：**仍须迁移其断言**（§11 M4/M5，消除陈旧契约误导源）；登记为 **dead-file backlog**（删除 或 纳入真实编译目标，待 Lead 决策）。

**R3 — 泄漏面复核（Lead `git ls-files` 复核，与本矩阵一致，r2 保留）**

- 泄漏 token 在**跟踪文件中恰好 3 个文件 4 处**（`config.rs:16`、`config.rs:34`、`ponysentry_telemetry_test.rs:184`、`src/lib/telemetry.ts:15`，与 §0.3 一致）；`.dev-team/` 内**不含**该 token。r3 时点：`config.rs` 已清零（Executor 已落地 C1/C2），`telemetry.ts:15` 已清零（C5 已落地）；仅剩 `ponysentry_telemetry_test.rs`（T2 已迁移）。

**R4 — `npm run verify` 的 `--staged` 假绿：WAIVE（不修，登记欠账）**

- 事实（`review-l3-scanner-wave18.json` finding#5）：`package.json:39` `"secrets:check": "node scripts/check-secrets.mjs --staged"`，`package.json:41` `verify` 串联它。干净索引下 `--staged` 输入为 0 → 在含**未暂存**泄漏的工作区上给出假绿。
- 缓解事实（已验证）：`.git/hooks/pre-commit` 调 `--staged`、`pre-push` 调 `--pre-push`（均加载全部规则），`.github/workflows/ci.yml:44` 独立跑 `node scripts/check-secrets.mjs --all` → 泄漏必须被 staged 才能进 commit；缺口仅在 `npm run verify` 自身。
- **裁定：WAIVE（本波不修）**。理由：属**项目级门禁语义变更**（改 `verify` 链行为，影响全仓自检口径），且 CI 与 git hooks 已分别覆盖 `--all`/`--pre-push`，超出本波"凭证清理 + 扫描器补盲"的最小闭包。
- **欠账登记（DEBT）**：`DEBT-W18-R4`：`npm run verify` 的 secrets 步骤应改为 `--all`（或 `--all` 与 `--staged` 串联）；到期条件 = 下一次涉及门禁/CI 语义的波次。**证据**：`review-l3-scanner-wave18.json` finding#5。

**R5 — 零断言假测试（移交 T2）**

- 事实（`review-l3-contract-wave18.json` finding#1）：`crates/pony-agent-core/tests/ponysentry_manual_upload_test.rs:137` `test_upload_session_trace_contract_shape` 函数体仅两行注释，零 `assert/expect/panic/unwrap`，`cargo test ... test_upload_session_trace_contract_shape` => `ok` Exit 0。
- 裁定：**FIX，移交 T2** 补真断言（至少 1 条可证伪断言：例如断言控制面入口存在/签名或行为契约）。见 §11 M12。
- 边界说明：该函数为**既有**缺陷（`git show 98bd584` / `d3ae148` 逐字相同），非本波引入；纳入本波仅因它属**本波冻结验收集**。

**R6 — live 用例静默 skip 以 `ok` 上报（移交 T2）**

- 事实（`review-l3-contract-wave18.json` finding#2）：`ponysentry_manual_upload_test.rs:151` 起，`:155-181` 三个 early `return`（`:159`/`:169`/`:177` 打印 `[skip] ...`）使默认环境下 `:224-238` 的 201/401/404 断言永不执行，却以 `test ... ok` 上报（等价 `#[ignore]` 但报 `ok`）。
- 裁定：**FIX，移交 T2**：拆分为 **"恒跑真断言" + "env 门控 live"** 两段；恒跑段必须有真断言（不得零断言通过），live 段须以 `#[ignore]` 或显式 env 门控并如实上报（不得以 `ok` 冒充）。见 §11 M13。
- 冲突说明：§11 头部"不得跳过"与 M7"授权 env 门控"的冲突，以本裁定为准：**门控允许，但必须以可区分的测试结果上报（`ignored`），且不得使整条用例零断言通过**。

**R7 — 看板断言为假（自反证）：既有缺陷纳入本波**

- 事实（`review-l3-contract-wave18.json` finding#3 **与** `review-l3-security-wave18.json` finding#1 **两路独立复现**）：`management/task-system/00_DASHBOARD.md:23` 断言"全仓 `grep -rn "MAX_SUBAGENT_DEPTH"` 零命中"，实测 **2 命中**，其中 1 处即该断言行自身，另 1 处为 `management/task-system/03_TASKS/PA-110-agent-teams-and-subagent-tools.md:26`（`- **AC-04**: 防护递归深度爆炸（MAX_SUBAGENT_DEPTH ≤ 2）。`）。仅在加 `--include=*.rs --include=*.ts` 时才是 0 命中。
- 裁定：**FIX（既有缺陷纳入本波）**。正确表述应为"全仓**代码**（`*.rs`/`*.ts`）零命中"；修正后该行必须自洽（改后重跑同一 grep，命中数必须与文本陈述一致）。
- 说明：该文件不在 `.dev-team/` 写域内，故**本特派员不修改**；裁定与验收判据在此登记，由 Lead 指派执行方（看板修正属 wave-18 ① 的既有任务面）。

---

### 9.3 侦察新增发现清单（F1–F6，r2 保留）

| # | 位置 | 事实 | 影响 |
|---|---|---|---|
| **F1** | `src/lib/telemetry.ts:13-15`（r1 时点） | 前端 `CLIENT_TOKEN` 同样硬编码同一 64-hex，作为 `VITE_PONYSENTRY_CLIENT_TOKEN` 未设时的回落；`:280-282` 把它写进 `X-Client-Token` 头；`:284` 浏览器侧 `fetch` 直发 `/api/v1/ingest` | **第三处泄漏点**。**Lead 已裁定纳入本波** → 冻结为 **C5（§5）**；**T3 已落地**（现 `telemetry.ts` token 计数 0） |
| **F2** | `crates/pony-agent-core/tests/ponysentry_manual_upload_test.rs:189` | 明文数据库凭证：`postgresql://job_copilot:<已掩码>@127.0.0.1:30543/ponysentry` | 既有硬编码凭证（非 64-hex，C4 规则不覆盖）。**不在本波写域**，登记移交 Lead 分诊（随 M7 一并处置） |
| **F3** | `tests/acceptance/ponysentry_telemetry_test.rs:177-198` | 与 `crates/pony-agent-core/tests/ponysentry_telemetry_test.rs:176-201` 的 `test_config_contract` **近乎重复的副本**（差异：副本无 token 断言、有 `assert!(default_cfg.enabled)` @`:182`） | 迁移必须**双份同步**（M4/M5），否则副本成为新的旧契约残留 |
| **F4** | `crates/pony-agent-core/tests/ponysentry_telemetry_test.rs:257` | `init(PonySentryConfig::default())` 位于 `test_breadcrumb_ring_buffer_limit` 末尾 | 默认 `enabled=false` 后该调用变为"禁用态 init"；该测试仅验证模块级函数不 panic（`:258-259`），语义仍成立，但断言强度下降，需 T2 评估是否补断言（M8） |
| **F5** | `scripts/check-secrets.mjs` 长度窗口 | 已冻结规则为**严格 64 位**窗口（§4.3），未覆盖 32-hex（MD5 尺寸）凭证 | **Lead 裁定：32-hex 不作"必须命中"要求** → backlog，见 §9.2 R1 |
| **F6** | `tests/acceptance/ponysentry_telemetry_test.rs` | 该 `.rs` 文件**既不被 cargo 编译**（全仓 Cargo.toml 零 `acceptance` 命中，grep EXIT=1；workspace members 仅 `crates/pony-agent-core`+`src-tauri`，`Cargo.toml:2-5`），**也不被 vitest 收集**（`vitest.config.ts:20` include 仅 `tests/**/*.spec.ts`/`*.test.ts`） | **孤儿死文件**（旧契约的未编译副本）。仍须迁移断言（M4/M5），并登记为 dead-file backlog，见 §9.2 R2 |

---

## 10. 影响面 / 非目标

- **非目标（不得改动）**：`client.rs` 全部早退与 sender 构造；`sanitizer.rs` 脱敏管道；`check-secrets.mjs` 既有 6 条前缀规则的 `isSafe`（`sk-`/`ghp_`/`gho_`/`AKIA`/`glpat-`/`xox`）、`BLOCKED_FILE_PATTERNS`、`isSafeFilename`、CLI 入口与退出码语义、`check-secrets.spec.ts` 文件名整体豁免分支；`telemetry.ts` 的 IPC 分支、test-mode 静默守卫（`:254-260`）、探针过滤（`:249-251`）、脱敏管道（`:54-108`）、堆栈解析（`:143-191`）。
- **r3 例外（明确允许改动，属本波契约面）**：`Private Key Block` 规则的**上下文来源**（不改进程内本体，仅由 C7 的调用点收窄窗口）；`scanTextContent:162` 的 context 实参语义；`telemetry.ts` fetch 分支新增 `signal`/`setTimeout`/`clearTimeout`。
- **本波不引入**：结构化日志、新依赖、**新导出符号**、新环境变量名。
- **向后兼容**：`Option<String>` 与 `bool` 类型不变；`endpoint/environment/release` 取值与回落链不变；前端全部导出符号名称/类型/可见性不变（C5 §5.2 约束 5）。
- **导出值变更（有意，C5）**：`INGEST_URL` 由 `"https://sentry.ponyjob.top"` 默认值改为 `""`；`CLIENT_TOKEN` 由硬编码 64-hex 默认值改为 `""`。**无外部导入方**（E11）。
- **语义变更（有意）**：后端 `enabled` 的 set-but-empty/纯空白由 `true` 改为 `false`（C2 r3，fail-closed）；门禁 `isSafe` 第二实参由"整文件内容"改为"有界窗口"（C7）；`Private Key Block` 的豁免范围由全文件收窄为块级（C7）。
- **运行时语义变更（有意）**：默认零 egress —— 后端 `enabled=false` 且无 token（C1/C2）；前端 `INGEST_URL`/`CLIENT_TOKEN` 未配置时不发 `fetch`（C5）；前端 fetch 硬超时 ≤3000ms（C8）。

---

## 11. 旧契约迁移清单（**移交 T2 执行**，逐条）

> 规则：T2 只能"迁移断言以匹配 §1–§8 各冻结契约"，**不得放宽为恒真断言、不得删除测试、不得跳过（`#[ignore]`）**；R6 例外（env 门控须以 `ignored` 如实上报，且不得使整条用例零断言）。

| # | 文件:行 | 现有旧契约断言 | 迁移方向（目标态） | 优先级 |
|---|---|---|---|---|
| M1 | `crates/pony-agent-core/tests/ponysentry_telemetry_test.rs:184-185` | `assert_eq!(default_cfg.client_token.as_deref(), Some("6aa12e9e…c5d24"))`（硬编码 token，同时是 C4 规则的命中点） | 改为 `assert!(default_cfg.client_token.is_none())`；**删除硬编码字面量** | P0（**已由 T2 落地**：现该文件 token 计数 0） |
| M2 | `crates/pony-agent-core/tests/ponysentry_telemetry_test.rs:186` | `assert!(default_cfg.enabled)` | 改为 `assert!(!default_cfg.enabled)` | P0 |
| M3 | `crates/pony-agent-core/tests/ponysentry_telemetry_test.rs:176-201` | `test_config_contract` 未覆盖"未设 env 的默认路径"（测试内先 set 再断言，无未设分支） | 新增反向断言：清空 `PONYSENTRY_CLIENT_TOKEN`/`PONYSENTRY_ENABLED` 后 `from_env()` 得 `client_token.is_none()` + `enabled == false`；并补 `"1"`/`"true"`/`"0"`/`"FALSE"` 真值表断言；**【r3 追加】** 补 `""`/`"   "`/`"\t"` => false 的 fail-closed 边界断言（§2 断言 7/8/9） | P1 |
| M4 | `tests/acceptance/ponysentry_telemetry_test.rs:182` | `assert!(default_cfg.enabled)`（副本） | 同 M2；并评估与 M1/M2 的双份同步机制（F3） | P0 |
| M5 | `tests/acceptance/ponysentry_telemetry_test.rs:177-198` | 副本 `test_config_contract` 与主文件漂移（无 token 断言） | 与主文件对齐到同一目标契约（含 M3 的反向断言与 r3 边界断言） | P1 |
| M6 | `crates/pony-agent-core/tests/ponysentry_manual_upload_test.rs:148` | `assert!(config.client_token.is_some(), "Default config must provide client token for sentry auth")` | 删除该断言；改为显式构造带 token 的 config（或 `PONYSENTRY_CLIENT_TOKEN` 注入）——**默认配置不再提供 token** | P0 |
| M7 | `crates/pony-agent-core/tests/ponysentry_manual_upload_test.rs:143-186`（`test_upload_trace_to_live_sentry_contract`） | 依赖 `default()` 的 token + 打**生产端点** `sentry.ponyjob.top`，断言 `201 CREATED`，并在 `:189` 内嵌明文 psql 凭证做清理 | 必须**去 live 网络化**：改为 local/mock ingest 或 `#[ignore]` + 显式 env 门控（**不得**默认打生产）；清理路径中的明文凭证（F2）随迁移一并上报 Lead | P0 |
| M8 | `crates/pony-agent-core/tests/ponysentry_telemetry_test.rs:257` | `init(PonySentryConfig::default())`（F4） | 保持不 panic 语义；如需保留"模块级函数可用"强度，改为 `init` 一个显式 `enabled: true` 的 config | P2 |
| M9 | `tests/check-secrets.spec.ts`（全 94 行） | 现有 8 条内容断言 + 文件名豁免断言 | **不改既有断言**；仅**新增**用例：命中 64-hex 字面量；不命中 7/8/12/40 位 git SHA；不命中 `checksum`/`sha256`/`computedHash`/`fingerprint` 上下文；`sk-draft/sk-test/sk-mock` 与 `.env.example` 豁免保持；**【r3 追加】** 新增 §4.7 的 G1–G6 六条**必须命中**用例（r2 实现下全 0 命中）+ C7 私钥块级豁免用例（真实块 + 远端 `test` 字样 => 命中；`// MOCK TEST KEY` 紧邻上一行 => 0 命中） | P1 |
| M10 | `tests/unit/telemetry-suppression.spec.ts:10-21` | 断言 vitest 下 `fetch`/`safeInvoke` 零调用（由 test-mode 守卫达成，**对 C5 空洞**，§5.4） | **不得删改既有断言**；**新增** C5 变异敏感断言：`INGEST_URL === ""` + `CLIENT_TOKEN === ""`（未设 env），并按 §5.4 第 2 条做源码静态断言（无 64-hex、无端点硬编码回落） | P0（C5 验收核心） |
| M11 | `tests/acceptance/ponysentry_telemetry_test.rs`（孤儿死文件，F6/R2） | 不被 cargo 编译、不被 vitest 收集；承载旧契约（`:182` `assert!(default_cfg.enabled)`） | 先按 M4/M5 迁移其断言（消除陈旧契约误导源）；再登记 **dead-file backlog**（删除 或 纳入真实编译目标，待 Lead 决策）。**不得**以"反正不编译"为由跳过迁移 | P1 |
| M12 | `crates/pony-agent-core/tests/ponysentry_manual_upload_test.rs:137-140`（`test_upload_session_trace_contract_shape`） | **零断言**恒真空通过（R5 / contract finding#1） | 补**真断言**（≥1 条可证伪）：断言控制面入口的存在性/签名/行为契约；**禁止**继续以注释充当测试体 | P0（r3 新增） |
| M13 | `crates/pony-agent-core/tests/ponysentry_manual_upload_test.rs:151-181`（`test_upload_trace_to_live_sentry_contract`） | `:155-181` 三个 early `return` 静默 skip，却以 `test ... ok` 上报（R6 / contract finding#2） | 拆为**两段**：①"恒跑真断言"段（无 env 依赖，必须有真断言）；②"env 门控 live"段（`#[ignore]` 或显式 env 门控，未满足条件时以 `ignored` 如实上报，**不得**报 `ok`）。两段均不得零断言通过 | P0（r3 新增） |

**迁移验收硬要求（T2）**：
1. `npm test`（含 `tests/check-secrets.spec.ts`、`tests/unit/telemetry-suppression.spec.ts` + C5/C8 新增断言）Exit 0；
2. `cargo test -p pony-agent-core --test ponysentry_telemetry_test`、`--test ponysentry_manual_upload_test` Exit 0；
3. `node scripts/check-secrets.mjs --all` 在迁移后 Exit 0（泄漏清空 + C4/C7 修复后仍零误报）；
4. C5/C8 变异敏感断言**必须存在**，且能被"恢复硬编码回落 / 删除 `signal` 或 `setTimeout`"反例证伪（Test Agent 需给出反例实测收据）；
5. M12/M13 修复后，`test_upload_session_trace_contract_shape` 与 live 用例的**恒跑段**必须各自含真断言（Test Agent 需给出"注释掉断言则测试失败"的实测收据）。

---

## 12. 冻结签名与 revision 变更记录

### 12.1 冻结签名

- 本矩阵为 wave-18 的**唯一契约基线**（当前 revision **`wave18-r3`**）。
- 任何偏离（新增豁免、改变规则长度窗口、改 `client.rs` 早退、改导出面、改 C5 守卫位置、改 C7 窗口方向/宽度、改 C8 超时上限）都必须走 Lead 裁定并升 revision，不得由实施者就地放宽。
- 配套产物：`.dev-team/nfr-baseline-wave18.json`、`.dev-team/nfr-baseline.schema.json`（additive-only）、`.dev-team/wave18-tooling-deviation.md`、`.dev-team/wave18b-adjudication.md`（12 项 findings 逐条裁定）。

### 12.2 revision 变更记录

| revision | 变更 | 原因（Lead 裁定 / 依据） |
|---|---|---|
| `wave18-r1` | 冻结 C1（`default()`）、C2（`from_env()`）、C3（`client.rs` 四处早退沿用）、C4（`check-secrets.mjs` 裸 64-hex 规则）；旧契约迁移清单 M1–M9（r1 编号 §7）；§0 证据 E1–E8 | task-1 初始交付 |
| `wave18-r2` | **新增 C5（§5，前端 `telemetry.ts` 默认零 egress）**；F1 由"待裁定"改判为"已纳入本波 → C5"；新增 §5.4 非空洞测试要求；新增 F5/F6 与裁定登记 R1/R2/R3；迁移清单新增 M10/M11；新增证据 E9–E12 | **Lead 裁定（wave18-r2）**：F1 第三处泄漏点纳入本波并冻结为 C5；登记 32-hex backlog 与孤儿死文件事实；同步 task-3 已生效要求 |
| `wave18-r3` | ① **C2 修订**：`PONYSENTRY_ENABLED` set-but-empty/纯空白 => `false`（§2，断言 7/8/9 + 冻结实现形态）；② **C4 三条修订**：R1 豁免 1/2/3 加 `credWord` 守卫、R2 豁免 2/3 位置锚定只豁免该 hex、R3 按行记忆化消除二次方（§4.2/§4.3/§4.6 + §4.7 的 G1–G6 反例用例）；③ **新增 C6**（`.env.example` 部署配置模板，§6）、**C7**（既有私钥规则整文件失明修复 + 反熵边界理由，§7）、**C8**（前端 ingest 硬超时 ≤3000ms，§8）；④ 裁定登记 R4（WAIVE + DEBT-W18-R4）/R5/R6/R7，R1/R2/R3 保留（§9）；⑤ 迁移清单新增 M12/M13 并强化验收（§11）；⑥ 原 §6–§9 顺延为 §9–§12 | **Lead 裁定（wave18-r3）**：4 路 L3/L3-P 审查共 12 项 findings，FIX 8 / WAIVE 1（R4）/ 既有缺陷纳入 1（R7）/ 登记 3（R1–R3） |
| — | **C4 长度窗口（严格 64 位）与导出面在 r3 中未做修改** | R1：零误报为绑定约束，不得为覆盖 32-hex 而放宽长度窗口；导出面兼容是 §4.1 硬约束 |

### 12.3 C7 反熵边界裁定（Lead 裁定原文要点，独立登记）

> 依据 task-4：「**Lead 裁定纳入本波**：同文件、同函数、同失效类别（门禁失明），且本波使命即"让门禁不再失明"，1 行级最小闭包」。

- **同文件**：缺陷在 `scripts/check-secrets.mjs`，与 C4 同文件。
- **同函数**：同在 `scanTextContent`（`:162` 调用点）。
- **同失效类别**：与 C4 补盲同属"门禁失明"（false negative），本波使命即消除门禁失明。
- **最小必要闭包**：改动收敛于 `:162` 一处调用点的上下文实参语义（整文件 → 有界窗口）+ 2 条规则对 context 的读取方式；不改导出面、不改规则条数、不改 CLI、不改既有 6 条前缀规则。
- **对"1 行级"的如实修正**：字面删除 `|| isSafe(v, content)` 会破坏 `tests/check-secrets.spec.ts:83-92`（mock 私钥标记位于 BEGIN 上一行），实测 `isSafe(BEGIN, BEGIN) === false` → 既有断言由 0 违规变 1 违规。故采用**有界窗口**（当前行 + 最多 3 行紧邻上文），其为满足"不得破坏既有 spec"前提下的最小**必要**闭包（详见 §7.2 与 §7.3）。
- **反熵声明**：本项不夹带任何与"门禁失明"无因果关联的重构；`Private Key Block::isSafe` 本体逐字保留，仅其**上下文来源**被收窄。
