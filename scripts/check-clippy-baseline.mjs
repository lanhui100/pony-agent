#!/usr/bin/env node
/**
 * scripts/check-clippy-baseline.mjs —— Rust lint 棘轮门（存量冻结，新增/恶化即红）
 *
 * 契约真源：`.meta/lints/lints.toml`（表）+ `.meta/lints/clippy.toml`（阈值）+ 本脚本 +
 * `scripts/clippy-baseline.json`（存量账，**按平台分册**）；决策与备选见 ADR
 * `.agents/notes/implemented/process/2026-11-18-rust-quality-gate-and-skills.md`。
 *
 * 为什么不是 `[workspace.lints]`：本仓 CI 用 `RUSTFLAGS=-D warnings` 做既有严格门。若把 lint 写进
 * Cargo.toml，cargo 会向 rustc 传 `--warn=<lint>`，而"特定 lint 级别覆盖 `warnings` 组级 deny"，
 * 等于让这些 lint 逃出既有 -D warnings 门（静默削弱，而不是增强）。故本门自带 lint 级别，
 * 在子进程里清空 `RUSTFLAGS` 后注入 `-W`/`-A`，只判"新增债"。
 *
 * 棘轮语义（与 scripts/check-file-size.mjs 同口径）：
 *   · 记账键 = (lint, 相对路径)，不绑行号（行漂移不产生假失败）；
 *   · live ≤ base → grandfathered，放行；live > base 或键不存在 → 新增/恶化，exit 1；
 *   · base 有而 live 变少 → 「可瘦身项」，仅提示，不判红（瘦身由人提交基线收缩）。
 *   · 基线**按平台分册**（`platforms.<process.platform>`）：cfg(windows)/cfg(unix) 分支令两边
 *     违例集合不同（实测本仓 28 个文件、约 24% 基线落在平台门控文件里），单一基线在另一平台
 *     必然假红。分册缺失 → exit 2 并提示在该平台 `--update-baseline`，绝不静默套用别平台的口径。
 *
 * 反 fail-open 护栏（都是实测出来的洞，不是想象）：
 *   · 扫描面指纹：`lints.toml` + `clippy.toml` 内容 sha256 与基线不符 → exit 2（改阈值不能零红放行）；
 *   · 元数据校验：基线的 `package` / `targets` 与本次调用不符 → exit 2（否则换包/换扫描面仍 PASS）；
 *   · 网格失效：0 条诊断却有存量、或实测不足基线 50% → exit 2（`--message-format` 被吞时不能判绿）；
 *   · 未知 lint：表里拼错的 lint 会产出 E0602 且无 span，过去被当外部文件丢弃 → 现在 exit 2；
 *   · 入口守卫用 realpath（+ win32 小写），符号链接调用不再"零输出 exit 0"；
 *   · `--from-json` 与 `--update-baseline` 互斥（禁止把夹具写成正式基线）；
 *   · 重基线含新增/上涨时**默认拒写**（exit 1），确认为有意变更需显式 `--allow-growth`。
 *
 * 退出码：
 *   0 通过（无新增/恶化）
 *   1 存在新增/恶化违例；或 `--update-baseline` 检出上涨/新增键却未加 `--allow-growth`
 *   2 门自身错误：用法错误、表/阈值不可解析、口径指纹不符、平台分册缺失、基线损坏/元数据不符、
 *     未知 lint、扫描面失效、cargo 未能完成扫描（原生退出码写进消息，不再原样透传以免与"1"混淆）
 *
 * 副作用：仅向 cargo 的 `--target-dir`（槽位 check → target-check/）写入构建产物，可重复运行。
 * 环境：子进程清空 RUSTFLAGS / CARGO_ENCODED_RUSTFLAGS（门自持 lint 级别），
 *   并设 CLIPPY_CONF_DIR=<repo>/.meta/lints（确定性，不依赖向上搜索）。
 *
 * 用法：
 *   node scripts/check-clippy-baseline.mjs                    # 判红/判绿
 *   node scripts/check-clippy-baseline.mjs --update-baseline  # 重基线（review diff 后提交）
 *   node scripts/check-clippy-baseline.mjs --platform win32   # 指定平台分册（默认 process.platform）
 *   node scripts/check-clippy-baseline.mjs --all-targets      # 含测试目标（默认 --lib --bins）
 *   node scripts/check-clippy-baseline.mjs --json             # 机器可读
 *   node scripts/check-clippy-baseline.mjs --from-json <file> # 逻辑体自检：读已捕获的 cargo JSON 流
 *
 * 注：本文件同时被 scripts/check-clippy-baseline.spec.mjs import（纯函数），
 *   故顶层不得有副作用，入口以 realpath 判定。
 */
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

export const DEFAULTS = Object.freeze({
  lintTable: ".meta/lints/lints.toml",
  clippyConfDir: ".meta/lints",
  clippyTomlName: "clippy.toml",
  baseline: "scripts/clippy-baseline.json",
  packageName: "pony-agent-core",
  invokeRustTarget: "scripts/invoke-rust-target.mjs"
});

export const BASELINE_VERSION = 2;

/** 实测不足基线的这个比例就判定"扫描面可能失效"，拒绝放行。见头注释「反 fail-open 护栏」。 */
export const GUARD_MIN_LIVE_RATIO = 0.5;

const VALID_LEVELS = new Set(["warn", "allow", "deny", "forbid"]);
const SECTION_KINDS = Object.freeze({ rust: "rust", clippy: "clippy" });
const UNKNOWN_LINT_CODES = new Set(["E0602", "unknown_lints", "renamed_and_removed_lints"]);

