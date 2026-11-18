#!/usr/bin/env node
/**
 * scripts/check-clippy-baseline.spec.mjs —— 棘轮门的**负样本自检**（判据：门自己会拒绝非法样本）
 *
 * 为什么需要它：门的承诺是"新增/恶化必红、口径漂移必拒"。承诺若没有可执行的反例，就只是说法
 * （AGENTS.md 常载命约 2：凡机械可查的承诺，配一条非零退出的命令）。本文件同时是 CI 步骤
 * `npm run rust:lint:selftest` 的实体——**它进 CI/verify，否则"门会不会空转"永远只靠读代码**。
 *
 * 覆盖（对应 2026-11-18 两路独立审核提出的 fail-open 面）：
 *   A 纯函数：表解析/旗标/指纹/比对/序列化/diff/JSON 流计数（含未知 lint 异常）
 *   B 基线装载：v2 版本、重复键、非正 count、绝对路径、缺阈值指纹、平台分册缺失、元数据不符
 *   C CLI 端到端：绿/红/口径漂移/平台缺失/元数据不符/阈值变更/空流/扫描面收缩/未知 lint/
 *     --from-json 与 --update-baseline 互斥/缺文件/--allow-growth 误用/重基线拒写与放行/分册保留
 *   D 进程面：桩 cargo（证明 RUSTFLAGS 被清空、CLIPPY_CONF_DIR 已注入、原生非零→门 exit 2）、
 *     符号链接入口（守卫用 realpath，不能静默不执行）
 *
 * 纯 Node（node:assert + spawnSync），不依赖网络；除临时目录外不写仓库文件。
 * 用法：node scripts/check-clippy-baseline.spec.mjs　退出码：0 全通过 / 1 有用例失败
 */
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import {
  allowedLints,
  assertBaselineMatchesCall,
  buildLintFlags,
  clippyConfigFingerprint,
  compareWithBaseline,
  ConfigError,
  diffBaseline,
  lintTableFingerprint,
  loadBaseline,
  normalizePath,
  parseArgs,
  parseCargoJsonStream,
  parseLintTable,
  platformSection,
  serializeBaselineEntries,
  totalOf
} from "./check-clippy-baseline.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, "..");
const GATE = path.join(HERE, "check-clippy-baseline.mjs");
const FIXTURES = path.join(HERE, "fixtures", "clippy");
const FIXTURE_TABLE = path.join(FIXTURES, "lints.toml");
const FIXTURE_STREAM = path.join(FIXTURES, "live-warnings.jsonl");
const REAL_CLIPPY_CONF_DIR = path.join(REPO_ROOT, ".meta", "lints");

const results = [];
function check(name, fn) {
  try {
    fn();
    results.push({ name, ok: true });
    process.stdout.write(`PASS  ${name}\n`);
  } catch (error) {
    results.push({ name, ok: false, error });
    process.stdout.write(`FAIL  ${name}\n      ${String(error.message).split("\n").join("\n      ")}\n`);
  }
}

const fixtureTableText = fs.readFileSync(FIXTURE_TABLE, "utf8");
const { entries: fixtureEntries } = parseLintTable(fixtureTableText, "fixtures/clippy/lints.toml");
const FIXTURE_FP = lintTableFingerprint(fixtureEntries);
const CLIPPY_SHA = clippyConfigFingerprint(REAL_CLIPPY_CONF_DIR);
const TARGETS = ["--lib", "--bins"];

/**
 * 夹具流的真值（入账口径：表内 allow 之外一律入账）：
 *   a.rs clippy::unwrap_used ×2（warning + error）、b.rs unreachable_pub ×1、
 *   a.rs clippy::needless_return ×1（表外但未 allow ⇒ 必须入账，防"组长出来的违例漏账"）、
 *   c.rs clippy::redundant_clone ×1（Windows 反斜杠路径归一化后必须落到同一键）。
 * 不入账：doc_markdown（表内 allow）、.cargo/registry 内文件（unix 与 windows 两种写法）、
 *   无 code 的 rustc 消息、note 级。
 */
