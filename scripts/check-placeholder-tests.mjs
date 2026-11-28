#!/usr/bin/env node
/**
 * scripts/check-placeholder-tests.mjs —— 恒真占位断言门（禁止 `assert!(true)` / `expect(true)` 式空测试）。
 *
 * 为什么需要（B1795831359205，ADR 2026-11-28-false-closure-adr-and-repo-hygiene-policy）：
 * `assert!(true)` 让一个测试文件「存在且绿」，但它断言的是编译期常量，任何回归都不会让它变红。
 * 事故形态是：工单被标 `done`、ADR 被标 `implemented`，而对应实现零行 —— 账本与现实的唯一
 * 连接点就是这个从不失败的用例。这类失效**不改变任何可见输出**，只能靠机械门钉住。
 *
 * 判定口径：剥掉注释后，逐个测试文件找**语句位置**的恒真断言：
 *   - Rust：`assert!(true)` / `assert_eq!(a, a)` 之外的 `assert!(true ...)` 形态（含 `assert!(true, "…")`）
 *   - TS/JS：`expect(true).toBe(true)` / `expect(1).toBe(1)` 等字面量自比较
 * 注释里提到这些写法（纪律说明）不算违规 —— 大量测试文件的头部注释正是这样写的。
 *
 * 豁免：`--baseline <file>` 里登记的行（`<相对路径>:<行号>`）放行；用于「已登记待裁决」的存量。
 *   基线与 `file-size-baseline.json` 同一条棘轮纪律：只许删条目，不许新增。
 *
 * 退出码：0 无违规；1 命中违规；2 用法/读取失败。
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SCAN_ROOTS = ["crates", "src-tauri", "tests", "src"];
const TS_EXT = new Set([".ts", ".tsx", ".js", ".mjs", ".vue"]);

function fail(message) {
  console.error(`[placeholder-tests] ${message}`);
  process.exit(2);
}

function walk(dir, out = []) {
  const abs = path.join(REPO_ROOT, dir);
  if (!fs.existsSync(abs)) return out;
  for (const entry of fs.readdirSync(abs, { withFileTypes: true })) {
    const rel = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (["node_modules", "target", "dist", "coverage", ".git"].includes(entry.name)) continue;
      walk(rel, out);
    } else if (entry.name.endsWith(".rs") || TS_EXT.has(path.extname(entry.name))) {
      out.push(rel);
    }
  }
  return out;
}

// 剥掉行注释、块注释（Rust 含嵌套）与字符串字面量内容，保留行号对齐。
function stripComments(source, isRust) {
  let out = "";
  let i = 0;
  const n = source.length;
  while (i < n) {
    const ch = source[i];
    const next = source[i + 1];
    if (ch === "/" && next === "/") {
      while (i < n && source[i] !== "\n") {
        out += " ";
        i += 1;
      }
      continue;
    }
    if (ch === "/" && next === "*") {
      let depth = 1;
      out += "  ";
      i += 2;
      while (i < n && depth > 0) {
        if (isRust && source[i] === "/" && source[i + 1] === "*") {
          depth += 1;
          out += "  ";
          i += 2;
          continue;
        }
        if (source[i] === "*" && source[i + 1] === "/") {
          depth -= 1;
          out += "  ";
          i += 2;
          continue;
        }
        out += source[i] === "\n" ? "\n" : " ";
        i += 1;
      }
      continue;
    }
    // 字符串/字符字面量：内容一律替换为空格（避免把 `let s = "assert!(true)"` 误判为违规）。
    // Rust 的单引号只在 `'x'`（字符字面量）形态下才是包裹符 —— `'a` 这类生命周期标注必须原样
    // 透传，否则会把其后整段代码当字符串吞掉，反而掩盖真实违规（漏判比误判更危险）。
    const isRustCharLiteral = isRust && ch === "'" && source[i + 2] === "'" && source[i + 1] !== "\\";
    const isStringStart = ch === '"' || ch === "`" || (!isRust && ch === "'") || isRustCharLiteral;
    if (isStringStart) {
      const quote = ch;
      out += ch;
      i += 1;
      while (i < n) {
        if (source[i] === "\\") {
          out += "  ";
          i += 2;
          continue;
        }
        if (source[i] === quote) {
          out += quote;
          i += 1;
          break;
        }
        out += source[i] === "\n" ? "\n" : " ";
        i += 1;
      }
      continue;
    }
    out += ch;
    i += 1;
  }
  return out;
}

const RUST_RULES = [
  { name: "assert!(true)", re: /\bassert!\s*\(\s*true\b/ },
  { name: "assert_eq!(x, x)", re: /\bassert_eq!\s*\(\s*(true|false|1|0)\s*,\s*\1\s*\)/ },
];
const TS_RULES = [
  { name: "expect(true).toBe(true)", re: /\bexpect\s*\(\s*(true|false|1|0)\s*\)\s*\.\s*to(Be|Equal)\s*\(\s*(true|false|1|0)\s*\)/ },
];

function loadBaseline(file) {
  if (!file) return new Set();
  const abs = path.join(REPO_ROOT, file);
  if (!fs.existsSync(abs)) fail(`基线不存在: ${file}`);
  const raw = fs.readFileSync(abs, "utf8");
  const parsed = JSON.parse(raw);
  if (!Array.isArray(parsed)) fail("基线必须是数组");
  return new Set(parsed);
}

function main() {
  const args = process.argv.slice(2);
  let baselineFile = null;
  for (let i = 0; i < args.length; i += 1) {
    if (args[i] === "--baseline") {
      baselineFile = args[i + 1];
      i += 1;
    } else if (args[i] === "--help" || args[i] === "-h") {
      console.log("用法: node scripts/check-placeholder-tests.mjs [--baseline <file>]");
      return;
    } else {
      fail(`未知参数: ${args[i]}`);
    }
  }
  const baseline = loadBaseline(baselineFile);
  const files = SCAN_ROOTS.flatMap((root) => walk(root));
  const violations = [];
  for (const rel of files) {
    const source = fs.readFileSync(path.join(REPO_ROOT, rel), "utf8");
    const isRust = rel.endsWith(".rs");
    const stripped = stripComments(source, isRust);
    const rules = isRust ? RUST_RULES : TS_RULES;
    stripped.split("\n").forEach((line, index) => {
      for (const rule of rules) {
        if (!rule.re.test(line)) continue;
        const key = `${rel}:${index + 1}`;
        if (baseline.has(key)) return;
        violations.push({ key, rule: rule.name, text: line.trim() });
      }
    });
  }
  if (violations.length > 0) {
    console.error(`[placeholder-tests] FAIL ${violations.length} 处恒真占位断言（断言编译期常量，任何回归都不会变红）：`);
    for (const v of violations) console.error(`  ${v.key}  ${v.rule}  ${v.text}`);
    console.error("  处置：写真实断言，或经裁决删除该用例；存量须显式登记到 --baseline。");
    process.exit(1);
  }
  console.log(`[placeholder-tests] PASS（扫描 ${files.length} 个文件，恒真占位断言 0；基线豁免 ${baseline.size} 条）`);
}

main();
