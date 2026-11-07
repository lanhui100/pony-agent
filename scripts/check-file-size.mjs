#!/usr/bin/env node
/**
 * 代码规模门禁：Rust 单文件行数 / 单函数行数 / 单函数花括号嵌套深度。
 *
 * 背景：`crates/pony-agent-core/src/bin/tui.rs` 已 15327 行、`tui_event_loop` 约 2318 行、
 *   嵌套 16 层，此前仓库没有任何可机器检查的规模护栏。本脚本把「规模」变成
 *   一条可复现的命令级断言，防止拆分之后再次悄悄长回去。
 *
 * 扫描范围（默认）：`crates/` 与 `src-tauri/src/` 下所有 `*.rs`（递归），
 *   跳过任意层级的 target* 目录、node_modules/、.git/。
 *
 * 阈值（均可用 CLI 覆盖）：
 *   --max-file-lines <n>  单文件行数上限，默认 3000
 *   --max-fn-lines <n>    单函数行数上限，默认 200
 *   --max-depth <n>       单函数花括号嵌套深度上限，默认 8
 *   --baseline <path>     基线棘轮：基线里已记账的违规不判失败（存量不增长）
 *   --update-baseline <path> 把当前违规全量写成基线（供人工 review diff 后提交）
 *
 * 棘轮语义（存量债不阻塞，新增债必失败）：
 *   - 基线键 = kind | path | name，不绑行号（避免行号漂移造成假失败）；
 *   - 违规值 ≤ 基线值 → grandfathered，不失败；违规值 > 基线值或键不存在 → 新增违规，退出码 1；
 *   - `exempt-*` 类违规永不进基线（豁免围栏自身失效必须当场修）；
 *   - 基线里已不再违规的条目属「可瘦身项」，仅在 --json 中列出，不判失败。
 *
 * 文件级豁免（代码围栏）：文件**顶部**（前 40 行内）允许一行
 *   `// ponylint:file-size-exempt <理由>`
 *   `// ponylint:file-size-exempt until <YYYY-MM-DD> <理由>`
 * 语义：
 *   - 命中豁免的文件整体跳过规模检查（文件行数 + 函数行数 + 嵌套深度）；
 *   - 带 `until` 且**已过期**必须判违规（防止永久豁免悄悄生效）；
 *   - 理由为空必须判违规（防止 `// ponylint:file-size-exempt` 空壳豁免）。
 *
 * 函数行数/深度统计：简化 Rust 词法扫描，先把行注释 `//`、可嵌套块注释、
 *   字符串 `"..."`、字符字面量 `'a'`、raw string `r#"..."#`（含 `br`/`cr` 前缀）
 *   全部抹成空格（保留换行），再按花括号配平判定函数体范围。测试函数、`mod` 体内
 *   函数、`impl` 体内方法均按自身独立记账，不做豁免。
 *
 * 输出（stdout，每行一条违规）：
 *   <相对路径> 文件行数=<n> 超限
 *   <相对路径>:<起始行> <函数名> 行数=<n>
 *   <相对路径>:<起始行> <函数名> 嵌套深度=<n>
 *   <相对路径> 豁免已过期 until=<YYYY-MM-DD>   （豁免围栏自身失效）
 *   <相对路径> 豁免缺少理由                      （豁免围栏为空壳）
 * 末尾汇总：`FILE-SIZE GATE: N violation(s)`。
 *
 * 退出码：
 *   0 = 通过（无违规）
 *   1 = 存在新增/恶化超限（无基线时为任意超限）
 *   2 = 脚本自身错误（扫描根不存在 / 参数非法 / 读取失败 / 基线不可解析）
 *
 * 用法：
 *   node scripts/check-file-size.mjs
 *   node scripts/check-file-size.mjs --json
 *   node scripts/check-file-size.mjs --max-file-lines 2000 --max-fn-lines 150 --max-depth 6
 *   node scripts/check-file-size.mjs --root crates/pony-agent-core/src --root src-tauri/src
 *   node scripts/check-file-size.mjs --root /tmp/sizefixture          # 夹具自测
 *
 * 纯 Node ESM，无第三方依赖；除 `--update-baseline` 写基线文件外不写任何文件。
 */

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(SCRIPT_DIR, "..");
const DEFAULT_ROOTS = ["crates", "src-tauri/src"];
const DEFAULT_MAX_FILE_LINES = 3000;
const DEFAULT_MAX_FN_LINES = 200;
const DEFAULT_MAX_DEPTH = 8;
const EXEMPT_SCAN_LINES = 40;
const SKIP_DIR_NAMES = new Set(["node_modules", ".git"]);