const EXPECTED = [
  { lint: "clippy::needless_return", path: "crates/x/a.rs", count: 1 },
  { lint: "clippy::redundant_clone", path: "crates/x/c.rs", count: 1 },
  { lint: "clippy::unwrap_used", path: "crates/x/a.rs", count: 2 },
  { lint: "unreachable_pub", path: "crates/x/b.rs", count: 1 }
];

function tmpDir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), "clippy-baseline-spec-"));
}

function writeJson(file, value) {
  fs.writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`, "utf8");
  return file;
}

/**
 * 构造 v2 基线文档；默认与夹具表 / 仓内 clippy.toml / 默认扫描面一致。
 * extraPlatforms 用于"重基线保留其他分册"用例。
 */
function baselineDoc({
  platform = "linux",
  entries = EXPECTED,
  packageName = "pony-agent-core",
  targets = TARGETS,
  clippySha = CLIPPY_SHA,
  version = 2,
  extraPlatforms = {}
} = {}) {
  const platforms = { ...extraPlatforms };
  platforms[platform] = {
    toolchain: "spec/from-json",
    generated_at: "2026-11-18T00:00:00.000Z",
    lints_sha256: FIXTURE_FP,
    entries
  };
  return {
    version,
    note: "spec fixture",
    package: packageName,
    targets,
    clippy_toml_sha256: clippySha,
    platforms
  };
}

function runGate(args, { env = {}, cwd = REPO_ROOT } = {}) {
  const run = spawnSync(process.execPath, [GATE, ...args], {
    cwd,
    env: { ...process.env, ...env },
    encoding: "utf8"
  });
  return { status: run.status, stdout: run.stdout || "", stderr: run.stderr || "" };
}

const FROM_JSON = ["--from-json", FIXTURE_STREAM, "--lints", FIXTURE_TABLE, "--platform", "linux"];

// ── A. 纯函数 ───────────────────────────────────────────────────────────
check("A1 表解析：rust/clippy 分组、内联 priority、allow 语义", () => {
  assert.equal(fixtureEntries.length, 5);
  const byLint = new Map(fixtureEntries.map((entry) => [entry.lint, entry]));
  assert.equal(byLint.get("unreachable_pub").kind, "rust");
  assert.equal(byLint.get("missing_docs").level, "allow");
  assert.equal(byLint.get("clippy::pedantic").priority, -1);
  assert.equal(byLint.get("clippy::doc_markdown").level, "allow");
});

check("A2 表解析：非法 section / 非法值 / 重复条目 / 空表 必须 fail-closed", () => {
  assert.throws(() => parseLintTable("[lints.unknown]\nfoo = \"warn\"\n"), ConfigError);
  assert.throws(() => parseLintTable("[lints.clippy]\nfoo = \"loud\"\n"), ConfigError);
  assert.throws(() => parseLintTable("[lints.clippy]\nfoo = \"warn\"\nfoo = \"warn\"\n"), ConfigError);
  assert.throws(() => parseLintTable("[lints.clippy]\nnot a pair\n"), ConfigError);
  assert.throws(() => parseLintTable(""), ConfigError);
  assert.throws(() => parseLintTable("foo = \"warn\"\n"), ConfigError);
});

check("A3 旗标：组级优先条目在前，allow 走 -A", () => {
  const flags = buildLintFlags(fixtureEntries);
  assert.equal(flags[0], "-Wclippy::pedantic");
  assert.ok(flags.includes("-Amissing_docs"));
  assert.ok(flags.includes("-Aclippy::doc_markdown"));
  assert.ok(flags.includes("-Wclippy::unwrap_used"));
});

check("A4 指纹：注释/空白/顺序不变，level 变动必变", () => {
  const reordered = `# 换顺序\n\n[lints.clippy]\nunwrap_used = "warn"\npedantic = { level = "warn", priority = -1 }\ndoc_markdown = "allow"\n[lints.rust]\nmissing_docs = "allow"\nunreachable_pub = "warn"\n`;
  assert.equal(lintTableFingerprint(parseLintTable(reordered).entries), FIXTURE_FP);
  const changed = parseLintTable(fixtureTableText.replace('unreachable_pub = "warn"', 'unreachable_pub = "allow"')).entries;
  assert.notEqual(lintTableFingerprint(changed), FIXTURE_FP);
});

