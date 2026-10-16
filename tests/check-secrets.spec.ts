import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterAll, beforeAll, describe, it, expect } from "vitest";
import {
  scanTextContent,
  checkBlockedFilename,
  isSafeFilename,
  SECRET_RULES,
  runChecks
} from "../scripts/check-secrets.mjs";

/** 与 `check-secrets.mjs` 的 masked 生成规则一致：前 4 位 + 星号 + 后 4 位。 */
function maskedOfSecretValue(value: string): string {
  return value.substring(0, 4) + "*".repeat(value.length - 8) + value.substring(value.length - 4);
}

/** 失败信息中不出现完整 64-hex 字面量（避免证据二次泄漏）。 */
function redactHexes(content: string, hexes: string[]): string {
  return hexes.reduce((acc, hex) => acc.replaceAll(hex, "<64hex>"), content);
}

/**
 * **意图断言**（防"只数条数、不验目标"的空洞断言）：不仅要求命中条数，
 * 还逐条校验命中的**是哪一个 hex**（掩码）与**哪一条规则**。
 * `expectedValues` 按命中顺序给出预期被捕获的 hex 取值；`contextLabel` 必须已脱敏。
 */
function expectCaughtSecret(
  hits: Array<{ rule: string; masked: string }>,
  expectedValues: string[],
  contextLabel: string
): void {
  const wanted = expectedValues.map(maskedOfSecretValue);
  expect(
    hits.map((hit) => hit.masked),
    `输入 ${contextLabel} 命中的 hex 取值不符合预期（期望 ${wanted.length} 条）`
  ).toEqual(wanted);
  expect(
    hits.every((hit) => hit.rule.toLowerCase().includes("64-hex")),
    `输入 ${contextLabel} 的命中必须来自 64-hex 规则，实际：${hits
      .map((hit) => hit.rule)
      .join(" | ")}`
  ).toBe(true);
}