export class ConfigError extends Error {}

// ────────────────────────────── 表解析 ──────────────────────────────

/** 去掉行内注释：只在引号外遇到 `#` 才截断（表里可能有含 `#` 的值）。 */
export function stripLintTableComment(line) {
  let out = "";
  let quote = null;
  for (const ch of line) {
    if (quote) {
      out += ch;
      if (ch === quote) quote = null;
      continue;
    }
    if (ch === '"' || ch === "'") {
      quote = ch;
      out += ch;
      continue;
    }
    if (ch === "#") break;
    out += ch;
  }
  return out;
}

function parseLevelValue(raw, where) {
  const value = raw.trim();
  const plain = value.match(/^"([a-z]+)"$/);
  if (plain) {
    if (!VALID_LEVELS.has(plain[1])) throw new ConfigError(`${where}: 非法 level "${plain[1]}"`);
    return { level: plain[1], priority: undefined };
  }
  const inline = value.match(/^\{(.*)\}$/);
  if (!inline) throw new ConfigError(`${where}: 无法解析的值 ${value}`);
  const levelMatch = inline[1].match(/level\s*=\s*"([a-z]+)"/);
  if (!levelMatch) throw new ConfigError(`${where}: 内联表缺 level（${value}）`);
  if (!VALID_LEVELS.has(levelMatch[1])) throw new ConfigError(`${where}: 非法 level "${levelMatch[1]}"`);
  const priorityMatch = inline[1].match(/priority\s*=\s*(-?\d+)/);
  return { level: levelMatch[1], priority: priorityMatch ? Number(priorityMatch[1]) : undefined };
}

/**
 * 解析 lints.toml → 条目表。
 * 返回 { entries: [{lint, kind, level, priority, line}], byLint: Map<qcName, entry> }
 * 未知 section / 无法解析的行一律抛 ConfigError（fail-closed，不做"猜一下"）。
 */
export function parseLintTable(text, sourceLabel = "<lint-table>") {
  const entries = [];
  let kind = null;
  text.split(/\r?\n/).forEach((rawLine, index) => {
    const lineNo = index + 1;
    const line = stripLintTableComment(rawLine).trim();
    if (!line) return;
    const section = line.match(/^\[([A-Za-z0-9_.-]+)\]$/);
    if (section) {
      const name = section[1];
      if (!name.startsWith("lints.")) {
        throw new ConfigError(`${sourceLabel}:${lineNo}: 未知 section [${name}]（只接受 [lints.rust] / [lints.clippy]）`);
      }
      const key = name.slice("lints.".length);
      if (!SECTION_KINDS[key]) {
        throw new ConfigError(`${sourceLabel}:${lineNo}: 未知 lint 组 "${key}"（只接受 rust / clippy）`);
      }
      kind = SECTION_KINDS[key];
      return;
    }
    if (!kind) throw new ConfigError(`${sourceLabel}:${lineNo}: 条目出现在任何 [lints.*] 之前`);
    const pair = line.match(/^([A-Za-z0-9_]+)\s*=\s*(.+)$/);
    if (!pair) throw new ConfigError(`${sourceLabel}:${lineNo}: 无法解析的行 "${line}"（本门不做自由格式容错）`);
    const [, name, rawValue] = pair;
    const { level, priority } = parseLevelValue(rawValue, `${sourceLabel}:${lineNo}`);
    const lint = kind === "clippy" ? `clippy::${name}` : name;
    if (entries.some((entry) => entry.lint === lint)) {
      throw new ConfigError(`${sourceLabel}:${lineNo}: 重复条目 ${lint}`);
    }
    entries.push({ lint, kind, level, priority, line: lineNo });
  });
  if (entries.length === 0) throw new ConfigError(`${sourceLabel}: 表为空（至少一条 lint）`);
  const byLint = new Map(entries.map((entry) => [entry.lint, entry]));
  return { entries, byLint };
}

/** 条目 → rustc 旗标（组级优先条目先给，其余按文件顺序；等级 allow 走 -A）。 */
export function buildLintFlags(entries) {
  const ordered = [
    ...entries.filter((entry) => entry.priority !== undefined),
    ...entries.filter((entry) => entry.priority === undefined)
  ];
  return ordered.map((entry) => (entry.level === "allow" ? `-A${entry.lint}` : `-W${entry.lint}`));
}

/** 表内容指纹：注释/空白/顺序变动不改变指纹；level 或 priority 变动必须改变。 */
export function lintTableFingerprint(entries) {
  const canonical = [...entries]
    .map((entry) => ({ lint: entry.lint, level: entry.level, priority: entry.priority ?? null }))
    .sort((a, b) => (a.lint < b.lint ? -1 : a.lint > b.lint ? 1 : 0));
  return createHash("sha256").update(JSON.stringify(canonical)).digest("hex");
}

/** 表里 level=allow 的 lint 是**唯一的豁免通道**；其余一切诊断入账（见 parseCargoJsonStream）。 */
export function allowedLints(entries) {
  return new Set(entries.filter((entry) => entry.level === "allow").map((entry) => entry.lint));
}

export function sha256File(filePath) {
  return createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
}

// ────────────────────────────── JSON 流解析 ──────────────────────────────