check("A5 比对：等于基线放行 / 恶化判新增 / 新键判新增 / 变少只算可瘦身", () => {
  const live = new Map([
    ["clippy::unwrap_used\u0000crates/x/a.rs", 2],
    ["unreachable_pub\u0000crates/x/b.rs", 1],
    ["clippy::needless_return\u0000crates/x/a.rs", 1],
    ["clippy::redundant_clone\u0000crates/x/c.rs", 1]
  ]);
  const equal = compareWithBaseline(live, EXPECTED);
  assert.equal(equal.newViolations.length, 0);
  assert.equal(equal.tightenings.length, 0);
  assert.equal(equal.liveTotal, 5);

  const worse = new Map(live);
  worse.set("clippy::unwrap_used\u0000crates/x/a.rs", 3);
  assert.deepEqual(compareWithBaseline(worse, EXPECTED).newViolations.map((row) => row.live), [3]);

  const brandNew = new Map(live);
  brandNew.set("clippy::as_conversions\u0000crates/x/d.rs", 1);
  assert.equal(compareWithBaseline(brandNew, EXPECTED).newViolations.length, 1);

  const shrunk = new Map([["clippy::unwrap_used\u0000crates/x/a.rs", 1]]);
  const tightening = compareWithBaseline(shrunk, EXPECTED);
  assert.equal(tightening.newViolations.length, 0);
  assert.equal(tightening.tightenings.length, 4);
});

check("A6 序列化：按 lint/path 排序、过滤 0 值、totalOf 正确", () => {
  const rows = serializeBaselineEntries(
    new Map([
      ["unreachable_pub\u0000crates/z.rs", 1],
      ["clippy::unwrap_used\u0000crates/a.rs", 2],
      ["clippy::unwrap_used\u0000crates/b.rs", 0]
    ])
  );
  assert.deepEqual(rows, [
    { lint: "clippy::unwrap_used", path: "crates/a.rs", count: 2 },
    { lint: "unreachable_pub", path: "crates/z.rs", count: 1 }
  ]);
  assert.equal(totalOf(rows), 3);
});

check("A7 基线 diff：新增/上涨/下降/消失 四类都要报出来", () => {
  const delta = diffBaseline(
    [{ lint: "a", path: "x.rs", count: 5 }, { lint: "b", path: "y.rs", count: 1 }],
    [{ lint: "a", path: "x.rs", count: 7 }, { lint: "c", path: "z.rs", count: 1 }]
  );
  assert.equal(delta.increased.length, 1);
  assert.equal(delta.added.length, 1);
  assert.equal(delta.removed.length, 1);
  assert.equal(delta.decreased.length, 0);
});

check("A8 JSON 流：allow 之外一律入账（含表外 lint），过滤外部 crate / 无 code / note", () => {
  const exempt = allowedLints(fixtureEntries);
  assert.deepEqual([...exempt].sort(), ["clippy::doc_markdown", "missing_docs"]);
  const parsed = parseCargoJsonStream(fs.readFileSync(FIXTURE_STREAM, "utf8"), { allowedLints: exempt, repoRoot: REPO_ROOT });
  assert.deepEqual(
    [...parsed.violations.entries()].sort(),
    [
      ["clippy::needless_return\u0000crates/x/a.rs", 1],
      ["clippy::redundant_clone\u0000crates/x/c.rs", 1],
      ["clippy::unwrap_used\u0000crates/x/a.rs", 2],
      ["unreachable_pub\u0000crates/x/b.rs", 1]
    ].sort()
  );
  assert.equal(parsed.perLint.get("clippy::unwrap_used"), 2);
  assert.equal(parsed.skipped.explicitlyAllowed, 1);
  assert.equal(parsed.skipped.externalFiles, 2);
  assert.equal(parsed.skipped.uncoded, 1);
  assert.equal(parsed.skipped.otherLevels, 1);
  assert.equal(parsed.skipped.unparsedLines >= 1, true);
  assert.equal(parsed.anomalies.length, 0);
});

