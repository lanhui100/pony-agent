#!/usr/bin/env node
/**
 * 仓库内可观测性变更门禁（CI 可执行版）
 *
 * ## 为什么存在
 *
 * 项目已声明 `docs/observability.yaml::enforcement_points`（ob2 收口契约），但权威审计器
 * `audit-diff.sh` 属于本机 skill 目录（`~/.agents/skills/observability-onboarding/`），
 * **GitHub Actions runner 上没有这份 skill**。若 CI 直接调用它，只会因文件缺失变成空转，
 * 得到一个「看起来装了门禁、其实什么都没拦」的假绿。
 *
 * 所以 CI 走本脚本：规则**仍然只来自项目声明**（不在代码里硬编码任何语言关键字），
 * 只是把执行引擎换成仓库自带、零依赖、跨平台的 Node 实现。
 *
 * ## 与权威器的一致性
 *
 * `tests/acceptance/wave47/gate-conformance.spec.ts` 会在本机 skill 可用时，
 * 用同一组夹具同时跑本脚本与 `audit-diff.sh`，断言判定一致。引擎可以有第二份，
 * 判定口径不能有第二份。
 *
 * ## 判定规则
 *
 * - **D1**：`docs/observability.yaml` 出现在本次变更中，但 HEAD 提交信息不含
 *   `[decl-change-approved]` → 违规（声明变更必须显式批准）。
 * - **A（越界实现）**：除收口点自身外，任何变更文件命中某收口点的
 *   `forbidden_in_other_files[].pattern` → 违规。豁免：`tests/`、`docs/`、`fixtures/`、`*.md`
 *   （与权威器同口径）。
 * - **B（收口点失效）**：收口点声明路径不存在，或本次变更后行数跌破基线的 50% → 违规。
 *
 * 退出码：0 通过｜2 违规｜4 工具/声明错误。
 */
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const DECL_PATH = "docs/observability.yaml";
const EXEMPT_PREFIXES = ["tests/", "docs/", "fixtures/"];
const EXEMPT_SUFFIXES = [".md"];
const APPROVAL_TOKEN = "[decl-change-approved]";

const repoRoot = process.env.GATE_REPO || process.cwd();
const baseRef = resolveBaseRef();

function fail(code, message) {
  console.error(`[FAIL] ${message}`);
  process.exit(code);
}

function git(args, opts = {}) {
  return execFileSync("git", args, {
    cwd: repoRoot,
    encoding: "utf-8",
    stdio: ["ignore", "pipe", "pipe"],
    ...opts
  });
}

