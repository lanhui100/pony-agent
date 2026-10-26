import { describe, it, expect } from "vitest";
import { execSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import os from "node:os";

const SCRIPT_PATH = "/home/dm/.agents/skills/observability-onboarding/scripts/audit-diff.sh";
const REPO_ROOT = path.resolve(__dirname, "../..");

// 本套件直驱本机 skill（audit-diff）与 ponygo，二者都只存在于开发机；
// CI runner 上没有，硬跑会得到 bash 127 的假红。故整体在缺依赖时跳过——
// CI 侧的等价覆盖由 gate-conformance.spec.ts（gate.mjs 引擎，含 D1 语义回归）承担。
const hasSkill = fs.existsSync(SCRIPT_PATH);
const hasPonygo = (() => {
  try {
    execSync("command -v ponygo", { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
})();

describe.skipIf(!hasSkill || !hasPonygo)("ob2 变更级门禁与横切收口点验收测试 (L2-T & L2-AT)", () => {
  it("T1-BASELINE: 当前工作区在 --decl-approved 下基线合规退出 0", () => {
    const res = execSync(`bash "${SCRIPT_PATH}" --repo "${REPO_ROOT}" --decl-approved 2>&1`, {
      encoding: "utf-8",
    });
    expect(res).toContain("[OK] audit-diff: 无越界实现、收口点完整");
  });

  it("T2-D1-UNAPPROVED: 声明文件变更未传 --decl-approved 时必须拦截 (退出码 2)", () => {
    // 自包含影子仓库：不依赖本仓库工作区是否恰好有未提交的声明改动
    // （旧实现断言的是宿主仓状态，声明一旦提交该用例即假红——属测试缺陷，已按契约演进重锚）
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "ob2-d1-unapproved-"));
    try {
      execSync(`git init -b main "${tmpDir}"`, { stdio: "ignore" });
      execSync(`git config user.email "test@test.local"`, { cwd: tmpDir, stdio: "ignore" });
      execSync(`git config user.name "Tester"`, { cwd: tmpDir, stdio: "ignore" });

      fs.mkdirSync(path.join(tmpDir, "docs"), { recursive: true });
      const declPath = path.join(tmpDir, "docs/observability.yaml");
      const declContent = `service: test-svc\nkind: desktop\nreviewed_at: "2026-10-26"\ndimensions:\n  tracing:\n    applicable: false\n    reason: n/a\n  metrics:\n    applicable: false\n    reason: n/a\n  logs:\n    applicable: false\n    reason: n/a\n  issues:\n    applicable: false\n    reason: n/a\n`;
      fs.writeFileSync(declPath, declContent, "utf-8");
      execSync(`git add . && git commit -m "baseline [decl-change-approved]"`, { cwd: tmpDir, stdio: "ignore" });

      // 破坏动作：仅改声明文件，且不传 --decl-approved
      fs.appendFileSync(declPath, "note: unapproved change\n", "utf-8");

      let exitCode = 0;
      let stderr = "";
      try {
        execSync(`bash "${SCRIPT_PATH}" --repo "${tmpDir}"`, {
          encoding: "utf-8",
          stdio: "pipe",
        });
      } catch (err: any) {
        exitCode = err.status;
        stderr = err.stderr?.toString() || "";
      }
      expect(exitCode).toBe(2);
      expect(stderr).toContain("[FAIL] [D1] 声明在本次变更中被修改，需要显式批准");
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  it("T3-RULE-A-FRONTEND: 前端非法调用 onErrorCaptured 破坏测试必须拦截 (退出码 2)", () => {
    // 构造临时未跟踪的测试组件，写入非收口点的 onErrorCaptured
    const dummyFile = path.join(REPO_ROOT, "src/components/common/TempForbiddenBoundary.vue");
    try {
      fs.writeFileSync(
        dummyFile,
        `<script setup lang="ts">\nimport { onErrorCaptured } from "vue";\nonErrorCaptured(() => {});\n</script>`,
        "utf-8"
      );

      let exitCode = 0;
      let stderr = "";
      try {
        execSync(`bash "${SCRIPT_PATH}" --repo "${REPO_ROOT}" --decl-approved`, {
          encoding: "utf-8",
          stdio: "pipe",
        });
      } catch (err: any) {
        exitCode = err.status;
        stderr = err.stderr?.toString() || "";
      }
      expect(exitCode).toBe(2);
      expect(stderr).toMatch(/\[FAIL\] \[D2\].*forbidden pattern/);
      expect(stderr).toContain("TempForbiddenBoundary.vue");
      expect(stderr).toContain("error_boundary");
    } finally {
      if (fs.existsSync(dummyFile)) {
        fs.unlinkSync(dummyFile);
      }
    }
  });

  it("T4-RULE-A-CORE: 核心层手写 traceparent header 破坏测试必须拦截 (退出码 2)", () => {
    const dummyFile = path.join(
      REPO_ROOT,
      "crates/pony-agent-core/src/agent/temp_forbidden_client.rs"
    );
    try {
      fs.writeFileSync(
        dummyFile,
        `// Forbidden manual traceparent injection\nfn send_req() {\n    let _ = req.header("traceparent", "00-test-01");\n}\n`,
        "utf-8"
      );

      let exitCode = 0;
      let stderr = "";
      try {
        execSync(`bash "${SCRIPT_PATH}" --repo "${REPO_ROOT}" --decl-approved`, {
          encoding: "utf-8",
          stdio: "pipe",
        });
      } catch (err: any) {
        exitCode = err.status;
        stderr = err.stderr?.toString() || "";
      }
      expect(exitCode).toBe(2);
      expect(stderr).toMatch(/\[FAIL\] \[D2\].*forbidden pattern/);
      expect(stderr).toContain("temp_forbidden_client.rs");
      expect(stderr).toContain("http_client");
    } finally {
      if (fs.existsSync(dummyFile)) {
        fs.unlinkSync(dummyFile);
      }
    }
  });

  it("T5-RULE-B-PATH-DESTRUCTION: 收口点被清空/删除对抗注入必须拦截 (退出码 2)", () => {
    // 构造影子仓库测试收口点失效逻辑
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "ob2-rule-b-"));
    try {
      // 创建最小 git 仓库
      execSync(`git init -b main "${tmpDir}"`, { stdio: "ignore" });
      execSync(`git config user.email "test@test.local"`, { cwd: tmpDir, stdio: "ignore" });
      execSync(`git config user.name "Tester"`, { cwd: tmpDir, stdio: "ignore" });

      // 创建合规收口点并 commit 作为基线
      fs.mkdirSync(path.join(tmpDir, "docs"), { recursive: true });
      fs.mkdirSync(path.join(tmpDir, "src"), { recursive: true });
      const epFile = path.join(tmpDir, "src/ep.ts");
      fs.writeFileSync(epFile, "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10\n", "utf-8");

      const declContent = `service: test-svc\nkind: desktop\nreviewed_at: "2026-10-26"\ndimensions:\n  tracing:\n    applicable: false\n    reason: n/a\n  metrics:\n    applicable: false\n    reason: n/a\n  logs:\n    applicable: false\n    reason: n/a\n  issues:\n    applicable: false\n    reason: n/a\nenforcement_points:\n  test_ep:\n    path: src/ep.ts\n    guarantees: [timeout]\n    forbidden_in_other_files:\n      - pattern: "\\bforbidden_call\\b"\n`;
      fs.writeFileSync(path.join(tmpDir, "docs/observability.yaml"), declContent, "utf-8");

      execSync(`git add . && git commit -m "baseline [decl-change-approved]"`, { cwd: tmpDir, stdio: "ignore" });

      // 破坏动作：将 ep.ts 现存行数削减为 2 行（< 基线的 50%）
      fs.writeFileSync(epFile, "line1\nline2\n", "utf-8");

      let exitCode = 0;
      let stderr = "";
      try {
        execSync(`bash "${SCRIPT_PATH}" --repo "${tmpDir}"`, {
          encoding: "utf-8",
          stdio: "pipe",
        });
      } catch (err: any) {
        exitCode = err.status;
        stderr = err.stderr?.toString() || "";
      }
      expect(exitCode).toBe(2);
      expect(stderr).toMatch(/收口点文件被清空|收口点失效/);
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  it("T6-EXEMPTION: 收口点自身修改允许命中自己的 forbidden pattern (白名单放行 Exit 0)", () => {
    // 构造影子仓库测试合法收口点豁免逻辑
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "ob2-exempt-"));
    try {
      execSync(`git init -b main "${tmpDir}"`, { stdio: "ignore" });
      execSync(`git config user.email "test@test.local"`, { cwd: tmpDir, stdio: "ignore" });
      execSync(`git config user.name "Tester"`, { cwd: tmpDir, stdio: "ignore" });

      fs.mkdirSync(path.join(tmpDir, "docs"), { recursive: true });
      fs.mkdirSync(path.join(tmpDir, "src"), { recursive: true });
      const epFile = path.join(tmpDir, "src/ep.ts");
      fs.writeFileSync(epFile, "line1\nline2\nline3\nline4\nline5\nline6\nline7\nline8\nline9\nline10\n", "utf-8");

      const declContent = `service: test-svc\nkind: desktop\nreviewed_at: "2026-10-26"\ndimensions:\n  tracing:\n    applicable: false\n    reason: n/a\n  metrics:\n    applicable: false\n    reason: n/a\n  logs:\n    applicable: false\n    reason: n/a\n  issues:\n    applicable: false\n    reason: n/a\nenforcement_points:\n  test_ep:\n    path: src/ep.ts\n    guarantees: [timeout]\n    forbidden_in_other_files:\n      - pattern: "\\bforbidden_call\\b"\n`;
      fs.writeFileSync(path.join(tmpDir, "docs/observability.yaml"), declContent, "utf-8");

      execSync(`git add . && git commit -m "baseline [decl-change-approved]"`, { cwd: tmpDir, stdio: "ignore" });

      // 在收口点 src/ep.ts 内部加入命中模式的内容（这是合法的唯一收口实现）
      fs.appendFileSync(epFile, "function ep_worker() { forbidden_call(); }\n", "utf-8");

      let output = "";
      try {
        output = execSync(`bash "${SCRIPT_PATH}" --repo "${tmpDir}" 2>&1`, {
          encoding: "utf-8",
        });
      } catch (err: any) {
        output = (err.stdout?.toString() || "") + (err.stderr?.toString() || "");
      }
      expect(output).toContain("[OK] audit-diff: 无越界实现、收口点完整");
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  it("T7-PONYGO-OB2-GATE: ponygo observe audit --level ob2 物理门禁为绿相 (Exit 0)", () => {
    let exitCode = 0;
    let stdout = "";
    try {
      stdout = execSync("ponygo observe audit --level ob2", {
        encoding: "utf-8",
        stdio: "pipe",
        timeout: 60000,
      });
    } catch (err: any) {
      exitCode = err.status ?? -1;
      stdout = err.stdout?.toString() || "";
    }
    // 绿相物理断言：ob2 收口与变更门禁已达标
    expect(exitCode).toBe(0);
    expect(stdout).toContain("[PASS] ob2 收口与变更门禁");
    expect(stdout).toContain("当前级: ob2");
    expect(stdout).toContain("目标级: ob2");
  }, 90000);
});