check("A9 JSON 流：未知 lint（E0602，无 span）必须进 anomalies，不得被当外部文件丢弃", () => {
  const stream = [
    JSON.stringify({ reason: "compiler-message", message: { code: { code: "E0602" }, level: "warning", message: "unknown lint: `clippy::as_conversion_typo`", spans: [] } }),
    JSON.stringify({ reason: "compiler-message", message: { code: { code: "clippy::unwrap_used" }, level: "warning", message: "x", spans: [{ file_name: "crates/x/a.rs", is_primary: true }] } })
  ].join("\n");
  const parsed = parseCargoJsonStream(stream, { allowedLints: new Set(), repoRoot: REPO_ROOT });
  assert.equal(parsed.anomalies.length, 1);
  assert.match(parsed.anomalies[0].text, /as_conversion_typo/);
  assert.equal(parsed.violations.size, 1);
});

// ── B. 基线装载 ─────────────────────────────────────────────────────────
check("B1 装载：v2 基线通过；v1 扁平格式必须拒绝", () => {
  const dir = tmpDir();
  const doc = loadBaseline(writeJson(path.join(dir, "ok.json"), baselineDoc()));
  assert.equal(doc.version, 2);
  assert.deepEqual(Object.keys(doc.platforms), ["linux"]);
  const v1 = writeJson(path.join(dir, "v1.json"), { version: 1, package: "x", targets: TARGETS, entries: EXPECTED });
  assert.throws(() => loadBaseline(v1), /基线版本.*不受支持/);
});

check("B2 装载：重复键 / 非正 count / 非法 path / 缺阈值指纹 必须拒绝", () => {
  const dir = tmpDir();
  assert.throws(() => loadBaseline(writeJson(path.join(dir, "dup.json"), baselineDoc({ entries: [EXPECTED[0], EXPECTED[0]] }))), /重复键/);
  assert.throws(() => loadBaseline(writeJson(path.join(dir, "zero.json"), baselineDoc({ entries: [{ lint: "clippy::x", path: "a.rs", count: 0 }] }))), /正整数/);
  assert.throws(() => loadBaseline(writeJson(path.join(dir, "abs.json"), baselineDoc({ entries: [{ lint: "clippy::x", path: "/etc/passwd", count: 1 }] }))), /仓内相对路径/);
  assert.throws(() => loadBaseline(writeJson(path.join(dir, "escape.json"), baselineDoc({ entries: [{ lint: "clippy::x", path: "../outside.rs", count: 1 }] }))), ConfigError);
  const noSha = baselineDoc();
  delete noSha.clippy_toml_sha256;
  assert.throws(() => loadBaseline(writeJson(path.join(dir, "nosha.json"), noSha)), /clippy_toml_sha256/);
});

check("B3 分册：缺失平台必须拒绝并列出可用分册（禁止套用别平台口径）", () => {
  const dir = tmpDir();
  const doc = loadBaseline(writeJson(path.join(dir, "b3.json"), baselineDoc({ platform: "linux" })));
  assert.throws(() => platformSection(doc, "win32", "scripts/clippy-baseline.json"), /平台分册缺失/);
  assert.equal(platformSection(doc, "linux", "x").entries.length, EXPECTED.length);
});

check("B4 元数据：package / targets / clippy.toml 指纹任一不符都必须拒绝", () => {
  const dir = tmpDir();
  const doc = loadBaseline(writeJson(path.join(dir, "b4.json"), baselineDoc()));
  const options = { packageName: "pony-agent-core", platform: "linux" };
  assert.doesNotThrow(() => assertBaselineMatchesCall(doc, options, TARGETS, CLIPPY_SHA));
  assert.throws(() => assertBaselineMatchesCall(doc, { ...options, packageName: "other" }, TARGETS, CLIPPY_SHA), /package/);
  assert.throws(() => assertBaselineMatchesCall(doc, options, ["--all-targets"], CLIPPY_SHA), /targets/);
  assert.throws(() => assertBaselineMatchesCall(doc, options, TARGETS, "f".repeat(64)), /clippy\.toml/);
});

