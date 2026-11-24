#!/usr/bin/env node
/**
 * scripts/check-bin-test-imports.mjs —— 禁止新增 bin-as-module 测试反模式（棘轮门禁）
 *
 * 背景：`crates/<crate>/src/bin/*.rs`（当前 `tui` bin 含 284 个 `#[test]`、`cli` bin 含 3 个 `#[test]`）在集成测试中
 * 通过 `#[path = "../src/bin/..."] mod ...;` 引入，会导致每个集成测试二进制重编并重跑整份 bin
 * 单测（49 份 tui + 1 份 cli = 13,919 次重复测试执行），也是 263 个集成测试二进制冷链接膨胀的核心诱因。
 *
 * 契约：
 * - 递归扫描 `crates/<crate>/tests` 与 `src-tauri/tests` 下全部 `.rs` 文件；
 * - 剥离行注释与块注释后匹配 `#[path = "...src/bin/..."]`（支持跨行属性，忽略文档注释）；
 * - 基线按「文件路径 → 允许次数」记账（`scripts/bin-as-module-baseline.json`）：
 *   不仅禁新增文件，同一文件内增量追加第二处 `#[path]` 同样 exit 1；
 * - `--update-baseline` 在文件或次数上涨时 fail-closed（exit 1），仅传 `--allow-growth` 才允许上涨；
 * - 基线 JSON 确定性排序、不含易变时间戳，收敛时零 diff 噪声；
 * - 支持 `--root <dir>` 指定待检仓库根目录（供隔离沙箱自检）。
 */
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const SELF_DIR = path.dirname(fileURLToPath(import.meta.url));
const DEFAULT_ROOT = path.resolve(SELF_DIR, "..");
const TEST_FILE_PATH_RX = /^(crates\/[^/]+\/tests|src-tauri\/tests)\/.*\.rs$/;
const BIN_PATH_ATTR_RX = /#\[\s*path\s*=\s*"[^"]*src[\\/]bin[\\/][^"]*"\s*\]/g;

function parseArgs(argv) {
  const opts = {
    root: DEFAULT_ROOT,
    updateBaseline: false,
    allowGrowth: false,
    json: false,
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--update-baseline") opts.updateBaseline = true;
    else if (arg === "--allow-growth") opts.allowGrowth = true;
    else if (arg === "--json") opts.json = true;
    else if (arg === "--root" && argv[i + 1]) opts.root = path.resolve(argv[++i]);
    else if (arg === "--help" || arg === "-h") opts.help = true;
    else throw new Error(`未知参数: ${arg}`);
  }
  return opts;
}

/** 递归枚举仓库内测试 `.rs` 相对路径；有 `.git` 时优先用 `git ls-files` 尊重 ignore。 */
function collectTestFiles(repoRoot) {
  const gitMarker = path.join(repoRoot, ".git");
  if (fs.existsSync(gitMarker)) {
    const raw = execFileSync("git", ["ls-files", "--cached", "--others", "--exclude-standard"], {
      cwd: repoRoot,
      encoding: "utf8",
      maxBuffer: 16 * 1024 * 1024,
    });
    return raw
      .split(/\r?\n/)
      .map((line) => line.trim())
      .filter((rel) => rel && TEST_FILE_PATH_RX.test(rel))
      .sort();
  }

  const found = [];
  const walk = (absDir, relPrefix) => {
    if (!fs.existsSync(absDir)) return;
    for (const entry of fs.readdirSync(absDir, { withFileTypes: true })) {
      if (entry.name === ".git" || entry.name === "node_modules" || entry.name.startsWith("target")) continue;
      const rel = relPrefix ? `${relPrefix}/${entry.name}` : entry.name;
      const full = path.join(absDir, entry.name);
      if (entry.isDirectory()) walk(full, rel);
      else if (entry.isFile() && TEST_FILE_PATH_RX.test(rel)) found.push(rel);
    }
  };
  walk(path.join(repoRoot, "crates"), "crates");
  walk(path.join(repoRoot, "src-tauri", "tests"), "src-tauri/tests");
  return found.sort();
}

