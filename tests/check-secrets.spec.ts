import { describe, it, expect } from "vitest";
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
});