check("B5 参数：--allow-growth 必须配 --update-baseline；--from-json 与重基线互斥", () => {
  assert.throws(() => parseArgs(["--allow-growth"]), /--allow-growth/);
  assert.throws(() => parseArgs(["--from-json", "x.jsonl", "--update-baseline"]), /需同时显式指定 --baseline/);
  assert.doesNotThrow(() => parseArgs(["--from-json", "x.jsonl", "--update-baseline", "--baseline", "/tmp/x.json"]));
  assert.throws(() => parseArgs(["--platform"]), /缺少参数值/);
  assert.throws(() => parseArgs(["--baseline=scripts/x.json"]), /未知参数/);
  assert.equal(parseArgs(["--platform", "win32"]).platform, "win32");
});

check("B6 路径归一化：Windows 盘符大小写 / 反斜杠 / 仓内绝对路径都落到同一相对键", () => {
  const winRoot = "C:\\work\\pony-agent";
  assert.equal(normalizePath("C:/work/pony-agent/crates/x/a.rs", winRoot), "crates/x/a.rs");
  assert.equal(normalizePath("c:\\work\\pony-agent\\crates\\x\\a.rs", "C:\\work\\pony-agent"), "crates/x/a.rs");
  assert.equal(normalizePath("./crates/x/a.rs", "/repo"), "crates/x/a.rs");
  assert.equal(normalizePath("/repo/crates/x/a.rs", "/repo"), "crates/x/a.rs");
  assert.equal(normalizePath("/other/place/a.rs", "/repo"), "/other/place/a.rs");
});

// ── C. CLI 端到端 ───────────────────────────────────────────────────────
check("C1 live 与基线相符 → exit 0", () => {
  const dir = tmpDir();
  const run = runGate([...FROM_JSON, "--baseline", writeJson(path.join(dir, "b.json"), baselineDoc())]);
  assert.equal(run.status, 0, run.stderr || run.stdout);
  assert.match(run.stdout, /PASS/);
});

check("C2 基线缺键 → exit 1（新增债必红）", () => {
  const dir = tmpDir();
  const partial = baselineDoc({ entries: [EXPECTED[0], EXPECTED[1], EXPECTED[2]] });
  const run = runGate([...FROM_JSON, "--baseline", writeJson(path.join(dir, "b.json"), partial)]);
  assert.equal(run.status, 1, run.stdout);
  assert.match(run.stdout, /unreachable_pub/);
});

check("C3 表指纹不符 → exit 2（改表不重基线 = 口径漂移）", () => {
  const dir = tmpDir();
  const stale = baselineDoc();
  stale.platforms.linux.lints_sha256 = "0".repeat(64);
  const run = runGate([...FROM_JSON, "--baseline", writeJson(path.join(dir, "b.json"), stale)]);
  assert.equal(run.status, 2, run.stdout);
  assert.match(run.stderr, /lint 表已变更/);
});

check("C4 平台分册缺失 → exit 2", () => {
  const dir = tmpDir();
  const run = runGate(["--from-json", FIXTURE_STREAM, "--lints", FIXTURE_TABLE, "--platform", "win32", "--baseline", writeJson(path.join(dir, "b.json"), baselineDoc())]);
  assert.equal(run.status, 2);
  assert.match(run.stderr, /平台分册缺失/);
});

check("C5 元数据不符（换包 / 换扫描面）→ exit 2", () => {
  const dir = tmpDir();
  const runPkg = runGate([...FROM_JSON, "--baseline", writeJson(path.join(dir, "a.json"), baselineDoc({ packageName: "totally-other-package" }))]);
  assert.equal(runPkg.status, 2);
  assert.match(runPkg.stderr, /package/);
  const runTargets = runGate([...FROM_JSON, "--baseline", writeJson(path.join(dir, "b.json"), baselineDoc({ targets: ["--all-targets"] }))]);
  assert.equal(runTargets.status, 2);
  assert.match(runTargets.stderr, /targets/);
});

check("C6 clippy.toml 阈值变更 / 缺失 → exit 2（改阈值不能零红放行）", () => {
  const dir = tmpDir();
  const confDir = path.join(dir, "conf");
  fs.mkdirSync(confDir);
  fs.writeFileSync(path.join(confDir, "clippy.toml"), "max-fn-params-bools = 3\n", "utf8");
  const changed = runGate(["--from-json", FIXTURE_STREAM, "--lints", FIXTURE_TABLE, "--platform", "linux", "--config-dir", confDir, "--baseline", writeJson(path.join(dir, "b.json"), baselineDoc())]);
  assert.equal(changed.status, 2);
  assert.match(changed.stderr, /clippy\.toml 已变更/);

  const missing = runGate(["--from-json", FIXTURE_STREAM, "--lints", FIXTURE_TABLE, "--platform", "linux", "--config-dir", path.join(dir, "nope"), "--baseline", writeJson(path.join(dir, "c.json"), baselineDoc())]);
  assert.equal(missing.status, 2);
  assert.match(missing.stderr, /阈值配置缺失/);
});