function resolveBaseRef() {
  const explicit = process.env.GATE_BASE;
  if (explicit) return explicit;
  const ghBase = process.env.GITHUB_BASE_REF;
  if (ghBase) return `origin/${ghBase}`;
  try {
    git(["rev-parse", "--verify", "HEAD~1"], { stdio: "ignore" });
    return "HEAD~1";
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------- 声明解析
/**
 * 解析 `enforcement_points` 块。
 *
 * 刻意只支持本项目声明实际使用的 YAML 子集：2 空格缩进、`key: value`、
 * `- item` 与 `- key: value` 列表。**遇到不认识的结构直接 fail-closed**，
 * 而不是猜——猜错会让门禁静默放行。
 */
function parseEnforcementPoints(raw) {
  const lines = raw.split(/\r?\n/);
  const start = lines.findIndex((l) => /^enforcement_points:\s*$/.test(l));
  if (start === -1) return {};

  const points = {};
  let current = null;
  let mode = null; // "guarantees" | "forbidden"

  for (let i = start + 1; i < lines.length; i++) {
    const line = lines[i];
    if (/^\S/.test(line)) break; // 回到顶层键，enforcement_points 结束
    if (!line.trim() || line.trim().startsWith("#")) continue;

    const indent = line.length - line.trimStart().length;
    const body = line.trim();

    if (indent === 2 && /^[\w-]+:\s*$/.test(body)) {
      current = body.slice(0, -1);
      points[current] = { guarantees: [], forbidden: [] };
      mode = null;
      continue;
    }
    if (!current) continue;

    if (indent === 4) {
      const pathMatch = body.match(/^path:\s*(.+)$/);
      if (pathMatch) {
        points[current].path = unquote(pathMatch[1]);
        mode = null;
        continue;
      }
      if (/^guarantees:\s*$/.test(body)) {
        mode = "guarantees";
        continue;
      }
      if (/^forbidden_in_other_files:\s*$/.test(body)) {
        mode = "forbidden";
        continue;
      }
      fail(4, `${DECL_PATH}: 收口点 ${current} 出现未支持的字段「${body}」（fail-closed）`);
    }

    if (indent === 6 && mode === "guarantees" && body.startsWith("- ")) {
      points[current].guarantees.push(unquote(body.slice(2)));
      continue;
    }
    if (indent === 6 && mode === "forbidden" && body.startsWith("- pattern:")) {
      points[current].forbidden.push({ pattern: unquote(body.slice("- pattern:".length)) });
      continue;
    }
    if (indent === 8 && mode === "forbidden" && body.startsWith("hint:")) {
      const last = points[current].forbidden[points[current].forbidden.length - 1];
      if (last) last.hint = unquote(body.slice("hint:".length));
      continue;
    }
    fail(4, `${DECL_PATH}: 收口点 ${current} 出现未支持的缩进/结构「${body}」（fail-closed）`);
  }
  return points;
}

function unquote(value) {
  const v = value.trim();
  if (v.startsWith('"') && v.endsWith('"')) {
    return JSON.parse(v); // YAML 双引号字符串的转义与 JSON 基本一致
  }
  if (v.startsWith("'") && v.endsWith("'")) {
    return v.slice(1, -1).replace(/''/g, "'");
  }
  return v;
}

// ---------------------------------------------------------------- 变更集合
function changedFiles() {
  const set = new Set();
  if (baseRef) {
    let diff = "";
    try {
      diff = git(["diff", "--name-only", `${baseRef}...HEAD`]);
    } catch {
      diff = git(["diff", "--name-only", baseRef]);
    }
    diff.split(/\r?\n/).filter(Boolean).forEach((f) => set.add(f));
  }
  // 工作区未提交改动也纳入（本地预检用；CI 上是干净的）
  try {
    git(["diff", "--name-only", "HEAD"])
      .split(/\r?\n/)
      .filter(Boolean)
      .forEach((f) => set.add(f));
    git(["ls-files", "--others", "--exclude-standard"])
      .split(/\r?\n/)
      .filter(Boolean)
      .forEach((f) => set.add(f));
  } catch {
    /* git 不可用时忽略 */
  }
  return [...set];
}

function isExempt(file) {
  if (EXEMPT_PREFIXES.some((p) => file.startsWith(p))) return true;
  return EXEMPT_SUFFIXES.some((s) => file.endsWith(s));
}

// ---------------------------------------------------------------- 规则
function ruleD1(files) {
  if (!files.includes(DECL_PATH)) return;
  const message = git(["log", "-1", "--pretty=%B"]).trim();
  if (!message.includes(APPROVAL_TOKEN)) {
    fail(
      2,
      `[D1] ${DECL_PATH} 在本次变更中被修改，但 HEAD 提交信息缺少 ${APPROVAL_TOKEN}；声明变更必须显式批准`
    );
  }
}

function ruleA(points, files) {
  let violations = 0;
  for (const [name, point] of Object.entries(points)) {
    for (const rule of point.forbidden) {
      const regex = new RegExp(rule.pattern);
      for (const file of files) {
        if (file === point.path) continue; // 收口点自身是唯一合法实现
        if (isExempt(file)) continue;
        const abs = path.join(repoRoot, file);
        if (!fs.existsSync(abs)) continue;
        const content = fs.readFileSync(abs, "utf-8");
        content.split(/\r?\n/).forEach((line, idx) => {
          if (!regex.test(line)) return;
          violations++;
          console.error(
            `[FAIL] [A] ${file}:${idx + 1} 命中 ${name} 的 forbidden pattern /${rule.pattern}/ → ${
              rule.hint ?? "该能力只能在其收口点实现"
            }`
          );
        });
      }
    }
  }
  if (violations > 0) process.exit(2);
}

function ruleB(points, files) {
  let violations = 0;
  for (const [name, point] of Object.entries(points)) {
    if (!point.path) {
      violations++;
      console.error(`[FAIL] [B] 收口点 ${name} 未声明 path`);
      continue;
    }
    const abs = path.join(repoRoot, point.path);
    if (!fs.existsSync(abs)) {
      violations++;
      console.error(`[FAIL] [B] 收口点 ${name} 的 path 不存在：${point.path}`);
      continue;
    }
    if (!files.includes(point.path) || !baseRef) continue;
    // 收口点被本次变更大幅削减 → 视为失效（阈值与权威器一致：跌破基线 50%）
    let baseLines = 0;
    try {
      baseLines = git(["show", `${baseRef}:${point.path}`]).split(/\r?\n/).length;
    } catch {
      continue;
    }
    const nowLines = fs.readFileSync(abs, "utf-8").split(/\r?\n/).length;
    if (baseLines > 0 && nowLines < baseLines * 0.5) {
      violations++;
      console.error(
        `[FAIL] [B] 收口点 ${name} 文件被清空/削减：${point.path} ${baseLines} → ${nowLines} 行（< 基线 50%）`
      );
    }
  }
  if (violations > 0) process.exit(2);
}

// ---------------------------------------------------------------- main
const declAbs = path.join(repoRoot, DECL_PATH);
if (!fs.existsSync(declAbs)) {
  console.log(`[OK] 未发现 ${DECL_PATH}，跳过变更门禁`);
  process.exit(0);
}

const points = parseEnforcementPoints(fs.readFileSync(declAbs, "utf-8"));
const files = changedFiles();

if (Object.keys(points).length === 0) {
  console.warn("[WARN] 声明未配置 enforcement_points，门禁无规则可判（空转）");
  process.exit(0);
}

ruleD1(files);
ruleA(points, files);
ruleB(points, files);

console.log(
  `[OK] 可观测性变更门禁通过：${Object.keys(points).length} 个收口点，扫描 ${files.length} 个变更文件`
);
process.exit(0);