/** 剥离行/块注释但保留换行符与字符偏移，避免文档注释误报且保持行号精确。 */
function stripCommentsPreservingOffsets(src) {
  let out = "";
  let i = 0;
  const n = src.length;
  while (i < n) {
    const ch = src[i];
    const next = src[i + 1];
    if (ch === "/" && next === "/") {
      while (i < n && src[i] !== "\n") {
        out += " ";
        i++;
      }
      continue;
    }
    if (ch === "/" && next === "*") {
      let depth = 1;
      out += "  ";
      i += 2;
      while (i < n && depth > 0) {
        if (src[i] === "/" && src[i + 1] === "*") {
          depth++;
          out += "  ";
          i += 2;
        } else if (src[i] === "*" && src[i + 1] === "/") {
          depth--;
          out += "  ";
          i += 2;
        } else {
          out += src[i] === "\n" ? "\n" : " ";
          i++;
        }
      }
      continue;
    }
    if (ch === '"') {
      out += ch;
      i++;
      while (i < n) {
        const c = src[i];
        out += c;
        i++;
        if (c === "\\" && i < n) {
          out += src[i];
          i++;
        } else if (c === '"') {
          break;
        }
      }
      continue;
    }
    out += ch;
    i++;
  }
  return out;
}

function lineNumberAt(text, offset) {
  let line = 1;
  for (let i = 0; i < offset; i++) {
    if (text[i] === "\n") line++;
  }
  return line;
}

/** 扫描全部测试 `.rs`，返回 Map<relPath, Array<{line, text}>>。 */
export function scanTestFiles(repoRoot) {
  const files = collectTestFiles(repoRoot);
  const occurrences = new Map();
  for (const rel of files) {
    const fullPath = path.join(repoRoot, rel);
    if (!fs.existsSync(fullPath)) continue;
    const raw = fs.readFileSync(fullPath, "utf8");
    const masked = stripCommentsPreservingOffsets(raw);
    BIN_PATH_ATTR_RX.lastIndex = 0;
    let match;
    while ((match = BIN_PATH_ATTR_RX.exec(masked)) !== null) {
      if (!occurrences.has(rel)) occurrences.set(rel, []);
      occurrences.get(rel).push({
        line: lineNumberAt(raw, match.index),
        text: raw.slice(match.index, match.index + match[0].length).replace(/\s+/g, " ").trim(),
      });
    }
  }
  return occurrences;
}

/** 读取基线：同时兼容 v2 ({file: count}) 与 v1 ([file, ...])。 */
export function loadBaseline(baselinePath) {
  if (!fs.existsSync(baselinePath)) {
    return { exists: false, counts: new Map() };
  }
  const data = JSON.parse(fs.readFileSync(baselinePath, "utf8"));
  const counts = new Map();
  if (Array.isArray(data.allowed_files)) {
    for (const f of data.allowed_files) counts.set(f, 1);
  } else if (data.allowed_files && typeof data.allowed_files === "object") {
    for (const [f, count] of Object.entries(data.allowed_files)) {
      if (Number.isInteger(count) && count > 0) counts.set(f, count);
    }
  }
  return { exists: true, counts };
}