check("C7 空诊断流 → exit 2（扫描面失效不得判绿）", () => {
  const dir = tmpDir();
  const empty = path.join(dir, "empty.jsonl");
  fs.writeFileSync(empty, "", "utf8");
  const run = runGate(["--from-json", empty, "--lints", FIXTURE_TABLE, "--platform", "linux", "--baseline", writeJson(path.join(dir, "b.json"), baselineDoc())]);
  assert.equal(run.status, 2, run.stdout);
  assert.match(run.stderr, /扫描面失效/);
});

check("C8 实测不足基线 50% → exit 2（疑似扫描面收缩）", () => {
  const dir = tmpDir();
  const bigEntries = Array.from({ length: 20 }, (_, i) => ({ lint: "clippy::as_conversions", path: `crates/x/f${i}.rs`, count: 10 }));
  const run = runGate([...FROM_JSON, "--baseline", writeJson(path.join(dir, "b.json"), baselineDoc({ entries: bigEntries }))]);
  assert.equal(run.status, 2, run.stdout);
  assert.match(run.stderr, /疑似扫描面收缩/);
});

check("C9 未知 lint → exit 2 且点名到具体 lint", () => {
  const dir = tmpDir();
  const stream = path.join(dir, "unknown.jsonl");
  fs.writeFileSync(
    stream,
    `${JSON.stringify({ reason: "compiler-message", message: { code: { code: "E0602" }, level: "warning", message: "unknown lint: `clippy::as_conversion_typo`", spans: [] } })}\n`,
    "utf8"
  );
  const run = runGate(["--from-json", stream, "--lints", FIXTURE_TABLE, "--platform", "linux", "--baseline", writeJson(path.join(dir, "b.json"), baselineDoc())]);
  assert.equal(run.status, 2);
  assert.match(run.stderr, /as_conversion_typo/);
});

check("C16 无源码位置的可计数诊断 → exit 2（禁止静默当外部文件丢弃）", () => {
  const dir = tmpDir();
  const stream = path.join(dir, "unanchored.jsonl");
  fs.writeFileSync(
    stream,
    `${JSON.stringify({ reason: "compiler-message", message: { code: { code: "clippy::unwrap_used" }, level: "warning", message: "package-level warning", spans: [] } })}\n`,
    "utf8"
  );
  const run = runGate(["--from-json", stream, "--lints", FIXTURE_TABLE, "--platform", "linux", "--baseline", writeJson(path.join(dir, "b.json"), baselineDoc())]);
  assert.equal(run.status, 2);
  assert.match(run.stderr, /没有源码位置/);
});

check("C10 --from-json 缺文件 → exit 2；夹具流不许写默认基线（解析期与主流程双闸）", () => {
  const dir = tmpDir();
  const missing = runGate(["--from-json", path.join(dir, "nope.jsonl"), "--lints", FIXTURE_TABLE, "--platform", "linux", "--baseline", writeJson(path.join(dir, "b.json"), baselineDoc())]);
  assert.equal(missing.status, 2);
  assert.match(missing.stderr, /--from-json 文件不可读/);

  // 未显式给 --baseline：解析期就拒（防止夹具流落到默认基线路径）
  const comboNoPath = runGate(["--from-json", FIXTURE_STREAM, "--lints", FIXTURE_TABLE, "--platform", "linux", "--update-baseline"]);
  assert.equal(comboNoPath.status, 2);
  assert.match(comboNoPath.stderr, /需同时显式指定 --baseline/);

  // 显式指向受控基线：主流程拒，且基线文件字节不得变化
  const controlled = path.join(REPO_ROOT, "scripts", "clippy-baseline.json");
  const before = fs.readFileSync(controlled, "utf8");
  const comboDefault = runGate(["--from-json", FIXTURE_STREAM, "--lints", FIXTURE_TABLE, "--platform", "linux", "--baseline", "scripts/clippy-baseline.json", "--update-baseline"]);
  assert.equal(comboDefault.status, 2);
  assert.match(comboDefault.stderr, /不得写入受控基线/);
  assert.equal(fs.readFileSync(controlled, "utf8"), before, "受控基线字节不得变化");
});