describe("check-secrets gate verification", () => {
  describe("Blocked Filenames", () => {
    it("should block sensitive files like .env and private keys", () => {
      expect(checkBlockedFilename(".env")).toBeTruthy();
      expect(checkBlockedFilename(".env.production")).toBeTruthy();
      expect(checkBlockedFilename("id_rsa")).toBeTruthy();
      expect(checkBlockedFilename("server.key")).toBeTruthy();
      expect(checkBlockedFilename("cert.pem")).toBeTruthy();
    });

    it("should allow safe files like .env.example or regular code", () => {
      expect(isSafeFilename(".env.example")).toBe(true);
      expect(isSafeFilename(".env.sample")).toBe(true);
      expect(checkBlockedFilename(".env.example")).toBeNull();
      expect(checkBlockedFilename("package.json")).toBeNull();
      expect(checkBlockedFilename("src/index.ts")).toBeNull();
    });
  });

  describe("Secret Patterns Detection", () => {
    it("should detect real OpenAI tokens", () => {
      const realLikeToken = "sk-proj-abcde1234567890abcdef1234567890";
      const leaks = scanTextContent(`const key = "${realLikeToken}";`);
      expect(leaks.length).toBe(1);
      expect(leaks[0].rule).toContain("LLM API Token");
      expect(leaks[0].masked).toContain("****");
    });

    it("should allow safe mock/dummy/draft tokens", () => {
      const content = `
        const draft = "sk-draft-key";
        const testKey = "sk-test-12345678901234567890";
        const mockKey = "sk-mock-12345678901234567890";
        const placeholder = "sk-placeholder-abc12345";
      `;
      const leaks = scanTextContent(content);
      expect(leaks.length).toBe(0);
    });

    it("should detect GitHub Personal Access Tokens", () => {
      const ghp = "ghp_123456789012345678901234567890123456";
      const leaks = scanTextContent(`GITHUB_TOKEN=${ghp}`);
      expect(leaks.length).toBe(1);
      expect(leaks[0].rule).toContain("GitHub Personal Access Token");
    });

    it("should detect AWS Access Key IDs", () => {
      const akia = "AKIAIOSFODNN7EXAMPLE"; // EXAMPLE is safe
      expect(scanTextContent(`aws_key = "${akia}"`).length).toBe(0);

      const realAkia = "AKIA1234567890ABCDEF";
      const leaks = scanTextContent(`aws_key = "${realAkia}"`);
      expect(leaks.length).toBe(1);
      expect(leaks[0].rule).toContain("AWS Access Key");
    });

    it("should detect GitLab PAT", () => {
      const glpat = "glpat-abcdef12345678901234";
      const leaks = scanTextContent(`gl_token = "${glpat}"`);
      expect(leaks.length).toBe(1);
      expect(leaks[0].rule).toContain("GitLab Personal Access Token");
    });

    it("should detect unmocked Private Key Blocks", () => {
      const privKey = `
-----BEGIN RSA PRIVATE KEY-----
MIIEowIBAAKCAQEA0Y1+xyz...
-----END RSA PRIVATE KEY-----
`;
      const leaks = scanTextContent(privKey);
      expect(leaks.length).toBe(1);
      expect(leaks[0].rule).toContain("Private Key Block");
    });

    it("should allow mock Private Key Blocks", () => {
      const mockPrivKey = `
// MOCK TEST KEY
-----BEGIN RSA PRIVATE KEY-----
MIIEowIBAAKCAQEA0Y1+xyz...
-----END RSA PRIVATE KEY-----
`;
      const leaks = scanTextContent(mockPrivKey);
      expect(leaks.length).toBe(0);
    });
  });

  // ---------------------------------------------------------------------------
  // wave-18 C4：裸 64-hex 高熵凭证规则（L2-AT ①②）
  // 契约基线：.dev-team/contract-matrix-wave18.md §4.2 / §4.3 / §4.4 / §4.5
  // 红相基线：SECRET_RULES 仅 7 条、无裸 hex 规则 ⇒ 真实泄漏完全失明（unpatched gate = 0）。
  // 说明：本文件被 scanTextContent 按文件名整体豁免（check-secrets.mjs:119-123），
  // 故以下所有样例串均在“无文件名”条件下扫描（最严苛路径），不会被豁免掩盖。
  // ---------------------------------------------------------------------------
  describe("wave-18 C4: Bare 64-hex high-entropy credential rule", () => {
    // 运行时拼装，避免本文件自身出现 64-hex 字面量（新规则会自我拦截）。
    const hex64 = (seed: string): string => seed.repeat(4); // 16 * 4 = 64

    const LOW = hex64("0123456789abcdef");
    const UPPER = hex64("0123456789ABCDEF");
    const MIXED = hex64("0123456789aBcDeF");

    it("① 命中：裸 64-hex 小写凭证（并给出脱敏掩码）", () => {
      const leaks = scanTextContent(`const value = "${LOW}";`);
      expect(leaks.length).toBe(1);
      expect(leaks[0].rule.toLowerCase()).toContain("64-hex");
      expect(leaks[0].masked).toContain("****");
      expect(leaks[0].line).toBe(1);
    });

    it("① 命中：大写与混合大小写 64-hex", () => {
      expect(scanTextContent(`const upper = "${UPPER}";`).length).toBe(1);
      expect(scanTextContent(`const mixed = "${MIXED}";`).length).toBe(1);
    });

    it("① 命中：hex 夹在注释 / JSON / 下划线前缀标识符中", () => {
      expect(scanTextContent(`// rotate ${LOW} before release`).length).toBe(1);
      expect(scanTextContent(`{"client_token": "${LOW}"}`).length).toBe(1);
      expect(scanTextContent(`const token_${LOW} = 1;`).length).toBe(1);
    });

    it("② 不报：git SHA 7/8/12/40 位", () => {
      const sha40 = LOW.slice(0, 40);
      const content = [
        `commit ${sha40}`,
        `short12 ${sha40.slice(0, 12)}`,
        `short8 ${sha40.slice(0, 8)}`,
        `short7 ${sha40.slice(0, 7)}`,
      ].join("\n");
      expect(scanTextContent(content).length).toBe(0);
    });

    it("② 不报：63 / 65 / 128 位 hex（长度窗口严格 64）", () => {
      const h63 = LOW.slice(0, 63);
      const h65 = `${LOW}a`;
      const h128 = `${LOW}${LOW}`;
      const content = [
        `a=${h63}`,
        `b=${h65}`,
        `c=${h128}`,
        `d=f${LOW}`,
      ].join("\n");
      expect(scanTextContent(content).length).toBe(0);
    });

    it("② 不报：既有 sk-draft / sk-test / sk-mock 与 .env.example 文件名豁免保持", () => {
      const content = [
        `draft = "sk-draft-key"`,
        `test = "sk-test-12345678901234567890"`,
        `mock = "sk-mock-12345678901234567890"`,
      ].join("\n");
      expect(scanTextContent(content).length).toBe(0);
      expect(isSafeFilename(".env.example")).toBe(true);
      expect(checkBlockedFilename(".env.example")).toBeNull();
    });

    it("② 不报：checksum / sha256sum / computedHash / fingerprint 上下文", () => {
      const content = [
        `checksum = "${LOW}"`,
        `${LOW}  crates/pony-agent-core/src/lib.rs`,
        `"computedHash": "${LOW}"`,
        `fingerprint: ${LOW}`,
      ].join("\n");
      expect(scanTextContent(content).length).toBe(0);
    });

    it("② 不报：Markdown 表格摘要单元格与反引号独占摘要（证据收据豁免）", () => {
      const content = [
        `| 值 | ${LOW} |`,
        `| 值 | \`${LOW}\` |`,
        `- 收据：\`${LOW}\``,
      ].join("\n");
      expect(scanTextContent(content).length).toBe(0);
    });

    it("② 兼容：SECRET_RULES 导出面保持既有 7 条规则并追加 64-hex 规则", () => {
      const names = SECRET_RULES.map((rule: { name: string }) => rule.name);
      const legacyRuleMarkers = [
        "LLM API Token",
        "GitHub Personal Access Token",
        "GitHub OAuth Access Token",
        "AWS Access Key",
        "GitLab Personal Access Token",
        "Slack Token",
        "Private Key Block",
      ];
      for (const marker of legacyRuleMarkers) {
        expect(
          names.some((name: string) => name.includes(marker)),
          `既有规则丢失：${marker}`
        ).toBe(true);
      }
      expect(names.some((name: string) => name.toLowerCase().includes("64-hex"))).toBe(true);
    });

    it("② 端到端零误报：runChecks({mode:'all'}) 在全仓迁移后必须 0 违规", () => {
      // 该断言同时覆盖“补盲后无漏报”与“全仓零误报”：规则落地后若任一 tracked 文件
      // 被误报，或三处泄漏点未清空，本用例即 FAIL（门禁跑在 npm run verify 上）。
      const violations = runChecks({ mode: "all" });
      expect(
        violations.map(
          (item: { filename: string; line?: number; rule: string }) =>
            `${item.filename}:${item.line ?? "-"} [${item.rule}]`
        )
      ).toEqual([]);
    });
  });

  // ---------------------------------------------------------------------------
  // wave-18b C4-r3（R1 豁免守卫 / R2 位置锚定 / R3 线性化）+ C7 私钥块级豁免
  // 契约基线：.dev-team/contract-matrix-wave18.md wave18-r3 §4.2 / §4.3 / §4.6 / §4.7 / §7
  // 权威证据源：.dev-team/review-l3-scanner-wave18.json finding#1/#2/#3/#4
  // 红相基线：r2 实现的豁免 1/2/3 无 credWord 守卫且按“整行”放行（G1–G6 全 0 命中）；
  //           私钥规则以整文件内容判定 TEST|MOCK|DUMMY|EXAMPLE（100% 失明）；
  //           每 match 重复整行运算 => O(matches × 行长) 二次方。
  // ---------------------------------------------------------------------------
  describe("wave-18b C4-r3 + C7", () => {
    const hex64 = (seed: string): string => seed.repeat(4); // 16 * 4 = 64
    const LOW = hex64("0123456789abcdef");
    const ALT = hex64("fedcba9876543210");
    const BACKTICK = String.fromCharCode(96);

    // ---- §4.7 G1–G6：必须命中（r2 实现下全部 0 命中） ----

    it("G1 必须命中：| PONYSENTRY_CLIENT_TOKEN | <64hex> |（R1 守卫豁免 3）", () => {
      const content = `| PONYSENTRY_CLIENT_TOKEN | ${LOW} |`;
      const hits = scanTextContent(content);
      expect(hits.length).toBe(1);
      expectCaughtSecret(hits, [LOW], redactHexes(content, [LOW, ALT]));
    });

    it("G2 必须命中：| client_token | <64hex> |（R1）", () => {
      const content = `| client_token | ${LOW} |`;
      const hits = scanTextContent(content);
      expect(hits.length).toBe(1);
      expectCaughtSecret(hits, [LOW], redactHexes(content, [LOW, ALT]));
    });

    it("G3 必须命中：{\"client_token\":\"<64hex>\",\"integrity\":\"sha512-x\"}（R1 守卫豁免 1）", () => {
      const content = `{"client_token":"${LOW}","integrity":"sha512-x"}`;
      const hits = scanTextContent(content);
      expect(hits.length).toBe(1);
      expectCaughtSecret(hits, [LOW], redactHexes(content, [LOW, ALT]));
    });

    it("G4 必须命中：{\"client_token\":\"<64hex>\",\"checksum\":\"abc\"}（R1）", () => {
      const content = `{"client_token":"${LOW}","checksum":"abc"}`;
      const hits = scanTextContent(content);
      expect(hits.length).toBe(1);
      expectCaughtSecret(hits, [LOW], redactHexes(content, [LOW, ALT]));
    });

    it("G5 必须命中：client_token=<64hex> // hash（R1）", () => {
      const content = `client_token=${LOW} // hash`;
      const hits = scanTextContent(content);
      expect(hits.length).toBe(1);
      expectCaughtSecret(hits, [LOW], redactHexes(content, [LOW, ALT]));
    });

    it("G6 意图断言：<64hex>  PONYSENTRY_CLIENT_TOKEN=<64hex> 中被捕获的必须是第二个（具名 token）", () => {
      const content = `${LOW}  PONYSENTRY_CLIENT_TOKEN=${ALT}`;
      const hits = scanTextContent(content);
      // 计数口径：r3/r4 契约 §4.7 原文均为「≥1 命中」。
      // （wave-18b 曾自行加严为 `=== 2`；r4 R1′ 改位置敏感守卫后该行仅第二个 hex 判违规，
      //  Lead 裁定 A：计数对齐契约原文，改为 ≥1，并补下方「抓到哪一个」的意图断言。）
      expect(hits.length).toBeGreaterThanOrEqual(1);
      // 意图断言：H1 落在 sha256sum 前缀槽被合法豁免；被捕获的必须是 H2（具名 token），H1 不得被捕获
      expect(hits.map((hit) => hit.masked)).toEqual([maskedOfSecretValue(ALT)]);
      expect(hits.some((hit) => hit.masked === maskedOfSecretValue(LOW))).toBe(false);
    });

    it("R2 必须命中：无凭证语义词时，一行内第二个 hex 不得继承豁免（sha256sum 前缀 / Markdown 单元格）", () => {
      // §4.3 R2：豁免 2/3 位置锚定，只豁免该位置的那个 hex；一行内其他位置的 64-hex 不继承豁免。
      // 本用例刻意不含任何凭证语义词，以**隔离 R2**（G6 在 R1 生效后由 R1 主导，无法单独证伪 R2）。
      // 意图断言：被捕获的必须是第二个（非槽位）hex。
      const sumPrefix = `${LOW}  crates/foo.rs ${ALT}`;
      const cell = `| ${LOW} | ${ALT} |`;
      expectCaughtSecret(scanTextContent(sumPrefix), [ALT], redactHexes(sumPrefix, [LOW, ALT]));
      expectCaughtSecret(scanTextContent(cell), [ALT], redactHexes(cell, [LOW, ALT]));
    });

    // ---- 反向：无凭证语义词的纯摘要上下文必须仍 0 命中（零误报不得回退） ----

    it("反向 0 命中：无凭证语义词的纯摘要上下文（checksum / sha256sum / 单元格 / 反引号 / computedHash / fingerprint）", () => {
      const cases: Array<[string, string]> = [
        ["checksum = \"<hex>\"", `checksum = "${LOW}"`],
        ["sha256sum 输出行", `${LOW}  crates/pony-agent-core/src/lib.rs`],
        ["Markdown 纯哈希单元格", `| ${LOW} |`],
        ["反引号独占摘要", `- 收据：${BACKTICK}${LOW}${BACKTICK}`],
        ["computedHash", `"computedHash": "${LOW}"`],
        ["fingerprint", `fingerprint: ${LOW}`],
      ];
      const offenders = cases
        .filter(([, content]) => scanTextContent(content).length !== 0)
        .map(([label]) => label);
      expect(offenders).toEqual([]);
    });

    // ---- C7 §7.4：私钥块级豁免（隔离临时目录，不污染仓库） ----

    const PK_BEGIN = "-----BEGIN RSA PRIVATE KEY-----";
    const PK_END = "-----END RSA PRIVATE KEY-----";
    const PK_BODY = "MIIEowIBAAKCAQEA0Y1";
    const REAL_BLOCK = `${PK_BEGIN}\n${PK_BODY}\n${PK_END}`;

    let fixtureDir = "";

    beforeAll(() => {
      fixtureDir = fs.mkdtempSync(path.join(os.tmpdir(), "w18b-secrets-"));
    });

    afterAll(() => {
      if (fixtureDir) {
        fs.rmSync(fixtureDir, { recursive: true, force: true });
      }
    });

    /** 写入隔离临时文件并按其真实路径扫描（谁创建谁清理，禁止污染仓库）。 */
    function scanFixture(name: string, content: string) {
      const filePath = path.join(fixtureDir, name);
      fs.writeFileSync(filePath, content, "utf8");
      return scanTextContent(fs.readFileSync(filePath, "utf8"), filePath);
    }

    it("C7-1 真实私钥块 + 同文件远端出现 test 字样 => 仍必须命中（当前 100% 漏报）", () => {
      const hits = scanFixture("real-key-with-remote-test.env.txt", `${REAL_BLOCK}\n// unit test fixture`);
      expect(hits.length).toBeGreaterThanOrEqual(1);
      expect(hits[0].rule).toContain("Private Key Block");
    });

    it("C7-2 私钥 base64 体中偶然含 TEST 字样 => 不得因此豁免", () => {
      const hits = scanFixture(
        "real-key-body-test.env.txt",
        `${PK_BEGIN}\nMIIEowTESTAAKCAQEA0Y1\n${PK_END}`
      );
      expect(hits.length).toBeGreaterThanOrEqual(1);
    });

    it("C7-3 多私钥块：前一块 mock 豁免、后一块真实 => 真实块必须命中（无首块歧义）", () => {
      const content = `// MOCK TEST KEY\n${REAL_BLOCK}\n\n\n\n${REAL_BLOCK}`;
      const hits = scanFixture("multi-block.env.txt", content);
      expect(hits.length).toBe(1);
    });

    it("C7-4 反向：// MOCK TEST KEY 紧邻 BEGIN 上一行 => 必须仍 0 命中（spec:83-92 不得破坏）", () => {
      expect(scanFixture("mock-key.env.txt", `// MOCK TEST KEY\n${REAL_BLOCK}`).length).toBe(0);
    });

    it("C7-5 反向：// EXAMPLE KEY 紧邻 BEGIN 上一行 => 必须仍 0 命中", () => {
      expect(scanFixture("example-key.env.txt", `// EXAMPLE KEY\n${REAL_BLOCK}`).length).toBe(0);
    });

    // ---- §4.6 性能硬约束（R3 线性化） ----

    it("C4-3 性能：4000×64hex 单行必须 ≤100ms，且命中数与拆行版本一致（§4.6 验收判据）", () => {
      const parts = Array.from({ length: 4000 }, () => LOW);
      const singleLine = parts.join(",");
      const splitLines = parts.join("\n");

      const started = performance.now();
      const singleHits = scanTextContent(singleLine);
      const elapsed = performance.now() - started;

      expect(singleHits.length).toBe(4000);
      expect(elapsed).toBeLessThanOrEqual(100);
      expect(scanTextContent(splitLines).length).toBe(4000);
    }, 120_000);

    it("C4-3 性能：约 1MB 单行 15000×64hex 必须 <3000ms，且命中数正确（不得为 0 或洪水式重复）", () => {
      const parts = Array.from({ length: 15000 }, () => LOW);
      const singleLine = parts.join(",");
      // 纯 ASCII：length === 字节数；975,000 B ≈ 1MB
      expect(singleLine.length).toBeGreaterThanOrEqual(900_000);

      const started = performance.now();
      const hits = scanTextContent(singleLine);
      const elapsed = performance.now() - started;

      expect(hits.length).toBe(15000);
      expect(elapsed).toBeLessThan(3000);
    }, 300_000);
  });

  // ---------------------------------------------------------------------------
  // wave-18c C4-R1′ / R2′ / R3′ / R4（r4 修订的红相扩展）
  // 权威证据源：
  //   .dev-team/review-wave18b-scanner.json finding#1（NEW-FP 回归）
  //   finding#2（R2 实现为「值锚定」而非「位置锚定」）
  //   finding#3（NEW-HIGH-DoS 残余二次方）
  //   finding#4（CRED_WORD_RE 空格分隔语义词缺口）
  // 红相基线（当前 r3 实现）：
  //   R1′ 语义词在 hex **之后**（sha256sum 收据的路径/文件名）时被误判违规；
  //   R2′ `f.prefixHex === m` / `f.cellHex === m` 按**值**豁免，同值第二个 hex 被吞；
  //   R3′ `context.split("\n").pop()` 每 match 执行一次 ⇒ O(matches × 行长)；
  //   R4  `client token` / `access token` / `api key` / 裸 `token` 不在 CRED_WORD_RE 内。
  // ---------------------------------------------------------------------------
  describe("wave-18c C4-R1′/R2′/R3′/R4", () => {
    const hex64 = (seed: string): string => seed.repeat(4); // 16 * 4 = 64
    const LOW = hex64("0123456789abcdef");
    const ALT = hex64("fedcba9876543210");

    /** 生成互不相同的 64-hex（避免「值锚定」把同值全部豁免而掩盖命中数判别力）。 */
    function distinctHex64(index: number): string {
      return index.toString(16).padStart(8, "0").repeat(8);
    }

    /** 失败信息中不出现完整 64-hex 字面量（避免证据二次泄漏）。 */
    function mask(content: string): string {
      return redactHexes(content, [LOW, ALT]);
    }

    // ---- C4-R1′ 新误报回归（r4 §4.8 FP1–FP6，必须 0 命中；r3 实测全部 = 1） ----

    it("R1′/§4.8 必须 0 命中：新误报回归面 FP1–FP6（摘要收据 + 同行凭证语义词）", () => {
      const BACKTICK = String.fromCharCode(96);
      const cases: Array<[string, string]> = [
        ["FP1 sha256sum 收据，文件名含 secret", `${LOW}  scripts/rotate-secret.sh`],
        ["FP2 JSON 收据，字段名/文件名含 secret", `{"file":"secrets.json","sha256":"${LOW}"}`],
        ["FP3 说明文字含 secret", `- 密钥指纹(secret fingerprint): ${LOW}`],
        ["FP4 收据路径含 credential", `${LOW}  config/credential-store.json`],
        ["FP5 收据路径含 password", `${LOW}  scripts/reset-password.sh`],
        ["FP6 反引号摘要 + 同行说明含 SECRET", `| ${BACKTICK}${LOW}${BACKTICK} | sha256 of SECRET_RULES sample |`],
        ["同族边界：token-rotation.md", `${LOW}  docs/token-rotation.md`],
        ["同族边界：credential-store.ts", `${LOW}  src/credential-store.ts`],
        ["同族边界：a/b/secret.key", `${LOW}  a/b/secret.key`],
      ];
      const offenders = cases
        .filter(([, content]) => scanTextContent(content, "").length !== 0)
        .map(([label, content]) => `${label} :: ${mask(content)}`);
      expect(offenders).toEqual([]);
    });

    it("R1′/§4.8 必须 0 命中：既有良性形态与长度窗口回归 FP7–FP14（不得回退）", () => {
      const BACKTICK = String.fromCharCode(96);
      const cases: Array<[string, string]> = [
        ["FP7 checksum 上下文", `checksum = "${LOW}"`],
        ["FP8 sha256sum 输出（无凭证词路径）", `${LOW}  /tmp/pony-run-e2e/shot-30s.png`],
        ["FP9 纯哈希单元格", `| ${LOW} |`],
        ["FP10 反引号摘要", `- 修订后 sha256：${BACKTICK}${LOW}${BACKTICK}`],
        ["FP11 git SHA-40", `commit ${"a".repeat(40)} fix`],
        ["FP12 git SHA-7", "commit 17dca7c fix"],
        ["FP13 63 位 hex", `x=${"a".repeat(63)}`],
        ["FP14 65 位 hex", `x=${"a".repeat(64)}a`],
      ];
      const offenders = cases
        .filter(([, content]) => scanTextContent(content, "").length !== 0)
        .map(([label, content]) => `${label} :: ${mask(content)}`);
      expect(offenders).toEqual([]);
    });

    it("R1′ 位置敏感判据：语义词在 hex 之前必须命中，在 hex 之后（收据文件名）必须 0 命中", () => {
      // 同一语义词 `secret`，仅位置不同 ⇒ 判定必须相反
      const before = `secret = "${LOW}"`;
      const after = `${LOW}  secret.txt`;
      expectCaughtSecret(scanTextContent(before, ""), [LOW], mask(before));
      expect(scanTextContent(after, "").length, `输入 ${mask(after)} 必须 0 命中`).toBe(0);
    });

    it("R1′ 必须 ≥1 命中（防过度纠正）：凭证语义词位于 hex 之前的各类形态", () => {
      const cases = [
        `client_token=${LOW}`,
        `{"client_token":"${LOW}"}`,
        `| PONYSENTRY_CLIENT_TOKEN | ${LOW} |`,
        `api_key: ${LOW}`,
        `secret = "${LOW}"`,
      ];
      for (const content of cases) {
        // 意图断言：命中的必须是行内那个 hex，且来自 64-hex 规则
        expectCaughtSecret(scanTextContent(content, ""), [LOW], mask(content));
      }
    });

    // ---- C4-R2′ 位置锚定（豁免 2/3 只豁免「该位置」的那个 hex） ----

    it("R2′ 必须恰好 1 命中：同一 hex 重复出现时，仅「非槽位」那一个被捕获（第二个不得继承豁免）", () => {
      // 同值重复 ⇒ 掩码无法区分位置；但「恰好 1 条」已排除 0（值锚定全豁免）与 2（槽位也被判违规），
      // 位置归属由下方异值用例（G15/G16 形态）的掩码断言证明。
      const sameSumPrefix = `${LOW}  ${LOW}`;
      const sameCell = `| ${LOW} | ${LOW} |`;
      expect(scanTextContent(sameSumPrefix, "").length, mask(sameSumPrefix)).toBe(1);
      expect(scanTextContent(sameCell, "").length, mask(sameCell)).toBe(1);
    });

    it("R2′ 意图断言：异值重复时，被捕获的必须是第二个（非槽位）hex —— 证明按位置而非按值锚定", () => {
      const sumPrefix = `${LOW}  ${ALT}`;
      const cell = `| ${LOW} | ${ALT} |`;
      expectCaughtSecret(scanTextContent(sumPrefix, ""), [ALT], mask(sumPrefix));
      expectCaughtSecret(scanTextContent(cell, ""), [ALT], mask(cell));
    });

    it("R2′ 反向：单 hex 的合法摘要槽仍必须 0 命中（防过度纠正）", () => {
      expect(scanTextContent(`${LOW}  path/to/file.txt`, "").length).toBe(0);
      expect(scanTextContent(`| ${LOW} |`, "").length).toBe(0);
    });

    // ---- C4-R4 残余漏报（空格分隔 / 裸 token 语义词） ----

    it("R4 必须 ≥1 命中：空格分隔或裸 token 的凭证标签单元格", () => {
      const cases = [
        `| client token | ${LOW} |`,
        `| access token | ${LOW} |`,
        `| api key | ${LOW} |`,
        `| token | ${LOW} |`,
      ];
      for (const content of cases) {
        // 意图断言：命中必须落在行内那个 hex 上
        expectCaughtSecret(scanTextContent(content, ""), [LOW], mask(content));
      }
    });

    it("R4 反向：无凭证语义词的表格单元格仍必须 0 命中（防过度纠正 / 零误报不回退）", () => {
      const cases = [
        `| commit sha | ${LOW} |`,
        `| digest | ${LOW} |`,
        `| checksum | ${LOW} |`,
        `| file | ${LOW} |`,
      ];
      const offenders = cases
        .filter((content) => scanTextContent(content, "").length !== 0)
        .map(mask);
      expect(offenders).toEqual([]);
    });

    // ---- C4-R3′ 性能硬约束（消除残余二次方） ----

    it("R3′ 性能：单行 5MB（77000×64hex 互不相同）必须 ≤2000ms 且命中数正确", () => {
      const count = 77000;
      const parts = new Array<string>(count);
      for (let index = 0; index < count; index += 1) {
        parts[index] = distinctHex64(index);
      }
      const singleLine = parts.join(" ");
      // 5,004,999 B ≈ 5MB；纯 ASCII ⇒ length === 字节数
      expect(singleLine.length).toBeGreaterThanOrEqual(5_000_000);

      const started = performance.now();
      const violations = scanTextContent(singleLine, "");
      const elapsed = performance.now() - started;

      // 首个 hex 位于 sha256sum 前缀槽（合法豁免），其余 count-1 条必须全部命中
      expect(violations.length).toBe(count - 1);
      expect(elapsed).toBeLessThanOrEqual(2000);
    }, 300_000);

    it("R3′ 性能：单行 10MB（154000×64hex 互不相同）必须 ≤5000ms 且命中数正确", () => {
      const count = 154000;
      const parts = new Array<string>(count);
      for (let index = 0; index < count; index += 1) {
        parts[index] = distinctHex64(index);
      }
      const singleLine = parts.join(" ");
      // 10,009,999 B ≈ 10MB
      expect(singleLine.length).toBeGreaterThanOrEqual(10_000_000);

      const started = performance.now();
      const violations = scanTextContent(singleLine, "");
      const elapsed = performance.now() - started;

      expect(violations.length).toBe(count - 1);
      expect(elapsed).toBeLessThanOrEqual(5000);
    }, 900_000);
  });
});