function evaluateRatchet(occurrences, baselineCounts) {
  const newFiles = [];
  const increasedFiles = [];
  const reducedFiles = [];
  let currentTotal = 0;

  for (const [file, list] of [...occurrences.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
    const cur = list.length;
    currentTotal += cur;
    const allowed = baselineCounts.get(file) ?? 0;
    if (allowed === 0) {
      newFiles.push({ file, current: cur, allowed: 0, list });
    } else if (cur > allowed) {
      increasedFiles.push({ file, current: cur, allowed, list });
    } else if (cur < allowed) {
      reducedFiles.push({ file, current: cur, allowed });
    }
  }

  for (const [file, allowed] of [...baselineCounts.entries()].sort((a, b) => a[0].localeCompare(b[0]))) {
    if (!occurrences.has(file)) {
      reducedFiles.push({ file, current: 0, allowed });
    }
  }

  let baselineTotal = 0;
  for (const v of baselineCounts.values()) baselineTotal += v;

  return {
    currentFiles: occurrences.size,
    currentTotal,
    baselineFiles: baselineCounts.size,
    baselineTotal,
    newFiles,
    increasedFiles,
    reducedFiles,
  };
}

function buildBaselineJson(occurrences) {
  const sortedEntries = [...occurrences.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  const allowedFiles = {};
  let total = 0;
  for (const [file, list] of sortedEntries) {
    allowedFiles[file] = list.length;
    total += list.length;
  }
  return (
    JSON.stringify(
      {
        version: 2,
        allowed_file_count: sortedEntries.length,
        allowed_total_imports: total,
        allowed_files: allowedFiles,
      },
      null,
      2
    ) + "\n"
  );
}

export function run(argv = process.argv.slice(2)) {
  const opts = parseArgs(argv);
  if (opts.help) {
    console.log("用法: node scripts/check-bin-test-imports.mjs [--update-baseline [--allow-growth]] [--root <dir>] [--json]");
    return 0;
  }

  const baselinePath = path.join(opts.root, "scripts", "bin-as-module-baseline.json");
  const occurrences = scanTestFiles(opts.root);
  const baseline = loadBaseline(baselinePath);
  const verdict = evaluateRatchet(occurrences, baseline.counts);

  if (opts.updateBaseline) {
    const hasGrowth =
      !baseline.exists ||
      verdict.newFiles.length > 0 ||
      verdict.increasedFiles.length > 0 ||
      verdict.currentTotal > verdict.baselineTotal;

    if (hasGrowth && !opts.allowGrowth) {
      console.error("[FAIL] 拒绝静默抬高 bin-as-module 基线：检出新增违规文件或单文件次数上涨（或尚无基线）。");
      console.error("如确需建册或豁免，请人工 review diff 后显式传 `--allow-growth`。");
      for (const item of [...verdict.newFiles, ...verdict.increasedFiles]) {
        console.error(`  - ${item.file}: 基线 ${item.allowed} → 当前 ${item.current}`);
      }
      return 1;
    }

    fs.mkdirSync(path.dirname(baselinePath), { recursive: true });
    fs.writeFileSync(baselinePath, buildBaselineJson(occurrences), "utf8");
    console.log(
      `[PASS] bin-as-module 基线已更新：${verdict.currentFiles} 个文件，共 ${verdict.currentTotal} 处引用`
    );
    return 0;
  }

  if (!baseline.exists) {
    console.error(`[FAIL] 缺少基线文件：${baselinePath}（fail-closed）`);
    return 2;
  }

  const failed = verdict.newFiles.length > 0 || verdict.increasedFiles.length > 0;
  if (opts.json) {
    console.log(JSON.stringify({ ok: !failed, ...verdict }, null, 2));
  }

  if (failed) {
    if (!opts.json) {
      console.error("\n[FAIL] 架构门禁：检测到新增 bin-as-module（#[path = \"...src/bin/...\"]）引入！");
      console.error("请将被测函数/数据结构抽取到 `crates/<crate>/src/` 模块并由 lib.rs 导出，通过 crate::/pony_agent_core:: 正常 use。\n");
      for (const item of verdict.newFiles) {
        console.error(`  - 新增违规文件: ${item.file} (${item.current} 处)`);
        for (const occ of item.list) console.error(`      L${occ.line}: ${occ.text}`);
      }
      for (const item of verdict.increasedFiles) {
        console.error(`  - 单文件次数上涨: ${item.file} (${item.allowed} → ${item.current})`);
        for (const occ of item.list) console.error(`      L${occ.line}: ${occ.text}`);
      }
    }
    return 1;
  }

  if (!opts.json) {
    if (verdict.reducedFiles.length > 0) {
      console.log(`\n[NOTICE] 棘轮可收紧：有 ${verdict.reducedFiles.length} 个测试文件的 bin-as-module 引用已减少或清零：`);
      for (const r of verdict.reducedFiles) console.log(`  + ${r.file}: ${r.allowed} → ${r.current}`);
      console.log("可运行 `npm run contract:bin-imports:update` 收窄基线。\n");
    }
    console.log(
      `[PASS] bin-as-module 棘轮门禁通过：当前 ${verdict.currentFiles} 文件/${verdict.currentTotal} 处（基线 ${verdict.baselineFiles} 文件/${verdict.baselineTotal} 处）`
    );
  }
  return 0;
}

const isMain = (() => {
  try {
    return (
      Boolean(process.argv[1]) &&
      fs.realpathSync(process.argv[1]) === fs.realpathSync(fileURLToPath(import.meta.url))
    );
  } catch {
    return false;
  }
})();

if (isMain) {
  process.exit(run());
}