check("C11 首次建册：无 --allow-growth 拒写；加 --allow-growth 后写入 v2 分册并可复算判绿", () => {
  const dir = tmpDir();
  const baseline = path.join(dir, "b.json");
  const refused = runGate([...FROM_JSON, "--baseline", baseline, "--update-baseline"]);
  assert.equal(refused.status, 1, refused.stdout);
  assert.match(refused.stdout, /首次建册/);
  assert.equal(fs.existsSync(baseline), false, "拒写时不得创建基线文件");

  const updated = runGate([...FROM_JSON, "--baseline", baseline, "--update-baseline", "--allow-growth", "--json"]);
  assert.equal(updated.status, 0, updated.stderr);
  const doc = JSON.parse(fs.readFileSync(baseline, "utf8"));
  assert.equal(doc.version, 2);
  assert.deepEqual(doc.platforms.linux.entries, EXPECTED);
  assert.equal(doc.platforms.linux.lints_sha256, FIXTURE_FP);
  assert.equal(doc.clippy_toml_sha256, CLIPPY_SHA);
  assert.equal(runGate([...FROM_JSON, "--baseline", baseline]).status, 0);
});

check("C12 重基线检出上涨 → 默认拒写 exit 1；--allow-growth 才写", () => {
  const dir = tmpDir();
  const baseline = writeJson(path.join(dir, "b.json"), baselineDoc({ entries: [EXPECTED[2]] }));
  const before = fs.readFileSync(baseline, "utf8");
  const refused = runGate([...FROM_JSON, "--baseline", baseline, "--update-baseline"]);
  assert.equal(refused.status, 1, refused.stdout);
  assert.match(refused.stdout, /拒绝写入/);
  assert.equal(fs.readFileSync(baseline, "utf8"), before, "拒写时基线字节不得变化");
  const allowed = runGate([...FROM_JSON, "--baseline", baseline, "--update-baseline", "--allow-growth"]);
  assert.equal(allowed.status, 0, allowed.stderr);
  assert.equal(JSON.parse(fs.readFileSync(baseline, "utf8")).platforms.linux.entries.length, EXPECTED.length);
});

check("C13 重基线保留其他平台分册", () => {
  const dir = tmpDir();
  const darwin = { toolchain: "spec", generated_at: "2026-11-01T00:00:00.000Z", lints_sha256: FIXTURE_FP, entries: [EXPECTED[0]] };
  const baseline = writeJson(path.join(dir, "b.json"), baselineDoc({ extraPlatforms: { darwin } }));
  const run = runGate([...FROM_JSON, "--baseline", baseline, "--update-baseline", "--json"]);
  assert.equal(run.status, 0, run.stderr);
  const doc = JSON.parse(fs.readFileSync(baseline, "utf8"));
  assert.deepEqual(Object.keys(doc.platforms).sort(), ["darwin", "linux"]);
  assert.deepEqual(doc.platforms.darwin, darwin);
});

check("C14 --json 失败路径：exit 1 且 stdout 是纯 JSON（ok=false）", () => {
  const dir = tmpDir();
  const run = runGate([...FROM_JSON, "--baseline", writeJson(path.join(dir, "b.json"), baselineDoc({ entries: [EXPECTED[0]] })), "--json"]);
  assert.equal(run.status, 1);
  const payload = JSON.parse(run.stdout);
  assert.equal(payload.ok, false);
  assert.equal(payload.newViolations.length, 3);
  assert.equal(payload.platform, "linux");
  assert.equal(payload.clippyTomlFingerprint, CLIPPY_SHA);
});

