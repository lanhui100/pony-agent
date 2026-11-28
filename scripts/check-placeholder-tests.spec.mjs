#!/usr/bin/env node
/**
 * scripts/check-placeholder-tests.spec.mjs —— 恒真占位断言门的正负样本自检。
 *
 * 为什么必须自带负样本（B1795831359205）：本门守护的失效形态是「测试恒绿但什么都没验」，
 * 它**不会**产生任何可见输出差异 —— 门自己写坏了也会安静地全绿。故钉住三件事：
 *   ① 语句位置的 `assert!(true)` / `expect(true).toBe(true)` 必须被拒（exit 1）；
 *   ② **注释里**提到这些写法必须放行（纪律说明大量存在于测试文件头部注释，误报会让门被绕过）；
 *   ③ `--baseline` 登记的行放行，且登记不生效时（行号漂移）必须仍被拒。
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SCRIPT = path.join(REPO_ROOT, "scripts", "check-placeholder-tests.mjs");

/** 在沙箱里造一个最小仓（门从 script 所在目录推导 REPO_ROOT，故须整目录复制）。 */
function makeSandbox() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "placeholder-tests-spec-"));
  fs.mkdirSync(path.join(dir, "scripts"), { recursive: true });
  fs.copyFileSync(SCRIPT, path.join(dir, "scripts", "check-placeholder-tests.mjs"));
  fs.mkdirSync(path.join(dir, "crates", "probe"), { recursive: true });
  return dir;
}

function run(root, extraArgs = []) {
  return spawnSync(
    process.execPath,
    [path.join(root, "scripts", "check-placeholder-tests.mjs"), ...extraArgs],
    { cwd: root, encoding: "utf8" }
  );
}

function write(root, rel, content) {
  const abs = path.join(root, rel);
  fs.mkdirSync(path.dirname(abs), { recursive: true });
  fs.writeFileSync(abs, content, "utf8");
}

let failures = 0;
function check(name, fn) {
  try {
    fn();
    console.log(`  [ok]   ${name}`);
  } catch (error) {
    failures += 1;
    console.error(`  [FAIL] ${name}: ${error.message}`);
  }
}

// ① 负样本：语句位置恒真断言必拒
check("N1 语句位 assert!(true) → exit 1", () => {
  const root = makeSandbox();
  write(root, "crates/probe/t.rs", "#[test]\nfn t() {\n    assert!(true);\n}\n");
  const r = run(root);
  assert.equal(r.status, 1, `期望 exit 1，实际 ${r.status}\n${r.stderr}`);
  assert.match(r.stderr, /crates\/probe\/t\.rs:3/);
});

check("N2 语句位 expect(true).toBe(true) → exit 1", () => {
  const root = makeSandbox();
  write(root, "crates/probe/t.spec.ts", "it('x', () => {\n  expect(true).toBe(true);\n});\n");
  const r = run(root);
  assert.equal(r.status, 1, `期望 exit 1，实际 ${r.status}\n${r.stderr}`);
});

check("N3 assert!(true) 带自定义消息同样拒", () => {
  const root = makeSandbox();
  write(root, "crates/probe/t.rs", "#[test]\nfn t() {\n    assert!(true, \"永远成立\");\n}\n");
  assert.equal(run(root).status, 1);
});

// ② 正样本：注释里的同样字面量必须放行（否则门会误伤纪律说明）
check("P1 行注释中的 assert!(true) → exit 0", () => {
  const root = makeSandbox();
  write(root, "crates/probe/t.rs", "//! 纪律：禁止 `assert!(true)` 式恒真。\n#[test]\nfn t() {\n    assert_eq!(1 + 1, 2);\n}\n");
  const r = run(root);
  assert.equal(r.status, 0, `期望 exit 0，实际 ${r.status}\n${r.stderr}`);
});

check("P2 块注释（Rust 嵌套）中的恒真断言 → exit 0", () => {
  const root = makeSandbox();
  write(root, "crates/probe/t.rs", "/* 外层 /* 内层 assert!(true); */ 仍在外层 */\n#[test]\nfn t() {\n    assert_eq!(1 + 1, 2);\n}\n");
  assert.equal(run(root).status, 0);
});

check("P3 字符串字面量里的 assert!(true) → exit 0", () => {
  const root = makeSandbox();
  write(root, "crates/probe/t.rs", "#[test]\nfn t() {\n    let s = \"assert!(true)\";\n    assert_eq!(s.len(), 13);\n}\n");
  assert.equal(run(root).status, 0);
});

// ③ 基线：登记放行；行号漂移后必须仍拒（否则基线会变成永久隐身衣）
check("B1 基线登记命中 → exit 0", () => {
  const root = makeSandbox();
  write(root, "crates/probe/t.rs", "#[test]\nfn t() {\n    assert!(true);\n}\n");
  write(root, "scripts/baseline.json", JSON.stringify(["crates/probe/t.rs:3"]));
  const r = run(root, ["--baseline", "scripts/baseline.json"]);
  assert.equal(r.status, 0, `期望 exit 0，实际 ${r.status}\n${r.stderr}`);
});

check("B2 行号漂移后基线不再命中 → exit 1（基线不是永久隐身衣）", () => {
  const root = makeSandbox();
  write(root, "crates/probe/t.rs", "#[test]\n\nfn t() {\n    assert!(true);\n}\n");
  write(root, "scripts/baseline.json", JSON.stringify(["crates/probe/t.rs:3"]));
  assert.equal(run(root, ["--baseline", "scripts/baseline.json"]).status, 1);
});

check("B3 基线文件缺失 → exit 2（fail-closed）", () => {
  const root = makeSandbox();
  write(root, "crates/probe/t.rs", "#[test]\nfn t() {\n    assert_eq!(1, 1);\n}\n");
  assert.equal(run(root, ["--baseline", "scripts/nope.json"]).status, 2);
});

// ④ 真实仓：仓库自己必须过门（含真实基线）
check("R1 本仓 + 真实基线 → exit 0", () => {
  const r = spawnSync(
    process.execPath,
    [SCRIPT, "--baseline", "scripts/placeholder-tests-baseline.json"],
    { cwd: REPO_ROOT, encoding: "utf8" }
  );
  assert.equal(r.status, 0, `本仓应过门，实际 ${r.status}\n${r.stderr}`);
});

if (failures > 0) {
  console.error(`check-placeholder-tests.spec: FAIL ${failures} 项`);
  process.exit(1);
}
console.log("check-placeholder-tests.spec: PASS（负样本必拒 + 正样本放行 + 基线语义符合预期）");
