import { describe, it, expect } from "vitest";
import {
  scanTextContent,
  checkBlockedFilename,
  isSafeFilename,
  SECRET_RULES
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
});