function printHelp() {
  process.stdout.write(
    [
      "用法：node scripts/check-file-size.mjs [选项]",
      "",
      "  --max-file-lines <n>  单文件行数上限（默认 3000）",
      "  --max-fn-lines <n>    单函数行数上限（默认 200）",
      "  --max-depth <n>       单函数花括号嵌套深度上限（默认 8）",
      "  --baseline <file>     基线棘轮文件；只对「超出基线」的新增违规判失败",
      "  --root <dir>          扫描根（可重复或用逗号分隔；默认 crates,src-tauri/src）",
      "  --json                以 JSON 输出（stdout 仅 JSON，退出码语义不变）",
      "  -h, --help            显示本帮助",
      "",
      "退出码：0=通过；1=存在违规；2=脚本自身错误（路径不存在/参数非法）。",
      "豁免：文件顶部 `// ponylint:file-size-exempt [until YYYY-MM-DD] <理由>`。"
    ].join("\n") + "\n"
  );
}

function parsePositiveInt(raw, flag, errors) {
  const value = Number(raw);
  if (!Number.isInteger(value) || value <= 0) {
    errors.push(`${flag} 必须是正整数，收到：${raw === undefined ? "(缺失)" : raw}`);
    return null;
  }
  return value;
}

function parseArgs(argv) {
  const options = {
    maxFileLines: DEFAULT_MAX_FILE_LINES,
    maxFnLines: DEFAULT_MAX_FN_LINES,
    maxDepth: DEFAULT_MAX_DEPTH,
    roots: [],
    baseline: null,
    updateBaseline: null,
    json: false,
    help: false,
    errors: []
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--help" || arg === "-h") {
      options.help = true;
      continue;
    }
    if (arg === "--json") {
      options.json = true;
      continue;
    }
    if (arg === "--max-file-lines" || arg === "--max-fn-lines" || arg === "--max-depth") {
      const value = parsePositiveInt(argv[++i], arg, options.errors);
      if (value === null) continue;
      if (arg === "--max-file-lines") options.maxFileLines = value;
      else if (arg === "--max-fn-lines") options.maxFnLines = value;
      else options.maxDepth = value;
      continue;
    }
    if (arg === "--baseline" || arg === "--update-baseline") {
      const raw = argv[++i];
      if (!raw) {
        options.errors.push(`${arg} 需要一个文件路径`);
        continue;
      }
      if (arg === "--baseline") options.baseline = raw;
      else options.updateBaseline = raw;
      continue;
    }
    if (arg === "--root") {
      const raw = argv[++i];
      if (!raw) {
        options.errors.push("--root 需要一个目录参数");
        continue;
      }
      for (const part of raw.split(",")) {
        const trimmed = part.trim();
        if (trimmed) options.roots.push(trimmed);
      }
      continue;
    }
    options.errors.push(`未知参数：${arg}`);
    return options;
  }

  if (options.roots.length === 0) options.roots = [...DEFAULT_ROOTS];
  return options;
}

/**
 * 把 Rust 源码词法层面「非代码」片段抹成空格（保留换行，长度不变）：
 * 行注释、可嵌套块注释、字符串、字符字面量、raw string。
 * 目的：让后续花括号配平与 `fn` 识别不受注释/字符串干扰。
 * 注：`'` 后不构成字符字面量时按生命周期标签处理（保留原样，不含花括号，无副作用）。
 */