export function normalizePath(fileName, repoRoot) {
  const root = repoRoot.replace(/\\/g, "/");
  const unified = String(fileName).replace(/\\/g, "/");
  // 盘符/目录大小写可能与 cargo 输出不一致（`c:\repo` vs `C:/repo`）：直接 startsWith 会失败并退化成
  // 绝对路径，导致存量全部变成"新增/消失"。故前缀匹配统一大小写不敏感（切片仍用原串，保持路径原样）。
  const haystack = unified.toLowerCase();
  const needle = root.toLowerCase();
  if (haystack.startsWith(`${needle}/`)) return unified.slice(root.length + 1);
  return unified.replace(/^\.\//, "");
}

/**
 * 解析 cargo 的 `--message-format json` 流 → 违例计数 + 异常诊断。
 *
 * 入账口径：`allowedLints`（表里显式 level=allow）之外的一切 warning/error 都入账。
 * 为什么不是"只算表里列出的"：表里的 `clippy::all` / `clippy::pedantic` 是**组**，
 * 组成员（如 `clippy::uninlined_format_args`）不会出现在表里；只认显式条目会让组拉起来的
 * 上千条违例静默漏账（实测少算约 1,800 条）。同理 clippy 默认组与 rustc 默认告警也入账。
 * 要豁免某条噪声，唯一手法是把它写进表里 `= "allow"`，这样豁免会出现在 diff 里。
 *
 * `anomalies`：未知 lint（E0602 / unknown_lints / renamed_and_removed_lints）。这类诊断没有
 * span 文件，若按"外部文件"丢弃，表里拼错的 lint 就变成永久空转（Rm 实测）。
 */
export function parseCargoJsonStream(streamText, { allowedLints, repoRoot = REPO_ROOT }) {
  const violations = new Map();
  const perLint = new Map();
  const anomalies = [];
  const skipped = {
    unparsedLines: 0,
    uncoded: 0,
    externalFiles: 0,
    explicitlyAllowed: 0,
    otherLevels: 0,
    unknownLints: 0,
    unanchored: 0
  };
  const unanchored = [];
  let messageCount = 0;

  for (const line of String(streamText).split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed.startsWith("{")) {
      if (trimmed) skipped.unparsedLines += 1;
      continue;
    }
    let record;
    try {
      record = JSON.parse(trimmed);
    } catch {
      skipped.unparsedLines += 1;
      continue;
    }
    if (record.reason !== "compiler-message") continue;
    const message = record.message || {};
    const code = message.code && message.code.code ? String(message.code.code) : null;
    if (!code) {
      skipped.uncoded += 1;
      continue;
    }
    const text = String(message.message || "");
    if (UNKNOWN_LINT_CODES.has(code) || /^unknown lint/i.test(text) || /renamed lint/i.test(text)) {
      anomalies.push({ code, level: message.level, text });
      skipped.unknownLints += 1;
      continue;
    }
    if (allowedLints.has(code)) {
      skipped.explicitlyAllowed += 1;
      continue;
    }
    if (message.level !== "warning" && message.level !== "error") {
      skipped.otherLevels += 1;
      continue;
    }
    const spans = Array.isArray(message.spans) ? message.spans : [];
    const primary = spans.find((span) => span && span.is_primary) || spans[0];
    const rawFile = primary && primary.file_name ? primary.file_name : null;
    if (!rawFile) {
      // 有 code 但无源码位置：可能是包级/宏展开诊断。既不能算外部依赖（会漏账），
      // 也无法定位到文件——fail-closed：交给人裁决，不静默丢弃。
      skipped.unanchored += 1;
      unanchored.push({ code, level: message.level, text });
      continue;
    }
    // 先归一化分隔符再判外部面：Windows 上 cargo 给的是 `C:\Users\...\.cargo\registry\...`，
    // 直接对原始串匹配 `/.cargo/registry/` 会漏（CI/Linux 与 Windows 都要正确）。
    const unifiedFile = String(rawFile).replace(/\\/g, "/");
    if (
      unifiedFile.includes("/.cargo/registry/") ||
      unifiedFile.includes("/.cargo/git/checkouts/") ||
      unifiedFile.includes("/rustup/toolchains/") ||
      unifiedFile.includes("/rustc/") ||
      unifiedFile.startsWith("..")
    ) {
      skipped.externalFiles += 1;
      continue;
    }
    const relPath = normalizePath(rawFile, repoRoot);
    const key = `${code}\u0000${relPath}`;
    violations.set(key, (violations.get(key) || 0) + 1);
    perLint.set(code, (perLint.get(code) || 0) + 1);
    messageCount += 1;
  }
  return { violations, perLint, anomalies, unanchored, skipped, messageCount };
}

// ────────────────────────────── 棘轮比对 ──────────────────────────────

export function compareWithBaseline(violations, baselineEntries) {
  const base = new Map();
  for (const entry of baselineEntries) {
    // 重复键会让棘轮静默变松（后者覆盖前者），纯函数也不能信调用方。
    const key = `${entry.lint}\u0000${entry.path}`;
    if (base.has(key)) throw new ConfigError(`基线重复键：${entry.lint} @ ${entry.path}`);
    if (!Number.isInteger(entry.count) || entry.count <= 0) {
      throw new ConfigError(`基线 count 必须正整数：${entry.lint} @ ${entry.path} = ${JSON.stringify(entry.count)}`);
    }
    base.set(key, entry.count);
  }
  const newViolations = [];
  let liveTotal = 0;
  for (const [key, count] of violations) {
    liveTotal += count;
    const baseCount = base.get(key) || 0;
    if (count > baseCount) {
      const [lint, file] = key.split("\u0000");
      newViolations.push({ lint, path: file, live: count, base: baseCount });
    }
  }
  const tightenings = [];
  let baseTotal = 0;
  for (const [key, count] of base) {
    baseTotal += count;
    const liveCount = violations.get(key) || 0;
    if (liveCount < count) {
      const [lint, file] = key.split("\u0000");
      tightenings.push({ lint, path: file, live: liveCount, base: count });
    }
  }
  newViolations.sort((a, b) => b.live - b.base - (a.live - a.base) || a.lint.localeCompare(b.lint));
  tightenings.sort((a, b) => b.base - a.base);
  return { newViolations, tightenings, liveTotal, baseTotal };
}

