#!/usr/bin/env node
/**
 * scripts/check-bin-test-imports.spec.mjs —— bin-as-module 门禁的正负样本自检（全沙箱隔离）。
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SCRIPT = path.join(REPO_ROOT, "scripts", "check-bin-test-imports.mjs");

function runGate(args, cwd = REPO_ROOT) {
  return spawnSync(process.execPath, [SCRIPT, ...args], {
    cwd,
    encoding: "utf8",
  });
}

function makeSandbox() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "bin-imports-spec-"));
  fs.mkdirSync(path.join(dir, "crates", "core-a", "tests", "nested"), { recursive: true });
  fs.mkdirSync(path.join(dir, "src-tauri", "tests"), { recursive: true });
  fs.mkdirSync(path.join(dir, "scripts"), { recursive: true });
  return dir;
}

// 1. 真实仓库基线必须 PASS
const realRes = runGate([]);
assert.equal(realRes.status, 0, `真实仓库门禁应 PASS，实际 stderr:\n${realRes.stderr}`);
console.log("PASS 1: 真实仓库 bin-as-module 基线通过");

const sandbox = makeSandbox();
try {
  const allowedFile = path.join(sandbox, "crates", "core-a", "tests", "existing.rs");
  const commentOnlyFile = path.join(sandbox, "crates", "core-a", "tests", "docs_only.rs");
  fs.writeFileSync(allowedFile, '#[path = "../src/bin/tui.rs"]\nmod tui;\n', "utf8");
  fs.writeFileSync(
    commentOnlyFile,
    '//! 注释里提及 #[path = "../src/bin/tui.rs"] 不应计入\n#[test]\nfn ok() {}\n',
    "utf8"
  );

  // 2. 首次建册缺少 --allow-growth 必须拒绝
  const initBlocked = runGate(["--root", sandbox, "--update-baseline"]);
  assert.equal(initBlocked.status, 1, "首次建册不带 --allow-growth 必须 exit 1");
  console.log("PASS 2: 首次建册缺少 --allow-growth 被拒绝");

  // 3. 带 --allow-growth 建册成功，且无时间戳保持确定性，只收录 1 文件/1 处（忽略注释）
  const initOk = runGate(["--root", sandbox, "--update-baseline", "--allow-growth"]);
  assert.equal(initOk.status, 0, `建册应通过:\n${initOk.stderr}`);
  const baselinePath = path.join(sandbox, "scripts", "bin-as-module-baseline.json");
  const raw1 = fs.readFileSync(baselinePath, "utf8");
  const parsed = JSON.parse(raw1);
  assert.equal(parsed.allowed_file_count, 1);
  assert.equal(parsed.allowed_total_imports, 1);
  assert.equal("updated_at" in parsed, false, "基线不得包含易变 updated_at 字段");
  const checkClean = runGate(["--root", sandbox]);
  assert.equal(checkClean.status, 0, "仅注释引用不应触发违规");
  console.log("PASS 3: --allow-growth 建册成功、确定性 JSON 且文档注释不误报");

  // 4. 嵌套子目录新增跨行 #[path = "...src/bin/..."] 必须被拦截
  const nestedBad = path.join(sandbox, "crates", "core-a", "tests", "nested", "bad_nested.rs");
  fs.writeFileSync(nestedBad, '#[path =\n  "../../src/bin/cli.rs"]\nmod cli;\n', "utf8");
  const nestedFail = runGate(["--root", sandbox]);
  assert.equal(nestedFail.status, 1, "嵌套子目录跨行引用必须 exit 1");
  assert.match(nestedFail.stderr, /nested\/bad_nested\.rs/);
  fs.unlinkSync(nestedBad);
  console.log("PASS 4: 递归扫描 + 跨行 #[path] 成功拦截嵌套违规文件");

  // 5. 白名单内已有文件再塞第 2 处引用必须被次数棘轮拦截，且无 --allow-growth 禁止重基线
  fs.writeFileSync(
    allowedFile,
    '#[path = "../src/bin/tui.rs"]\nmod tui;\n#[path = "../src/bin/cli.rs"]\nmod cli;\n',
    "utf8"
  );
  const countFail = runGate(["--root", sandbox]);
  assert.equal(countFail.status, 1, "同文件次数上涨必须被拦截");
  assert.match(countFail.stderr, /单文件次数上涨/);
  const updateBlocked = runGate(["--root", sandbox, "--update-baseline"]);
  assert.equal(updateBlocked.status, 1, "次数上涨无 --allow-growth 禁止重基线");
  console.log("PASS 5: 单文件次数棘轮生效，且禁止无 --allow-growth 洗白上涨");

  // 6. 消除违规后可直接 --update-baseline 收紧基线，且重跑输出零字节漂移
  fs.writeFileSync(allowedFile, "#[test]\nfn migrated() {}\n", "utf8");
  const shrinkNotice = runGate(["--root", sandbox]);
  assert.equal(shrinkNotice.status, 0);
  assert.match(shrinkNotice.stdout, /棘轮可收紧/);
  const shrinkOk = runGate(["--root", sandbox, "--update-baseline"]);
  assert.equal(shrinkOk.status, 0, "收敛收紧无需 --allow-growth");
  const after1 = fs.readFileSync(baselinePath, "utf8");
  const shrinkRepeat = runGate(["--root", sandbox, "--update-baseline"]);
  assert.equal(shrinkRepeat.status, 0);
  assert.equal(fs.readFileSync(baselinePath, "utf8"), after1, "重复更新基线必须确定性零 diff");
  console.log("PASS 6: 消除违规后可无需 --allow-growth 收紧且输出完全确定");

  // 7. src-tauri/tests 下违规同样受扫描覆盖
  const tauriBad = path.join(sandbox, "src-tauri", "tests", "bad_tauri.rs");
  fs.writeFileSync(tauriBad, '#[path = "../../crates/pony-agent-core/src/bin/tui.rs"]\nmod tui;\n', "utf8");
  const tauriFail = runGate(["--root", sandbox]);
  assert.equal(tauriFail.status, 1, "src-tauri/tests 下违规必须被拦截");
  assert.match(tauriFail.stderr, /src-tauri\/tests\/bad_tauri\.rs/);
  console.log("PASS 7: src-tauri/tests 纳入扫描并拦截违规");
} finally {
  fs.rmSync(sandbox, { recursive: true, force: true });
}

console.log("check-bin-test-imports.spec: ALL PASSED");