function maskRustSource(source) {
  const chars = source.split("");
  const length = source.length;
  const blank = (index) => {
    if (index < length && chars[index] !== "\n" && chars[index] !== "\r") chars[index] = " ";
  };

  // 识别 raw string 起始：可选 b/c 前缀 + r + n 个 # + "
  const rawLiteralStart = (pos) => {
    if (pos > 0 && /[A-Za-z0-9_]/.test(source[pos - 1])) return null;
    let cursor = pos;
    if (source[cursor] === "b" || source[cursor] === "c") cursor += 1;
    if (source[cursor] !== "r") return null;
    cursor += 1;
    let hashes = 0;
    while (source[cursor] === "#") {
      hashes += 1;
      cursor += 1;
    }
    if (source[cursor] !== '"') return null;
    return { hashes, quoteIndex: cursor };
  };

  let i = 0;
  while (i < length) {
    const current = source[i];
    const next = source[i + 1];

    // 行注释：// 到行尾
    if (current === "/" && next === "/") {
      while (i < length && source[i] !== "\n") {
        blank(i);
        i += 1;
      }
      continue;
    }

    // 块注释：/* ... */，Rust 允许嵌套
    if (current === "/" && next === "*") {
      let depth = 0;
      while (i < length) {
        if (source[i] === "/" && source[i + 1] === "*") {
          blank(i);
          blank(i + 1);
          depth += 1;
          i += 2;
          continue;
        }
        if (source[i] === "*" && source[i + 1] === "/") {
          blank(i);
          blank(i + 1);
          depth -= 1;
          i += 2;
          if (depth === 0) break;
          continue;
        }
        blank(i);
        i += 1;
      }
      continue;
    }

    // raw string：r"..." / r#"..."# / br#"..."# / cr#"..."#
    if (current === "r" || current === "b" || current === "c") {
      const rawStart = rawLiteralStart(i);
      if (rawStart) {
        const terminator = `"${"#".repeat(rawStart.hashes)}`;
        let cursor = i;
        // 抹掉前缀、#、开引号
        while (cursor <= rawStart.quoteIndex) {
          blank(cursor);
          cursor += 1;
        }
        while (cursor < length) {
          if (source.startsWith(terminator, cursor)) {
            for (let k = 0; k < terminator.length; k += 1) blank(cursor + k);
            cursor += terminator.length;
            break;
          }
          blank(cursor);
          cursor += 1;
        }
        i = cursor;
        continue;
      }
    }

    // 普通字符串："..."（处理转义）
    if (current === '"') {
      blank(i);
      let cursor = i + 1;
      while (cursor < length) {
        if (source[cursor] === "\\") {
          blank(cursor);
          blank(cursor + 1);
          cursor += 2;
          continue;
        }
        if (source[cursor] === '"') {
          blank(cursor);
          cursor += 1;
          break;
        }
        blank(cursor);
        cursor += 1;
      }
      i = cursor;
      continue;
    }

    // 字符字面量：'a' / '\n' / '\u{1F600}'；否则视为生命周期标签
    if (current === "'" && (source[i + 1] === "\\" || source[i + 2] === "'")) {
      blank(i);
      let cursor = i + 1;
      while (cursor < length) {
        if (source[cursor] === "\\") {
          blank(cursor);
          blank(cursor + 1);
          cursor += 2;
          continue;
        }
        if (source[cursor] === "'") {
          blank(cursor);
          cursor += 1;
          break;
        }
        blank(cursor);
        cursor += 1;
      }
      i = cursor;
      continue;
    }

    i += 1;
  }

  return chars.join("");
}

/** 统计内容行数：末尾换行不额外计一行（与 `wc -l` 口径一致）。 */
function countLines(source) {
  if (source.length === 0) return 0;
  let lines = 0;
  for (let i = 0; i < source.length; i += 1) {
    if (source[i] === "\n") lines += 1;
  }
  return source.endsWith("\n") ? lines : lines + 1;
}