check("C15 --print-command 只打印命令且含表内旗标（-W 与 -A 都在）", () => {
  const run = runGate(["--print-command", "--lints", FIXTURE_TABLE]);
  assert.equal(run.status, 0, run.stderr);
  assert.equal(run.stdout.split("\n").filter(Boolean).length, 1);
  assert.match(run.stdout, /check clippy -p pony-agent-core --lib --bins --message-format json --/);
  assert.match(run.stdout, /-Wclippy::unwrap_used/);
  assert.match(run.stdout, /-Amissing_docs/);
});

// ── D. 进程面 ───────────────────────────────────────────────────────────
if (process.platform !== "win32") {
  function makeCargoShim(dir, body) {
    const binDir = path.join(dir, "bin");
    fs.mkdirSync(binDir, { recursive: true });
    const shim = path.join(binDir, "cargo");
    fs.writeFileSync(shim, ["#!/usr/bin/env bash", ...body, ""].join("\n"), "utf8");
    fs.chmodSync(shim, 0o755);
    return binDir;
  }

  check("D1 桩 cargo：RUSTFLAGS 被清空、CLIPPY_CONF_DIR 已注入、流被解析 → exit 0", () => {
    const dir = tmpDir();
    const binDir = makeCargoShim(dir, [
      'if [ "$1" = "clippy" ] && [ "$2" = "--version" ]; then echo "clippy 0.0.0-shim"; exit 0; fi',
      'printf "RUSTFLAGS=[%s]\\nCLIPPY_CONF_DIR=[%s]\\n" "${RUSTFLAGS-<unset>}" "${CLIPPY_CONF_DIR-<unset>}" > "$SHIM_LOG"',
      'cat "$SHIM_STREAM"',
      'exit "${SHIM_EXIT:-0}"'
    ]);
    const log = path.join(dir, "env.log");
    const run = runGate(["--lints", FIXTURE_TABLE, "--platform", "linux", "--baseline", writeJson(path.join(dir, "b.json"), baselineDoc())], {
      env: {
        PATH: `${binDir}:${process.env.PATH}`,
        SHIM_LOG: log,
        SHIM_STREAM: FIXTURE_STREAM,
        SHIM_EXIT: "0",
        RUSTFLAGS: "-D warnings"
      }
    });
    assert.equal(run.status, 0, run.stderr || run.stdout);
    const envText = fs.readFileSync(log, "utf8");
    assert.match(envText, /RUSTFLAGS=\[<unset>\]/);
    assert.match(envText, new RegExp(`CLIPPY_CONF_DIR=\\[${REAL_CLIPPY_CONF_DIR.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\]`));
  });

  check("D2 桩 cargo 非零退出 → 门 exit 2（不得与「新增债 1」同码）", () => {
    const dir = tmpDir();
    const binDir = makeCargoShim(dir, [
      'if [ "$1" = "clippy" ] && [ "$2" = "--version" ]; then echo "clippy shim"; exit 0; fi',
      "exit 3"
    ]);
    const run = runGate(["--lints", FIXTURE_TABLE, "--platform", "linux", "--baseline", writeJson(path.join(dir, "b.json"), baselineDoc())], {
      env: { PATH: `${binDir}:${process.env.PATH}`, SHIM_STREAM: FIXTURE_STREAM, SHIM_LOG: path.join(dir, "x.log") }
    });
    assert.equal(run.status, 2, run.stdout);
    assert.match(run.stderr, /未能完成扫描/);
    assert.match(run.stderr, /3/);
  });

  check("D3 符号链接入口：守卫用 realpath，必须真正执行（不是零输出 exit 0）", () => {
    const dir = tmpDir();
    const link = path.join(dir, "gate.mjs");
    fs.symlinkSync(GATE, link);
    const run = spawnSync(process.execPath, [link, "--help"], { cwd: REPO_ROOT, encoding: "utf8" });
    assert.equal(run.status, 0);
    assert.match(run.stdout || "", /用法/);
  });
}

// ── 汇总 ────────────────────────────────────────────────────────────────
const failed = results.filter((entry) => !entry.ok);
process.stdout.write(`\n${results.length - failed.length}/${results.length} 用例通过\n`);
if (failed.length > 0) {
  for (const entry of failed) process.stdout.write(`  失败：${entry.name}\n`);
  process.exitCode = 1;
} else {
  process.exitCode = 0;
}