/** 基线条目序列化：按 lint、path 排序，count 只保留 >0。 */
export function serializeBaselineEntries(violations) {
  const rows = [];
  for (const [key, count] of violations) {
    if (count <= 0) continue;
    const [lint, file] = key.split("\u0000");
    rows.push({ lint, path: file, count });
  }
  rows.sort((a, b) => (a.lint === b.lint ? (a.path < b.path ? -1 : a.path > b.path ? 1 : 0) : a.lint < b.lint ? -1 : 1));
  return rows;
}

export function diffBaseline(oldEntries, newEntries) {
  const oldMap = new Map(oldEntries.map((entry) => [`${entry.lint}\u0000${entry.path}`, entry.count]));
  const newMap = new Map(newEntries.map((entry) => [`${entry.lint}\u0000${entry.path}`, entry.count]));
  const summary = { added: [], increased: [], decreased: [], removed: [] };
  for (const [key, count] of newMap) {
    const before = oldMap.get(key);
    const [lint, file] = key.split("\u0000");
    if (before === undefined) summary.added.push({ lint, path: file, count });
    else if (count > before) summary.increased.push({ lint, path: file, before, count });
    else if (count < before) summary.decreased.push({ lint, path: file, before, count });
  }
  for (const [key, before] of oldMap) {
    if (!newMap.has(key)) {
      const [lint, file] = key.split("\u0000");
      summary.removed.push({ lint, path: file, before });
    }
  }
  return summary;
}

export function totalOf(entries) {
  return entries.reduce((sum, entry) => sum + entry.count, 0);
}

// ────────────────────────────── 基线装载（v2：平台分册）──────────────────────────────

function assertEntryShape(entry, where) {
  if (!entry || typeof entry !== "object") throw new ConfigError(`${where}: 条目不是对象`);
  if (typeof entry.lint !== "string" || entry.lint.length === 0) throw new ConfigError(`${where}: lint 名非法`);
  if (/[\u0000\s]/.test(entry.lint)) throw new ConfigError(`${where}: lint 名含 NUL/空白（${JSON.stringify(entry.lint)}）`);
  if (typeof entry.path !== "string" || entry.path.length === 0) throw new ConfigError(`${where}: path 非法`);
  if (entry.path.includes("\u0000") || path.isAbsolute(entry.path) || entry.path.startsWith("..")) {
    throw new ConfigError(`${where}: path 必须是仓内相对路径（收到 ${entry.path}）`);
  }
  if (!Number.isInteger(entry.count) || entry.count <= 0) {
    throw new ConfigError(`${where}: count 必须是正整数（收到 ${JSON.stringify(entry.count)}）`);
  }
}

/**
 * 装载并校验基线。除形状外，还锁三件事（都是实测的 fail-open 通道）：
 *   ① version 必须等于 BASELINE_VERSION（v1 只能经 --update-baseline 重建）；
 *   ② entries 不许重复键、count 必须正、path 必须仓内相对；
 *   ③ 每条分册自带 lints_sha256（表口径），全局带 clippy_toml_sha256（阈值口径）。
 */
export function loadBaseline(baselinePath) {
  let text;
  try {
    text = fs.readFileSync(baselinePath, "utf8");
  } catch {
    throw new ConfigError(`基线不存在：${baselinePath}（先跑 --update-baseline 并 review diff）`);
  }
  let doc;
  try {
    doc = JSON.parse(text);
  } catch (error) {
    throw new ConfigError(`基线不是合法 JSON：${baselinePath}（${error.message}）`);
  }
  if (!doc || typeof doc !== "object") throw new ConfigError(`基线不是对象：${baselinePath}`);
  if (doc.version !== BASELINE_VERSION) {
    throw new ConfigError(
      `基线版本 ${JSON.stringify(doc.version)} 不受支持（需要 ${BASELINE_VERSION}）：` +
        `v1 是单平台扁平格式，已在 2026-11-18 因平台错配废弃；用 --update-baseline 重建（会写平台分册）`
    );
  }
  if (typeof doc.package !== "string" || doc.package.length === 0) throw new ConfigError("基线缺 package");
  if (!Array.isArray(doc.targets) || doc.targets.length === 0) throw new ConfigError("基线缺 targets");
  if (typeof doc.clippy_toml_sha256 !== "string" || doc.clippy_toml_sha256.length !== 64) {
    throw new ConfigError("基线缺 clippy_toml_sha256（阈值口径指纹）");
  }
  if (!doc.platforms || typeof doc.platforms !== "object" || Object.keys(doc.platforms).length === 0) {
    throw new ConfigError("基线缺 platforms 分册（至少一个平台）");
  }
  for (const [platform, section] of Object.entries(doc.platforms)) {
    const where = `platforms.${platform}`;
    if (!section || typeof section !== "object") throw new ConfigError(`${where}: 分册不是对象`);
    if (typeof section.lints_sha256 !== "string" || section.lints_sha256.length !== 64) {
      throw new ConfigError(`${where}: 缺 lints_sha256`);
    }
    if (!Array.isArray(section.entries)) throw new ConfigError(`${where}: 缺 entries 数组`);
    const seen = new Set();
    for (const entry of section.entries) {
      assertEntryShape(entry, where);
      const key = `${entry.lint}\u0000${entry.path}`;
      if (seen.has(key)) throw new ConfigError(`${where}: 重复键 ${entry.lint} @ ${entry.path}`);
      seen.add(key);
    }
  }
  return doc;
}