function buildLineIndex(masked) {
  const starts = [0];
  for (let i = 0; i < masked.length; i += 1) {
    if (masked[i] === "\n") starts.push(i + 1);
  }
  return (offset) => {
    let low = 0;
    let high = starts.length - 1;
    while (low < high) {
      const mid = (low + high + 1) >> 1;
      if (starts[mid] <= offset) low = mid;
      else high = mid - 1;
    }
    return low + 1; // 1-based 行号
  };
}

/**
 * 扫描函数：返回每个函数声明的起始行、行数、最大花括号嵌套深度。
 * 只有带函数体 `{ ... }` 的声明被记账（`fn f();` 这类 trait 声明跳过）。
 */
function scanFunctions(source, masked) {
  const lineOf = buildLineIndex(masked);
  const fnPattern = /\bfn\s+([A-Za-z_][A-Za-z0-9_]*)/g;
  const matches = [];
  let match;
  while ((match = fnPattern.exec(masked)) !== null) {
    matches.push({ index: match.index, name: match[1] });
  }

  const functions = [];
  const stack = [];
  let pending = null;
  let matchCursor = 0;
  let depth = 0;
  let parenDepth = 0;

  for (let i = 0; i < masked.length; i += 1) {
    if (matchCursor < matches.length && matches[matchCursor].index === i) {
      pending = {
        name: matches[matchCursor].name,
        startOffset: i,
        depthAtDecl: depth,
        parenDepthAtDecl: parenDepth
      };
      matchCursor += 1;
    }

    const ch = masked[i];

    if (ch === "{") {
      const depthBefore = depth;
      depth += 1;
      for (const frame of stack) {
        if (depth > frame.maxDepth) frame.maxDepth = depth;
      }
      if (pending && pending.depthAtDecl === depthBefore && pending.parenDepthAtDecl === parenDepth) {
        stack.push({
          name: pending.name,
          startOffset: pending.startOffset,
          openDepth: depth,
          maxDepth: depth
        });
        pending = null;
      }
      continue;
    }

    if (ch === "}") {
      const top = stack[stack.length - 1];
      if (top && top.openDepth === depth) {
        const startLine = lineOf(top.startOffset);
        const endLine = lineOf(i);
        functions.push({
          name: top.name,
          startLine,
          lines: endLine - startLine + 1,
          depth: top.maxDepth - top.openDepth + 1
        });
        stack.pop();
      }
      depth = depth > 0 ? depth - 1 : 0;
      continue;
    }

    if (ch === "(" || ch === "[") {
      parenDepth += 1;
      continue;
    }
    if (ch === ")" || ch === "]") {
      parenDepth = parenDepth > 0 ? parenDepth - 1 : 0;
      continue;
    }
    if (ch === ";" && pending && pending.depthAtDecl === depth && pending.parenDepthAtDecl === parenDepth) {
      pending = null;
    }
  }

  return functions;
}

/**
 * 解析文件顶部豁免围栏。
 * 返回 null（无豁免）或 { status: "ok"|"expired"|"no-reason"|"bad-date", until, reason }。
 */
function readExemption(source, todayIso) {
  const headLines = source.split("\n", EXEMPT_SCAN_LINES);
  const pattern = /^[ \t]*\/\/[ \t]*ponylint:file-size-exempt\b(.*)$/;
  for (const line of headLines) {
    const found = line.match(pattern);
    if (!found) continue;
    const rest = found[1].trim();
    if (rest.length === 0) return { status: "no-reason", until: null, reason: "" };

    const untilMatch = rest.match(/^until\s+(\d{4}-\d{2}-\d{2})\b(.*)$/);
    if (untilMatch) {
      const until = untilMatch[1];
      const reason = untilMatch[2].trim();
      if (!/^\d{4}-\d{2}-\d{2}$/.test(until) || Number.isNaN(Date.parse(`${until}T00:00:00Z`))) {
        return { status: "bad-date", until, reason };
      }
      if (reason.length === 0) return { status: "no-reason", until, reason: "" };
      // 日期均为 YYYY-MM-DD，字典序即时间序；到期当天仍有效
      if (todayIso > until) return { status: "expired", until, reason };
      return { status: "ok", until, reason };
    }

    // `until` 后面跟了非法日期（如 2026/01/01）也要报出来，避免绕过
    const looseUntil = rest.match(/^until\b\s*(.*)$/);
    if (looseUntil) {
      return { status: "bad-date", until: looseUntil[1].trim(), reason: "" };
    }

    return { status: "ok", until: null, reason: rest };
  }
  return null;
}

