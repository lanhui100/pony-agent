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
      expect(scanTextContent(`| PONYSENTRY_CLIENT_TOKEN | ${LOW} |`).length).toBe(1);
    });

    it("G2 必须命中：| client_token | <64hex> |（R1）", () => {
      expect(scanTextContent(`| client_token | ${LOW} |`).length).toBe(1);
    });

    it("G3 必须命中：{\"client_token\":\"<64hex>\",\"integrity\":\"sha512-x\"}（R1 守卫豁免 1）", () => {
      expect(
        scanTextContent(`{"client_token":"${LOW}","integrity":"sha512-x"}`).length
      ).toBe(1);
    });

    it("G4 必须命中：{\"client_token\":\"<64hex>\",\"checksum\":\"abc\"}（R1）", () => {
      expect(scanTextContent(`{"client_token":"${LOW}","checksum":"abc"}`).length).toBe(1);
    });

    it("G5 必须命中：client_token=<64hex> // hash（R1）", () => {
      expect(scanTextContent(`client_token=${LOW} // hash`).length).toBe(1);
    });

    it("G6 必须命中：<64hex>  PONYSENTRY_CLIENT_TOKEN=<64hex>（R1 + R2：第二个 hex 不得被吞）", () => {
      // R2 位置锚定：豁免 2（sha256sum 前缀）只能豁免该行首个 hex；
      // 且 R1 规定同行出现凭证语义词时全部豁免失效 => 一行两个 hex 必须两条都命中。
      expect(scanTextContent(`${LOW}  PONYSENTRY_CLIENT_TOKEN=${ALT}`).length).toBe(2);
    });

    it("R2 必须命中：无凭证语义词时，一行内第二个 hex 不得继承豁免（sha256sum 前缀 / Markdown 单元格）", () => {
      // §4.3 R2：豁免 2/3 位置锚定，只豁免该位置的那个 hex；一行内其他位置的 64-hex 不继承豁免。
      // 本用例刻意不含任何凭证语义词，以**隔离 R2**（G6 在 R1 生效后由 R1 主导，无法单独证伪 R2）。
      expect(scanTextContent(`${LOW}  crates/foo.rs ${ALT}`).length).toBe(1);
      expect(scanTextContent(`| ${LOW} | ${ALT} |`).length).toBe(1);
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
});