export function platformSection(doc, platform, baselinePath) {
  const section = doc.platforms[platform];
  if (!section) {
    const available = Object.keys(doc.platforms).join(" / ");
    throw new ConfigError(
      `平台分册缺失：没有 platforms.${platform}（现有：${available}）。` +
        `cfg(windows)/cfg(unix) 分支令各平台违例集合不同，禁止套用别平台的口径；` +
        `请在 ${platform} 上跑 --update-baseline 生成该分册（不会改动其他分册）后提交 ${baselinePath}`
    );
  }
  return section;
}

// ────────────────────────────── 进程与环境 ──────────────────────────────

export function buildCargoCommand({ repoRoot, packageName, allTargets, flags, lintTable, invokeRustTarget }) {
  const targets = allTargets ? ["--all-targets"] : ["--lib", "--bins"];
  return {
    command: process.execPath,
    args: [
      path.join(repoRoot, invokeRustTarget),
      "check",
      "clippy",
      "-p",
      packageName,
      ...targets,
      "--message-format",
      "json",
      "--",
      ...flags
    ],
    description:
      `node ${invokeRustTarget} check clippy -p ${packageName} ${targets.join(" ")} ` +
      `--message-format json -- ${flags.join(" ")}   # 表：${lintTable}`
  };
}

function childEnv(clippyConfDir) {
  const env = { ...process.env, CLIPPY_CONF_DIR: clippyConfDir, CARGO_TERM_COLOR: "never" };
  // 门自持 lint 级别：继承 -D warnings 会把可数的 warning 变 error（clippy 以 101 退出），
  // 棘轮失去"记账"面。RUSTFLAGS 的严格性由既有 cargo:check:shared 步骤承担。
  delete env.RUSTFLAGS;
  delete env.CARGO_ENCODED_RUSTFLAGS;
  return env;
}

function readToolchainVersions() {
  const run = (command, args) => {
    const result = spawnSync(command, args, { encoding: "utf8" });
    return result.status === 0 ? String(result.stdout).trim() : "unknown";
  };
  return { rustc: run("rustc", ["--version"]), clippy: run("cargo", ["clippy", "--version"]) };
}

export function clippyConfigFingerprint(configDir) {
  const file = path.join(configDir, DEFAULTS.clippyTomlName);
  let buffer;
  try {
    buffer = fs.readFileSync(file);
  } catch {
    throw new ConfigError(
      `clippy 阈值配置缺失：${file}（阈值参与口径指纹，缺了就不能判——如 fn_params_excessive_bools 的 ` +
        `max-fn-params-bools 会把 114 条基线静默变成"可瘦身"）`
    );
  }
  return createHash("sha256").update(buffer).digest("hex");
}

// ────────────────────────────── CLI ──────────────────────────────

const USAGE = `用法：node scripts/check-clippy-baseline.mjs [选项]

  --baseline <path>        基线文件（默认 ${DEFAULTS.baseline}）
  --platform <name>        平台分册（默认 process.platform：${process.platform}）
  --update-baseline        用本次扫描结果重写**本平台**分册（人工 review diff 后提交）
  --allow-growth           --update-baseline 时允许新增/上涨（默认拒写，见 exit 1）
  --all-targets            扫描面含测试目标（默认 --lib --bins；测试内 unwrap 由 clippy.toml 豁免）
  --from-json <file>       不跑 cargo，直接解析已捕获的 cargo JSON 流（自检/离线复算；与 --update-baseline 互斥）
  --lints <path>           lint 表路径（默认 ${DEFAULTS.lintTable}；自检用）
  --config-dir <path>      clippy.toml 目录（默认 ${DEFAULTS.clippyConfDir}）
  --package <name>         扫描包（默认 ${DEFAULTS.packageName}）
  --json                   输出机器可读结果（stdout 只有 JSON）
  --print-command          只打印将执行的 cargo 命令
  -h, --help               本帮助

退出码：0 通过 / 1 新增或恶化（或重基线检出上涨且未 --allow-growth）/ 2 门自身错误`;

export function parseArgs(argv) {
  const options = {
    baseline: DEFAULTS.baseline,
    lintTable: DEFAULTS.lintTable,
    clippyConfDir: DEFAULTS.clippyConfDir,
    packageName: DEFAULTS.packageName,
    platform: process.platform,
    allTargets: false,
    json: false,
    updateBaseline: false,
    allowGrowth: false,
    printCommand: false,
    fromJson: null,
    baselineExplicit: false,
    help: false
  };
  const takesValue = {
    "--baseline": "baseline",
    "--lints": "lintTable",
    "--config-dir": "clippyConfDir",
    "--package": "packageName",
    "--platform": "platform",
    "--from-json": "fromJson"
  };
  for (let i = 0; i < argv.length; i += 1) {
    const token = argv[i];
    if (token === "-h" || token === "--help") {
      options.help = true;
      continue;
    }
    if (token in takesValue) {
      const value = argv[i + 1];
      if (!value || value.startsWith("--")) throw new ConfigError(`${token} 缺少参数值`);
      options[takesValue[token]] = value;
      if (token === "--baseline") options.baselineExplicit = true;
      i += 1;
      continue;
    }
    if (token === "--update-baseline") {
      options.updateBaseline = true;
      continue;
    }
    if (token === "--allow-growth") {
      options.allowGrowth = true;
      continue;
    }
    if (token === "--all-targets") {
      options.allTargets = true;
      continue;
    }
    if (token === "--json") {
      options.json = true;
      continue;
    }
    if (token === "--print-command") {
      options.printCommand = true;
      continue;
    }
    throw new ConfigError(`未知参数 "${token}"（本门零容错，不做前缀匹配）`);
  }
  if (options.allowGrowth && !options.updateBaseline) {
    throw new ConfigError("--allow-growth 只在 --update-baseline 下有效");
  }
  if (options.fromJson && options.updateBaseline && !options.baselineExplicit) {
    throw new ConfigError(
      "--from-json 与 --update-baseline 需同时显式指定 --baseline（禁止把夹具流写进默认基线；自检用临时路径）"
    );
  }
  return options;
}