function collectRustFiles(rootDir, out) {
  const entries = fs.readdirSync(rootDir, { withFileTypes: true });
  for (const entry of entries) {
    if (SKIP_DIR_NAMES.has(entry.name)) continue;
    const full = path.join(rootDir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name.startsWith("target")) continue;
      collectRustFiles(full, out);
      continue;
    }
    if (entry.isFile() && entry.name.endsWith(".rs")) out.push(full);
  }
}

function toRelPath(absolutePath) {  const relPath = path.relative(REPO_ROOT, absolutePath);
  // 仓库外路径（如 /tmp 夹具）保留绝对路径，避免出现一堆 ../../
  if (relPath.startsWith("..") || path.isAbsolute(relPath)) {
    return absolutePath.split(path.sep).join("/");
  }
  return relPath.split(path.sep).join("/");
}

const BASELINE_VERSION = 1;

function violationKey(violation) {
  // occurrence：同文件内同名函数的出现序号。只按 name 记账会让同名函数互相覆盖
  // （如 projection.rs 的 `apply` 223 行 / 388 行），误把长的那条当成「未恶化」。
  return `${violation.kind}|${violation.path}|${violation.name ?? ""}|${violation.occurrence ?? 0}`;
}

function assignOccurrences(violations) {
  const counters = new Map();
  for (const violation of violations) {
    const base = `${violation.kind}|${violation.path}|${violation.name ?? ""}`;
    const seen = counters.get(base) ?? 0;
    violation.occurrence = seen;
    counters.set(base, seen + 1);
  }
  return violations;
}

function resolveRepoPath(target) {
  return path.isAbsolute(target) ? target : path.resolve(REPO_ROOT, target);
}

function loadBaselineFile(target) {
  const absolute = resolveRepoPath(target);
  const parsed = JSON.parse(fs.readFileSync(absolute, "utf8"));
  if (!parsed || !Array.isArray(parsed.entries)) throw new Error("基线缺少 entries 数组");
  const map = new Map();
  for (const entry of parsed.entries) {
    if (!entry || typeof entry.path !== "string" || typeof entry.kind !== "string") {
      throw new Error("基线 entries 中存在非法条目");
    }
    map.set(violationKey(entry), entry);
  }
  return { absolute, map };
}

