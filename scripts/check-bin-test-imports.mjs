import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { execFileSync } from "node:child_process";

const REPO_ROOT = path.resolve(path.dirname(new URL(import.meta.url).pathname), "..");
const BASELINE_PATH = path.join(REPO_ROOT, "scripts", "bin-as-module-baseline.json");

function getBaseline() {
  if (!fs.existsSync(BASELINE_PATH)) {
    return new Set();
  }
  const data = JSON.parse(fs.readFileSync(BASELINE_PATH, "utf8"));
  return new Set(data.allowed_files || []);
}

function scanTestFiles() {
  const filesOutput = execFileSync("git", ["ls-files", "--cached", "--others", "--exclude-standard", "crates/*/tests/*.rs"], {
    cwd: REPO_ROOT,
    encoding: "utf8"
  });

  const occurrences = new Map();
  const pattern = /#\[path\s*=\s*"[^"]*src\/bin\/[^"]*"\]/;

  for (const relPath of filesOutput.split("\n")) {
    const file = relPath.trim();
    if (!file) continue;
    const fullPath = path.join(REPO_ROOT, file);
    if (!fs.existsSync(fullPath)) continue;
    const lines = fs.readFileSync(fullPath, "utf8").split("\n");
    for (let i = 0; i < lines.length; i++) {
      if (pattern.test(lines[i])) {
        if (!occurrences.has(file)) occurrences.set(file, []);
        occurrences.get(file).push({ line: i + 1, text: lines[i] });
      }
    }
  }
  return occurrences;
}

const isUpdate = process.argv.includes("--update-baseline");
const occurrences = scanTestFiles();
const currentFiles = Array.from(occurrences.keys()).sort();

if (isUpdate) {
  const baseline = {
    version: 1,
    description: "bin-as-module 存量反模式白名单（棘轮门禁：只减不增，严禁新增文件引入 src/bin）",
    updated_at: new Date().toISOString(),
    allowed_count: currentFiles.length,
    allowed_files: currentFiles
  };
  fs.writeFileSync(BASELINE_PATH, JSON.stringify(baseline, null, 2) + "\n", "utf8");
  console.log(`[PASS] 基线已更新：当前共收录 ${currentFiles.length} 个存量文件`);
  process.exit(0);
}

const baseline = getBaseline();
const newViolations = [];
for (const file of currentFiles) {
  if (!baseline.has(file)) {
    newViolations.push(file);
  }
}

if (newViolations.length > 0) {
  console.error("\n[FAIL] 架构门禁：检测到未在基线中的测试文件使用了 #[path = '...src/bin/...'] 反模式！");
  console.error("违规将二进制目标当模块引入会导致全量内嵌测试被重复执行，极易引发死循环与内存打穿。");
  console.error("解决办法：请将被测函数/数据结构抽取到 crates/*/src/lib.rs 或库子模块中，通过 use 正常引用。\n");
  for (const file of newViolations) {
    console.error(`  - 违规文件: ${file}`);
    for (const occ of occurrences.get(file)) {
      console.error(`      L${occ.line}: ${occ.text.trim()}`);
    }
  }
  process.exit(1);
}

// 棘轮检测：若基线中存在已修复的文件，提醒或收窄
const resolved = [];
for (const file of baseline) {
  if (!occurrences.has(file)) {
    resolved.push(file);
  }
}

if (resolved.length > 0) {
  console.log(`\n[NOTICE] 棘轮进阶：有 ${resolved.length} 个测试文件已彻底根治 bin-as-module 反模式！`);
  for (const r of resolved) {
    console.log(`  + 已修复: ${r}`);
  }
  console.log(`请运行 'node scripts/check-bin-test-imports.mjs --update-baseline' 收紧基线。\n`);
}

console.log(`[PASS] bin-as-module 棘轮门禁检查通过：未引入任何新增违规（当前存量 ${currentFiles.length} / 基线允许 ${baseline.size}）`);