function emitHuman(lines) {
  for (const line of lines) process.stdout.write(`${line}\n`);
}

function writeBaseline(baselinePath, doc) {
  const tmp = `${baselinePath}.tmp`;
  fs.writeFileSync(tmp, `${JSON.stringify(doc, null, 2)}\n`, "utf8");
  fs.renameSync(tmp, baselinePath);
}

function main(argv) {
  const options = parseArgs(argv);
  if (options.help) {
    emitHuman([USAGE]);
    return 0;
  }

  const repoRoot = REPO_ROOT;
  const lintTablePath = path.resolve(repoRoot, options.lintTable);
  const baselinePath = path.resolve(repoRoot, options.baseline);
  const clippyConfDir = path.resolve(repoRoot, options.clippyConfDir);

  // 夹具流（--from-json）永远不许写进受控基线：那是"用自检数据覆盖真实存量账"。
  if (options.fromJson && options.updateBaseline && baselinePath === path.resolve(repoRoot, DEFAULTS.baseline)) {
    throw new ConfigError(
      `拒绝：--from-json 不得写入受控基线 ${DEFAULTS.baseline}（夹具流只能写 --baseline 指定的临时文件）`
    );
  }

  let tableText;
  try {
    tableText = fs.readFileSync(lintTablePath, "utf8");
  } catch (error) {
    throw new ConfigError(`lint 表不可读：${lintTablePath}（${error.message}）`);
  }
  const { entries } = parseLintTable(tableText, options.lintTable);
  const flags = buildLintFlags(entries);
  const fingerprint = lintTableFingerprint(entries);
  const exempt = allowedLints(entries);
  const clippyTomlSha = clippyConfigFingerprint(clippyConfDir);

  const targets = options.allTargets ? ["--all-targets"] : ["--lib", "--bins"];
  const cargo = buildCargoCommand({
    repoRoot,
    packageName: options.packageName,
    allTargets: options.allTargets,
    flags,
    lintTable: options.lintTable,
    invokeRustTarget: DEFAULTS.invokeRustTarget
  });

  if (options.printCommand) {
    emitHuman([cargo.description]);
    return 0;
  }

  // 口径漂移与元数据不符必须在**跑 cargo 之前**拒掉：否则要白等一次全量 clippy 才拿到与代码无关的 exit 2。
  let previousDoc = null;
  let section = null;
  if (!options.updateBaseline) {
    previousDoc = loadBaseline(baselinePath);
    assertBaselineMatchesCall(previousDoc, options, targets, clippyTomlSha);
    section = platformSection(previousDoc, options.platform, options.baseline);
    if (section.lints_sha256 !== fingerprint) {
      throw new ConfigError(
        `lint 表已变更（表指纹 ${fingerprint.slice(0, 16)}… ≠ platforms.${options.platform} 基线 ` +
          `${String(section.lints_sha256).slice(0, 16)}…）：改表即改口径，必须 npm run rust:lint:update 重基线并 review diff`
      );
    }
  } else if (fs.existsSync(baselinePath)) {
    // 重基线也要先校验旧基线可读、可合并，避免写坏其他平台分册。
    previousDoc = loadBaseline(baselinePath);
    assertBaselineMatchesCall(previousDoc, options, targets, clippyTomlSha);
  }

  let streamText;
  let toolchain;
  if (options.fromJson) {
    try {
      streamText = fs.readFileSync(path.resolve(repoRoot, options.fromJson), "utf8");
    } catch (error) {
      throw new ConfigError(`--from-json 文件不可读：${options.fromJson}（${error.message}）`);
    }
    toolchain = { rustc: "from-json", clippy: "from-json" };
  } else {
    toolchain = readToolchainVersions();
    if (!fs.existsSync(path.join(repoRoot, DEFAULTS.invokeRustTarget))) {
      throw new ConfigError(`缺少 ${DEFAULTS.invokeRustTarget}（Rust target 槽位调用器）`);
    }
    const run = spawnSync(cargo.command, cargo.args, {
      cwd: repoRoot,
      env: childEnv(clippyConfDir),
      encoding: "utf8",
      maxBuffer: 1024 * 1024 * 1024
    });
    if (run.error) throw new ConfigError(`无法启动 cargo：${run.error.message}`);
    if (run.stderr) process.stderr.write(run.stderr.endsWith("\n") ? run.stderr : `${run.stderr}\n`);
    if (run.status !== 0) {
      // 编译失败 / clippy 崩溃 / 工具链缺失：门不能"读不到数据却放行"。统一归 2（门自身无法裁决），
      // 原生退出码写进消息——否则 cargo 的 1（配置/包名错）会与"新增债 1"同码，人无法区分。
      throw new ConfigError(
        `cargo clippy 未能完成扫描（原生退出码 ${run.status === null ? "signal" : run.status}）：` +
          `编译失败或工具链缺失，门 fail-closed，不据此判红绿`
      );
    }
    streamText = run.stdout;
  }

  const { violations, perLint, anomalies, unanchored, skipped, messageCount } = parseCargoJsonStream(streamText, {
    allowedLints: exempt,
    repoRoot
  });

  if (anomalies.length > 0) {
    const names = [...new Set(anomalies.map((row) => row.text))].slice(0, 10).join("；");
    throw new ConfigError(
      `lint 表里有 clippy 不认识的 lint（${anomalies.length} 条诊断）：${names}。` +
        `拼错的 lint 不会报错、只会永久空转，必须立刻修表`
    );
  }
  if (unanchored.length > 0) {
    const names = [...new Set(unanchored.map((row) => `${row.code}: ${row.text}`))].slice(0, 10).join("；");
    throw new ConfigError(
      `有 ${unanchored.length} 条诊断带 lint code 但没有源码位置，无法归属到本仓文件：${names}。` +
        `门拒绝静默丢弃（那会漏账）；确认属外部/宏来源后，可在 lints.toml 里显式 allow 该 lint`
    );
  }
  if (messageCount === 0) {
    throw new ConfigError(
      `扫描面失效：0 条可计数诊断（表内 allow 之外没有 warning/error）。` +
        `门拒绝据此判绿——检查 cargo clippy 是否真的在跑、--message-format json 是否被吞`
    );
  }

  if (options.updateBaseline) {
    const newEntries = serializeBaselineEntries(violations);
    const previousSection = previousDoc ? previousDoc.platforms[options.platform] : null;
    const delta = previousSection ? diffBaseline(previousSection.entries, newEntries) : null;
    // 写入只在两种情形被允许：① 纯收缩（无新增/上涨）；② 显式 --allow-growth（承认增长）。
    // 首次为某平台建分册同样属于"一次性接收全部存量"，也要显式承认——否则删掉分册即可洗白新增债。
    const growth = delta ? delta.added.length + delta.increased.length : newEntries.length;
    if (growth > 0 && !options.allowGrowth) {
      const lines = delta
        ? [
            `[clippy-baseline] 拒绝写入：本次相对 platforms.${options.platform} 有 ${delta.added.length} 个新增键 / ` +
              `${delta.increased.length} 个计数上涨（共 ${growth} 处）——重基线不是"消红"工具`,
            ...(delta.increased || []).slice(0, 10).map((row) => `  ↑ ${row.lint}  ${row.path}  ${row.before} → ${row.count}`),
            ...(delta.added || []).slice(0, 10).map((row) => `  + ${row.lint}  ${row.path}  ${row.count}`)
          ]
        : [
            `[clippy-baseline] 拒绝写入：platforms.${options.platform} 是首次建册，将一次性接收 ${newEntries.length} 条存量——` +
              `等于承认当前全部债务，必须显式 --allow-growth`
          ];
      lines.push("[clippy-baseline] 先修代码；若确属有意变更（工具链升级/新 lint 纳入/新平台建册），加 --allow-growth 重跑");
      emitHuman(lines);
      return 1;
    }
    const platforms = previousDoc ? { ...previousDoc.platforms } : {};
    platforms[options.platform] = {
      toolchain: `${toolchain.rustc} / ${toolchain.clippy}`,
      generated_at: new Date().toISOString(),
      lints_sha256: fingerprint,
      entries: newEntries
    };
    const doc = {
      version: BASELINE_VERSION,
      note:
        "Rust lint 棘轮基线（存量冻结，按平台分册）：只许删条目或下调 count；新增/恶化由 npm run rust:lint 判红；" +
        "重基线须人工 review diff 后提交（含上涨时需 --allow-growth）。",
      package: options.packageName,
      targets,
      clippy_toml_sha256: clippyTomlSha,
      platforms
    };
    writeBaseline(baselinePath, doc);
    const total = totalOf(newEntries);
    const lines = [
      `[clippy-baseline] 已写入 platforms.${options.platform}：${path.relative(repoRoot, baselinePath)}` +
        `（${newEntries.length} 条键 / ${total} 条违例）`,
      `[clippy-baseline] 表指纹 ${fingerprint.slice(0, 16)}…  阈值指纹 ${clippyTomlSha.slice(0, 16)}…  toolchain：${toolchain.clippy}`
    ];
    if (!delta) {
      lines.push(`[clippy-baseline] 首次为 platforms.${options.platform} 建基线：${total} 条全部计入存量（无旧账可比）`);
    }
    if (delta) {
      lines.push(
        `[clippy-baseline] 相对旧分册 diff：新增 ${delta.added.length} / 上涨 ${delta.increased.length} / ` +
          `下降 ${delta.decreased.length} / 消失 ${delta.removed.length}`
      );
      for (const row of delta.increased.slice(0, 10)) lines.push(`  ↑ ${row.lint}  ${row.path}  ${row.before} → ${row.count}`);
      for (const row of delta.added.slice(0, 10)) lines.push(`  + ${row.lint}  ${row.path}  ${row.count}`);
    }
    const stale = Object.entries(platforms).filter(([name, value]) => name !== options.platform && value.lints_sha256 !== fingerprint);
    if (stale.length > 0) {
      lines.push(
        `[clippy-baseline] 注意：其他平台分册（${stale.map(([name]) => name).join(" / ")}）仍是旧表指纹，` +
          `需在各平台分别 --update-baseline`
      );
    }
    if (skipped.explicitlyAllowed > 0 || skipped.externalFiles > 0) {
      lines.push(
        `[clippy-baseline] 未入账：表内 allow 豁免 ${skipped.explicitlyAllowed} 条 / 外部文件 ${skipped.externalFiles} 条` +
          `（豁免的唯一通道是 lints.toml 里的 = "allow"，改它必然出现在 diff 里）`
      );
    }
    if (options.json) {
      process.stdout.write(
        `${JSON.stringify({ ok: true, updated: true, platform: options.platform, entries: newEntries.length, total }, null, 2)}\n`
      );
    } else {
      emitHuman(lines);
    }
    return 0;
  }

  const { newViolations, tightenings, liveTotal, baseTotal } = compareWithBaseline(violations, section.entries);
  // 反 fail-open：实测远低于存量 ⇒ 扫描面大概率失效（--message-format 被吞、cfg 变体没编到、表被改小）。
  // 正常修复会分批把基线调低；一次性掉一半以上必须先解释再重基线。
  if (baseTotal > 0 && liveTotal < baseTotal * GUARD_MIN_LIVE_RATIO) {
    throw new ConfigError(
      `疑似扫描面收缩：实测 ${liveTotal} 条，基线 ${baseTotal} 条（< ${Math.round(GUARD_MIN_LIVE_RATIO * 100)}%）。` +
        `门拒绝据此判绿——确认是批量修复后，用 --update-baseline（如仍有上涨需 --allow-growth）显式收基线`
    );
  }

  const ok = newViolations.length === 0;
  const summary = {
    ok,
    platform: options.platform,
    package: options.packageName,
    targets,
    lintsFingerprint: fingerprint,
    clippyTomlFingerprint: clippyTomlSha,
    liveTotal,
    baseTotal,
    newViolations,
    tightenings,
    skipped,
    messageCount,
    unanchored: unanchored.length,
    perLint: Object.fromEntries([...perLint.entries()].sort((a, b) => b[1] - a[1]))
  };

  if (options.json) {
    process.stdout.write(`${JSON.stringify(summary, null, 2)}\n`);
    return ok ? 0 : 1;
  }

  const lines = [
    `[clippy-baseline] 扫描：${options.packageName}（${targets.join(" ")}）@${options.platform}  ` +
      `表=${options.lintTable}#${fingerprint.slice(0, 12)}  toolchain=${toolchain.clippy}`,
    `[clippy-baseline] 实测 ${liveTotal} / 基线 ${baseTotal}　新增/恶化 ${newViolations.length}　可瘦身 ${tightenings.length}`
  ];
  if (ok) {
    lines.push("[clippy-baseline] PASS：无新增债（存量见 scripts/clippy-baseline.json）");
    for (const row of tightenings.slice(0, 5)) {
      lines.push(`  ↓ 可瘦身 ${row.lint}  ${row.path}  ${row.base} → ${row.live}（基线可下调）`);
    }
  } else {
    lines.push("[clippy-baseline] FAIL：以下键是基线没有、或计数已恶化的（存量不判红，新增必红）");
    for (const row of newViolations.slice(0, 20)) {
      lines.push(`  + ${row.lint}  ${row.path}  live ${row.live} / base ${row.base}`);
    }
    if (newViolations.length > 20) lines.push(`  …另有 ${newViolations.length - 20} 条（--json 看全量）`);
    lines.push("[clippy-baseline] 修复：优先改代码（?/expect 带理由、From/TryFrom、#[expect(..., reason=...)]）");
    lines.push("[clippy-baseline] 若确属口径变化（工具链升级/表变更）：review 后 npm run rust:lint:update 提交基线收缩");
  }
  emitHuman(lines);
  return ok ? 0 : 1;
}