function writeBaselineFile(target, violations, options) {
  const absolute = resolveRepoPath(target);
  const entries = violations
    .filter((violation) => !violation.kind.startsWith("exempt-"))
    .map((violation) => ({
      path: violation.path,
      kind: violation.kind,
      name: violation.name ?? null,
      occurrence: violation.occurrence ?? 0,
      value: violation.value
    }))
    .sort((a, b) =>
      a.path === b.path
        ? a.kind === b.kind
          ? String(a.name ?? "").localeCompare(String(b.name ?? "")) ||
            (a.occurrence ?? 0) - (b.occurrence ?? 0)
          : a.kind.localeCompare(b.kind)
        : a.path.localeCompare(b.path)
    );
  const payload = {
    version: BASELINE_VERSION,
    note: "存量规模债基线（棘轮）：只许删条目/下调 value，新增违规由 size:check 判失败；本文件由 --update-baseline 生成。",
    thresholds: {
      maxFileLines: options.maxFileLines,
      maxFnLines: options.maxFnLines,
      maxDepth: options.maxDepth
    },
    entries
  };
  fs.mkdirSync(path.dirname(absolute), { recursive: true });
  fs.writeFileSync(absolute, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
  return { absolute, entryCount: entries.length };
}

function applyBaseline(violations, baselineMap) {
  const failures = [];
  const grandfathered = [];
  for (const violation of violations) {
    // 豁免围栏自身失效（过期/缺理由/日期非法）不接受基线豁免，必须当场修。
    if (violation.kind.startsWith("exempt-")) {
      failures.push({ ...violation, baselineValue: null });
      continue;
    }
    const known = baselineMap.get(violationKey(violation));
    if (known && typeof known.value === "number" && violation.value <= known.value) {
      grandfathered.push({ ...violation, baselineValue: known.value });
    } else {
      failures.push({ ...violation, baselineValue: known ? known.value : null });
    }
  }
  const present = new Set(violations.map(violationKey));
  const shrunken = [...baselineMap.values()].filter((entry) => !present.has(violationKey(entry)));
  return { failures, grandfathered, shrunken };
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    printHelp();
    return 0;
  }
  if (options.errors.length > 0) {
    for (const message of options.errors) process.stderr.write(`[check-file-size] 参数错误：${message}\n`);
    process.stderr.write("[check-file-size] 用 --help 查看用法\n");
    return 2;
  }

  const resolvedRoots = [];
  for (const root of options.roots) {
    const absolute = path.isAbsolute(root) ? root : path.resolve(REPO_ROOT, root);
    if (!fs.existsSync(absolute) || !fs.statSync(absolute).isDirectory()) {
      process.stderr.write(`[check-file-size] 扫描根不存在或不是目录：${root}\n`);
      return 2;
    }
    resolvedRoots.push(absolute);
  }

  const files = [];
  try {
    const found = [];
    for (const rootDir of resolvedRoots) collectRustFiles(rootDir, found);
    files.push(...new Set(found));
    files.sort();
  } catch (error) {
    process.stderr.write(`[check-file-size] 遍历目录失败：${error?.message ?? String(error)}\n`);
    return 2;
  }

  const todayIso = new Date().toISOString().slice(0, 10);
  const violations = [];
  const exempted = [];

  for (const absolutePath of files) {
    const relPath = toRelPath(absolutePath);
    let source;
    try {
      source = fs.readFileSync(absolutePath, "utf8");
    } catch (error) {
      process.stderr.write(`[check-file-size] 读取失败：${relPath}（${error?.message ?? String(error)}）\n`);
      return 2;
    }

    const exemption = readExemption(source, todayIso);
    if (exemption && exemption.status === "ok") {
      exempted.push({ path: relPath, until: exemption.until, reason: exemption.reason });
      continue;
    }
    if (exemption && exemption.status === "expired") {
      violations.push({
        path: relPath,
        kind: "exempt-expired",
        line: 0,
        value: null,
        limit: null,
        text: `${relPath} 豁免已过期 until=${exemption.until}`
      });
      continue;
    }
    if (exemption && exemption.status === "no-reason") {
      violations.push({
        path: relPath,
        kind: "exempt-no-reason",
        line: 0,
        value: null,
        limit: null,
        text: `${relPath} 豁免缺少理由`
      });
      continue;
    }
    if (exemption && exemption.status === "bad-date") {
      violations.push({
        path: relPath,
        kind: "exempt-bad-date",
        line: 0,
        value: null,
        limit: null,
        text: `${relPath} 豁免日期非法 until=${exemption.until || "(空)"}`
      });
      continue;
    }

    const fileLines = countLines(source);
    if (fileLines > options.maxFileLines) {
      violations.push({
        path: relPath,
        kind: "file-lines",
        line: 0,
        value: fileLines,
        limit: options.maxFileLines,
        text: `${relPath} 文件行数=${fileLines} 超限`
      });
    }

    const masked = maskRustSource(source);
    for (const fn of scanFunctions(source, masked)) {
      if (fn.lines > options.maxFnLines) {
        violations.push({
          path: relPath,
          kind: "fn-lines",
          line: fn.startLine,
          name: fn.name,
          value: fn.lines,
          limit: options.maxFnLines,
          text: `${relPath}:${fn.startLine} ${fn.name} 行数=${fn.lines}`
        });
      }
      if (fn.depth > options.maxDepth) {
        violations.push({
          path: relPath,
          kind: "fn-depth",
          line: fn.startLine,
          name: fn.name,
          value: fn.depth,
          limit: options.maxDepth,
          text: `${relPath}:${fn.startLine} ${fn.name} 嵌套深度=${fn.depth}`
        });
      }
    }
  }

  assignOccurrences(violations);
  violations.sort((a, b) => (a.path === b.path ? a.line - b.line : a.path < b.path ? -1 : 1));

  if (options.updateBaseline) {
    try {
      const written = writeBaselineFile(options.updateBaseline, violations, options);
      process.stdout.write(
        `FILE-SIZE BASELINE: 写入 ${written.entryCount} 条 → ${toRelPath(written.absolute)}\n`
      );
      return 0;
    } catch (error) {
      process.stderr.write(
        `[check-file-size] 写基线失败：${options.updateBaseline}（${error?.message ?? String(error)}）\n`
      );
      return 2;
    }
  }

  let failures = violations;
  let grandfathered = [];
  let shrunken = [];
  let baselineInfo = null;
  if (options.baseline) {
    try {
      const loaded = loadBaselineFile(options.baseline);
      const applied = applyBaseline(violations, loaded.map);
      failures = applied.failures;
      grandfathered = applied.grandfathered;
      shrunken = applied.shrunken;
      baselineInfo = {
        path: toRelPath(loaded.absolute),
        entries: loaded.map.size,
        grandfathered: grandfathered.length,
        shrunken: shrunken.length
      };
    } catch (error) {
      process.stderr.write(
        `[check-file-size] 读取基线失败：${options.baseline}（${error?.message ?? String(error)}）\n`
      );
      return 2;
    }
  }

  const summary = {
    filesScanned: files.length,
    exemptedFiles: exempted.length,
    overLimitCount: violations.length,
    violationCount: failures.length,
    baseline: baselineInfo,
    limits: {
      maxFileLines: options.maxFileLines,
      maxFnLines: options.maxFnLines,
      maxDepth: options.maxDepth
    }
  };

  if (options.json) {
    process.stdout.write(
      JSON.stringify(
        { ok: failures.length === 0, summary, violations: failures, grandfathered, shrunken, exempted },
        null,
        2
      ) + "\n"
    );
  } else {
    for (const violation of failures) {
      const hint = violation.kind.startsWith("exempt-")
        ? "（豁免围栏自身失效，不接受基线豁免）"
        : violation.baselineValue === null || violation.baselineValue === undefined
          ? "（基线无此条目，属新增超限）"
          : `（基线允许 ≤${violation.baselineValue}）`;
      process.stdout.write(`${violation.text} ${hint}\n`);
    }
    process.stdout.write(
      baselineInfo
        ? `FILE-SIZE GATE: ${failures.length} new violation(s) / ${violations.length} over limit / ` +
            `${grandfathered.length} grandfathered / ${shrunken.length} shrunken baseline entr(ies)\n`
        : `FILE-SIZE GATE: ${failures.length} violation(s)\n`
    );
  }

  if (failures.length > 0) {
    process.stderr.write(
      `[check-file-size] 阈值：文件≤${options.maxFileLines} 行 / 函数≤${options.maxFnLines} 行 / 嵌套≤${options.maxDepth} 层；` +
        `扫描 ${files.length} 个 .rs 文件，豁免 ${exempted.length} 个，超限 ${violations.length} 条，` +
        `其中新增/恶化 ${failures.length} 条\n`
    );
  }

  return failures.length === 0 ? 0 : 1;
}

try {
  process.exitCode = main();
} catch (error) {
  process.stderr.write(`[check-file-size] 未捕获异常：${error?.stack ?? String(error)}\n`);
  process.exitCode = 2;
}
