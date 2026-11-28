#!/usr/bin/env node
/**
 * scripts/check-file-size.spec.mjs —— 文件规模门的**豁免语法**正负样本自检。
 *
 * 为什么单独一条（B1795831357716，2026-11-28）：
 * 事故形态不是「阈值被绕过」，而是「豁免语法写错被当成永久豁免」。帮助文本曾把
 * `[until YYYY-MM-DD]` 写成示例，而解析器只认无方括号形态 ⇒ 照抄示例的两个巨石文件
 * （pony-tui/src/tui.rs 14,740 行、pony-tools/src/workspace_hotloader.rs 3,920 行）
 * 的到期日形同虚设。这类失效**不改变任何可见输出**（门照样绿），只能靠负样本钉住。
 *
 * 全部用例在 mkdtemp 沙箱里跑，不触碰仓库真实文件。
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SCRIPT = path.join(REPO_ROOT, "scripts", "check-file-size.mjs");
const MAX_LINES = 3000;
const OVER_LIMIT_BODY = "// line\n".repeat(MAX_LINES + 200);

function runGate(root) {
  return spawnSync(
    process.execPath,
    [SCRIPT, "--root", root, "--max-file-lines", String(MAX_LINES)],
    { cwd: REPO_ROOT, encoding: "utf8" }
  );
}

/** 建沙箱并写入唯一一个超限 .rs，返回 { dir, write(header) }。 */
function makeSandbox() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "file-size-spec-"));
  return {
    dir,
    write(firstLine) {
      const body = firstLine === null ? "" : `${firstLine}\n`;
      fs.writeFileSync(path.join(dir, "probe.rs"), body + OVER_LIMIT_BODY, "utf8");
    },
  };
}

const sandbox = makeSandbox();
let failures = 0;

function check(name, fn) {
  try {
    fn();
    console.log(`  PASS  ${name}`);
  } catch (error) {
    failures += 1;
    console.error(`  FAIL  ${name}\n        ${error.message}`);
  }
}

try {
  // P1：无豁免 → 超限必须判红（门本身不空转）
  check("P1 无豁免的超限文件判红（exit 1）", () => {
    sandbox.write(null);
    const res = runGate(sandbox.dir);
    assert.equal(res.status, 1, `期望 exit 1，实际 ${res.status}\n${res.stdout}`);
  });

  // P2：合法未到期豁免 → 放行（exit 0）
  check("P2 `until <未来日期> <理由>` 放行（exit 0）", () => {
    sandbox.write("// ponylint:file-size-exempt until 2099-12-31 沙箱探针");
    const res = runGate(sandbox.dir);
    assert.equal(res.status, 0, `期望 exit 0，实际 ${res.status}\n${res.stdout}`);
  });

  // N1（事故原形）：方括号写法 → 必须判红，不得退化成永久豁免
  check("N1 `[until <未来日期>]` 方括号写法判红（exit 1）", () => {
    sandbox.write("// ponylint:file-size-exempt [until 2099-12-31] 沙箱探针");
    const res = runGate(sandbox.dir);
    assert.equal(res.status, 1, `期望 exit 1，实际 ${res.status}\n${res.stdout}`);
    assert.match(res.stdout, /方括号/, "错误信息必须点名方括号写法");
  });

  // N2：已过期豁免 → 判红
  check("N2 `until <过期日期>` 判红（exit 1）", () => {
    sandbox.write("// ponylint:file-size-exempt until 2020-01-01 沙箱探针");
    const res = runGate(sandbox.dir);
    assert.equal(res.status, 1, `期望 exit 1，实际 ${res.status}\n${res.stdout}`);
  });

  // N3：只写 until 不给理由 → 判红
  check("N3 `until <未来日期>` 缺理由判红（exit 1）", () => {
    sandbox.write("// ponylint:file-size-exempt until 2099-12-31");
    const res = runGate(sandbox.dir);
    assert.equal(res.status, 1, `期望 exit 1，实际 ${res.status}\n${res.stdout}`);
  });

  // N4：裸豁免（无 until）→ 仍按永久豁免放行，但这是**已知设计**（不设日期不算违规）
  check("P3 裸豁免（无 until）按永久豁免放行（exit 0，既有语义）", () => {
    sandbox.write("// ponylint:file-size-exempt 沙箱探针");
    const res = runGate(sandbox.dir);
    assert.equal(res.status, 0, `期望 exit 0，实际 ${res.status}\n${res.stdout}`);
  });
} finally {
  fs.rmSync(sandbox.dir, { recursive: true, force: true });
}

if (failures > 0) {
  console.error(`[file-size-spec] FAILED：${failures} 条`);
  process.exit(1);
}
console.log("[file-size-spec] PASS：豁免语法三态（无/合法/方括号/过期/缺理由）判定全部符合预期");