/** 基线的扫描面元数据必须与本次调用一致，否则"基线是 A 包/别口径的，门在扫 B 包"仍会 PASS。 */
export function assertBaselineMatchesCall(doc, options, targets, clippyTomlSha) {
  if (doc.package !== options.packageName) {
    throw new ConfigError(
      `基线扫描面不符：基线是 package=${doc.package}，本次是 ${options.packageName}（换包等于换口径，需另建基线或改回）`
    );
  }
  const sameTargets = doc.targets.length === targets.length && doc.targets.every((value, index) => value === targets[index]);
  if (!sameTargets) {
    throw new ConfigError(
      `基线扫描面不符：基线 targets=${JSON.stringify(doc.targets)}，本次是 ${JSON.stringify(targets)}（口径必须逐项一致）`
    );
  }
  if (doc.clippy_toml_sha256 !== clippyTomlSha) {
    throw new ConfigError(
      `clippy.toml 已变更（阈值指纹 ${clippyTomlSha.slice(0, 16)}… ≠ 基线 ${String(doc.clippy_toml_sha256).slice(0, 16)}…）：` +
        `改阈值即改口径（例如 max-fn-params-bools 决定 114 条基线），必须重基线并 review diff`
    );
  }
}

/** 入口守卫：必须 realpath 判定（符号链接调用不能静默不执行），win32 另做小写化。 */
export function isDirectInvocation(argv1 = process.argv[1]) {
  if (!argv1) return false;
  try {
    const selfPath = fs.realpathSync(fileURLToPath(import.meta.url));
    const invokedPath = fs.realpathSync(path.resolve(argv1));
    if (process.platform === "win32") return selfPath.toLowerCase() === invokedPath.toLowerCase();
    return selfPath === invokedPath;
  } catch {
    return false;
  }
}

if (isDirectInvocation()) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (error) {
    if (error instanceof ConfigError) {
      process.stderr.write(`[clippy-baseline] 拒绝：${error.message}\n`);
      process.exitCode = 2;
    } else {
      throw error;
    }
  }
}
