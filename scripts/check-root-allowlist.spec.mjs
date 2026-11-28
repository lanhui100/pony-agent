#!/usr/bin/env node
/**
 * scripts/check-root-allowlist.spec.mjs —— 根目录白名单门的正负样本自检。
 *
 * 负样本必须自带：本门守护的失效形态（根目录堆垃圾）不影响任何测试，
 * 门自己写坏了会安静地全绿 —— 只有负样本能钉住「它真的会拒」。
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SCRIPT = path.join(REPO_ROOT, "scripts", "check-root-allowlist.mjs");

function makeRepo() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "root-allowlist-spec-"));
  fs.mkdirSync(path.join(dir, "scripts"), { recursive: true });
  fs.copyFileSync(SCRIPT, path.join(dir, "scripts", "check-root-allowlist.mjs"));
  const git = (...args) => spawnSync("git", args, { cwd: dir, encoding: "utf8" });
  git("init", "-q");
  git("config", "user.email", "spec@example.invalid");
  git("config", "user.name", "spec");
  return { dir, git };
}

function run(dir) {
  return spawnSync(process.execPath, [path.join(dir, "scripts", "check-root-allowlist.mjs")], {
    cwd: dir,
    encoding: "utf8",
  });
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

check("N1 未登记的根级文件 → exit 1 并点名", () => {
  const { dir, git } = makeRepo();
  fs.writeFileSync(path.join(dir, "package.json"), "{}");
  fs.writeFileSync(path.join(dir, "stray-report.json"), "{}");
  git("add", "-A");
  const r = run(dir);
  assert.equal(r.status, 1, `期望 exit 1，实际 ${r.status}\n${r.stderr}`);
  assert.match(r.stderr, /stray-report\.json/);
});

check("P1 只有白名单内文件 → exit 0", () => {
  const { dir, git } = makeRepo();
  for (const f of ["package.json", "AGENTS.md", "README.md", "Cargo.toml"]) {
    fs.writeFileSync(path.join(dir, f), "x");
  }
  git("add", "-A");
  const r = run(dir);
  assert.equal(r.status, 0, `期望 exit 0，实际 ${r.status}\n${r.stderr}`);
});

check("P2 子目录里的任意文件不受根白名单约束 → exit 0", () => {
  const { dir, git } = makeRepo();
  fs.writeFileSync(path.join(dir, "package.json"), "{}");
  fs.mkdirSync(path.join(dir, "docs"), { recursive: true });
  fs.writeFileSync(path.join(dir, "docs", "anything-goes.md"), "x");
  git("add", "-A");
  assert.equal(run(dir).status, 0);
});

check("N2 未跟踪的根级垃圾不判红（门只管 git 跟踪面）", () => {
  const { dir, git } = makeRepo();
  fs.writeFileSync(path.join(dir, "package.json"), "{}");
  git("add", "-A");
  fs.writeFileSync(path.join(dir, "untracked-tmp.json"), "{}");
  const r = run(dir);
  assert.equal(r.status, 0, `未跟踪文件不应判红，实际 ${r.status}\n${r.stderr}`);
});

check("R1 本仓 → exit 0", () => {
  const r = spawnSync(process.execPath, [SCRIPT], { cwd: REPO_ROOT, encoding: "utf8" });
  assert.equal(r.status, 0, `本仓应过门，实际 ${r.status}\n${r.stderr}`);
});

if (failures > 0) {
  console.error(`check-root-allowlist.spec: FAIL ${failures} 项`);
  process.exit(1);
}
console.log("check-root-allowlist.spec: PASS（负样本必拒 + 正样本放行 + 跟踪面语义符合预期）");
