/**
 * Wave-47 · 双引擎一致性（conformance）
 *
 * CI 在 GitHub runner 上跑仓库内引擎 `scripts/observability/gate.mjs`（runner 无本机 skill），
 * 本机则跑权威器 `audit-diff.sh`。**引擎可以有两份，判定口径不能有两份** ——
 * 本套件用同一批夹具同时喂给两个引擎，断言退出码逐例一致。
 *
 * 若哪天一致性破了，说明有人改了一边而没改另一边，必须以权威器为准修正仓库内引擎。
 */
import { describe, it, expect } from "vitest";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const AUDIT_DIFF = path.join(
  os.homedir(),
  ".agents/skills/observability-onboarding/scripts/audit-diff.sh"
);
const GATE = path.resolve(__dirname, "../../../scripts/observability/gate.mjs");
const skillAvailable = fs.existsSync(AUDIT_DIFF);

const DECL = `service: conformance-svc
kind: desktop
reviewed_at: "2026-10-26"
dimensions:
  tracing:
    applicable: false
    reason: n/a
  metrics:
    applicable: false
    reason: n/a
  logs:
    applicable: false
    reason: n/a
  issues:
    applicable: false
    reason: n/a
enforcement_points:
  test_ep:
    path: src/ep.ts
    guarantees:
      - timeout
    forbidden_in_other_files:
      - pattern: "\\\\bforbidden_call\\\\b"
        hint: 只能在收口点实现
`;

function sh(cmd, args, cwd) {
  try {
    const stdout = execFileSync(cmd, args, { cwd, encoding: "utf-8", stdio: "pipe" });
    return { code: 0, out: stdout };
  } catch (err) {
    return {
      code: err.status ?? -1,
      out: `${err.stdout ?? ""}${err.stderr ?? ""}`
    };
  }
}

/** 建一个最小仓库：基线提交含声明 + 收口点文件。 */
function makeFixture() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "wave47-conformance-"));
  execFileSync("git", ["init", "-b", "main", dir], { stdio: "ignore" });
  execFileSync("git", ["config", "user.email", "t@t.local"], { cwd: dir, stdio: "ignore" });
  execFileSync("git", ["config", "user.name", "T"], { cwd: dir, stdio: "ignore" });

  fs.mkdirSync(path.join(dir, "docs"), { recursive: true });
  fs.mkdirSync(path.join(dir, "src"), { recursive: true });
  fs.writeFileSync(path.join(dir, "docs/observability.yaml"), DECL, "utf-8");
  fs.writeFileSync(path.join(dir, "src/ep.ts"), Array.from({ length: 20 }, (_, i) => `line${i + 1}`).join("\n") + "\n", "utf-8");

  execFileSync("git", ["add", "."], { cwd: dir, stdio: "ignore" });
  execFileSync("git", ["commit", "-m", "baseline [decl-change-approved]"], { cwd: dir, stdio: "ignore" });
  return dir;
}

function runBoth(dir) {
  const authoritative = sh("bash", [AUDIT_DIFF, "--repo", dir], dir);
  // GATE_BASE=HEAD ⇒ 只看工作区改动，与权威器默认口径对齐
  const repoGate = (() => {
    try {
      const stdout = execFileSync("node", [GATE], {
        cwd: dir,
        encoding: "utf-8",
        stdio: "pipe",
        env: { ...process.env, GATE_REPO: dir, GATE_BASE: "HEAD" }
      });
      return { code: 0, out: stdout };
    } catch (err) {
      return { code: err.status ?? -1, out: `${err.stdout ?? ""}${err.stderr ?? ""}` };
    }
  })();
  return { authoritative, repoGate };
}

describe.skipIf(!skillAvailable)("wave47 双引擎一致性（gate.mjs vs audit-diff.sh）", () => {
  it("C1: 干净工作树 → 两引擎均放行（exit 0）", () => {
    const dir = makeFixture();
    try {
      const { authoritative, repoGate } = runBoth(dir);
      expect(authoritative.code).toBe(0);
      expect(repoGate.code).toBe(0);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it("C2: 非收口点出现 forbidden pattern → 两引擎均拦截（exit 2）", () => {
    const dir = makeFixture();
    try {
      fs.writeFileSync(path.join(dir, "src/rogue.ts"), "export function x() { forbidden_call(); }\n", "utf-8");
      const { authoritative, repoGate } = runBoth(dir);
      expect(authoritative.code).toBe(2);
      expect(repoGate.code).toBe(2);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it("C3: 收口点被删除 → 两引擎均拦截（exit 2）", () => {
    const dir = makeFixture();
    try {
      fs.rmSync(path.join(dir, "src/ep.ts"));
      const { authoritative, repoGate } = runBoth(dir);
      expect(authoritative.code).toBe(2);
      expect(repoGate.code).toBe(2);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });

  it("C4: 收口点自身命中自己的 pattern → 两引擎均放行（豁免一致）", () => {
    const dir = makeFixture();
    try {
      fs.appendFileSync(path.join(dir, "src/ep.ts"), "function w() { forbidden_call(); }\n", "utf-8");
      const { authoritative, repoGate } = runBoth(dir);
      expect(authoritative.code).toBe(0);
      expect(repoGate.code).toBe(0);
    } finally {
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });
});
