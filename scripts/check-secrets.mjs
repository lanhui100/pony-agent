#!/usr/bin/env node

/**
 * scripts/check-secrets.mjs
 * 
 * 跨平台敏感信息与脱敏自动化校验门禁 (Cross-platform Secret Leak Detection Gate)
 * 
 * 支持模式：
 * 1. 暂存区检查 (pre-commit): `node scripts/check-secrets.mjs --staged`
 * 2. 推送前检查 (pre-push):   `node scripts/check-secrets.mjs --pre-push [range]`
 * 3. 工作区或全库扫描 (scan): `node scripts/check-secrets.mjs --all`
 */

import { execSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";

// wave-18 C4（r3）：裸 64-hex 规则的模块级辅助（非导出）。
// R3：一切与 match 无关的上下文判定按「行」记忆化一次，消除 O(matches × 行长) 二次方。
const HEX64 = "[0-9a-fA-F]{64}";
const CRED_WORD_RE = /(client[_-]?token|access[_-]?token|api[_-]?key|secret|password|passwd|credential|authorization|bearer)/;
const BENIGN_RE = /(sha-?256|sha-?512|checksum|integrity|digest|computedhash|fingerprint|pristine_proof|\bhash\b|校验和|指纹)/;
const SUMPREFIX_RE = new RegExp(`^\\s*(${HEX64})\\s+\\S`);
const CELL_RE = new RegExp(`\\|\\s*(${HEX64})\\s*\\|`);
const BACKTICK_RE = new RegExp("`(" + HEX64 + ")`", "g");

// R3：按「行」记忆化全部与 match 无关的上下文判定；有界缓存（≤4 条）防无界增长。
const _hexFactsCache = new Map();
function hexLineFacts(line) {
  const hit = _hexFactsCache.get(line);
  if (hit) return hit;
  const lower = line.toLowerCase();
  const facts = {
    credWord: CRED_WORD_RE.test(lower),
    benignKeyword: BENIGN_RE.test(lower),
    prefixHex: (SUMPREFIX_RE.exec(line) || [null, null])[1]?.toLowerCase() ?? null,
    cellHex: (CELL_RE.exec(line) || [null, null])[1]?.toLowerCase() ?? null,
    backtickHexes: new Set([...line.matchAll(BACKTICK_RE)].map((x) => x[1].toLowerCase())),
  };
  if (_hexFactsCache.size >= 4) _hexFactsCache.clear();
  _hexFactsCache.set(line, facts);
  return facts;
}

// C7（r3）：`isSafe` 第二实参语义为「当前行 + 最多 3 行紧邻上文」的有界窗口。
// 规则本体按 r3 §4.2 冻结形态取窗口最后一行（`context.split("\n").pop()`）；
// 与 match 无关的整行判定已由 `hexLineFacts` 按行记忆化（R3 §4.6），
// 故每 match 仅剩一次 O(窗口) 的行提取，无二次方放大。

// 常见敏感前缀与凭证正则
export const SECRET_RULES = [
  {
    name: "OpenAI / Anthropic / Generic LLM API Token (sk-...)",
    regex: /\b(sk-[a-zA-Z0-9_-]{20,})\b/g,
    isSafe: (match) => {
      const lower = match.toLowerCase();
      return (
        lower.startsWith("sk-draft") ||
        lower.startsWith("sk-test") ||
        lower.startsWith("sk-mock") ||
        lower.startsWith("sk-dummy") ||
        lower.startsWith("sk-placeholder") ||
        lower.includes("example") ||
        lower.includes("fake") ||
        lower.startsWith("sk-xxxx") ||
        /^sk-[0-9a-z]{0,10}$/i.test(match)
      );
    }
  },
  {
    name: "GitHub Personal Access Token (ghp_...)",
    regex: /\b(ghp_[a-zA-Z0-9]{30,})\b/g,
    isSafe: (match) => {
      const lower = match.toLowerCase();
      return lower.includes("mock") || lower.includes("test") || lower.includes("dummy") || lower.includes("example");
    }
  },
  {
    name: "GitHub OAuth Access Token (gho_...)",
    regex: /\b(gho_[a-zA-Z0-9]{30,})\b/g,
    isSafe: (match) => {
      const lower = match.toLowerCase();
      return lower.includes("mock") || lower.includes("test") || lower.includes("dummy") || lower.includes("example");
    }
  },
  {
    name: "AWS Access Key ID (AKIA...)",
    regex: /\b(AKIA[0-9A-Z]{16})\b/g,
    isSafe: (match) => match.includes("EXAMPLE") || match.includes("TEST") || match.includes("MOCK")
  },
  {
    name: "GitLab Personal Access Token (glpat-...)",
    regex: /\b(glpat-[a-zA-Z0-9_-]{20,})\b/g,
    isSafe: (match) => {
      const lower = match.toLowerCase();
      return lower.includes("mock") || lower.includes("test") || lower.includes("dummy") || lower.includes("example");
    }
  },
  {
    name: "Slack Token (xox[baprs]-...)",
    regex: /\b(xox[baprs]-[0-9a-zA-Z]{10,48})\b/g,
    isSafe: (match) => {
      const lower = match.toLowerCase();
      return lower.includes("mock") || lower.includes("test") || lower.includes("dummy") || lower.includes("example");
    }
  },
  {
    name: "Private Key Block",
    regex: /-----BEGIN (?:[A-Z0-9_-]+ )?PRIVATE KEY-----/g,
    isSafe: (_match, context = "") => {
      const upper = context.toUpperCase();
      return upper.includes("MOCK") || upper.includes("TEST") || upper.includes("DUMMY") || upper.includes("EXAMPLE");
    }
  },
  // wave-18 C4（r3）：裸 64-hex 高熵凭证（SHA-256 尺寸 client token）
  // 契约基线：`.dev-team/contract-matrix-wave18.md` §4.2 / §4.3 / §4.6 / §4.7。
  // 长度窗口严格 64：用 `(?<![0-9a-fA-F])` / `(?![0-9a-fA-F])` 而非 `\b`，
  // 既能命中 `token_<64hex>`，又拒绝 63/65/128 位 hex 长串与 7/8/12/40 位 git SHA。
  {
    name: "Bare 64-Hex High-Entropy Credential (SHA-256 sized client token)",
    regex: /(?<![0-9a-fA-F])([0-9a-fA-F]{64})(?![0-9a-fA-F])/g,
    isSafe: (match, context = "") => {
      // C7：context 为「当前行 + 最多 3 行紧邻上文」的有界窗口；行级判定取最后一行。
      const line = context.split("\n").pop();
      const f = hexLineFacts(line); // R3：每行只算一次，之后 O(1)
      // R1：同行出现凭证语义词 => 一律判违规（优先于豁免 1/2/3/4）
      if (f.credWord) return false;
      if (f.benignKeyword) return true; // 豁免 1（已被 R1 守卫）
      const m = match.toLowerCase();
      if (f.prefixHex === m) return true;      // 豁免 2：只豁免 sha256sum 前缀位置的那个 hex（R2）
      if (f.cellHex === m) return true;        // 豁免 3：只豁免 Markdown 独立单元格内的那个 hex（R2）
      if (f.backtickHexes.has(m)) return true; // 豁免 4
      return false;
    }
  }
];

// 禁止提交的敏感文件名规则
export const BLOCKED_FILE_PATTERNS = [
  /^\.env(?:\.local|\.production|\.development|\.staging)?$/i,
  /\.pem$/i,
  /\.key$/i,
  /\.pfx$/i,
  /\.p12$/i,
  /^id_rsa(?:[._-].+)?$/i,
  /^id_ed25519(?:[._-].+)?$/i,
  /id_ecdsa(?:[._-].+)?$/i
];

export function isSafeFilename(filepath) {
  const base = path.basename(filepath);
  if (base.toLowerCase() === ".env.example" || base.toLowerCase() === ".env.template" || base.toLowerCase() === ".env.sample") {
    return true;
  }
  return false;
}

export function checkBlockedFilename(filepath) {
  const base = path.basename(filepath);
  if (isSafeFilename(filepath)) return null;

  for (const pattern of BLOCKED_FILE_PATTERNS) {
    if (pattern.test(base)) {
      return `禁止提交敏感凭证/私钥/环境变量文件: ${filepath}`;
    }
  }
  return null;
}

export function scanTextContent(content, filename = "") {
  // 门禁自身的测试用例文件包含检测样例字符串，豁免测试文件自身
  const normalizedFilename = filename.replace(/\\/g, "/");
  if (normalizedFilename.endsWith("check-secrets.spec.ts")) {
    return [];
  }

  const violations = [];
  const lines = content.split(/\r?\n/);

  for (let lineNum = 1; lineNum <= lines.length; lineNum++) {
    const line = lines[lineNum - 1];

    // C7（r3）：context 收窄为「当前行 + 最多 3 行紧邻上文」的有界窗口。
    // 禁止把整文件内容交给 isSafe（既有 Private Key Block 规则会因此 100% 失明）。
    // 窗口不含 BEGIN 行之后的内容：私钥 base64 体可能偶然含 "TEST"/"MOCK" 字样。
    const contextWindow = lines.slice(Math.max(0, lineNum - 4), lineNum).join("\n");

    for (const rule of SECRET_RULES) {
      rule.regex.lastIndex = 0;
      let match;
      while ((match = rule.regex.exec(line)) !== null) {
        const secretVal = match[1] || match[0];
        // C7：传入有界窗口（当前行 + 最多 3 行紧邻上文），禁止整文件内容。
        if (rule.isSafe && rule.isSafe(secretVal, contextWindow)) {
          continue;
        }

        const masked = secretVal.length > 8
          ? secretVal.substring(0, 4) + "*".repeat(secretVal.length - 8) + secretVal.substring(secretVal.length - 4)
          : "****";

        violations.push({
          rule: rule.name,
          line: lineNum,
          filename,
          masked,
          preview: line.trim().substring(0, 100)
        });
      }
    }
  }

  return violations;
}

function getStagedFiles() {
  try {
    const out = execSync("git diff --cached --name-only --diff-filter=ACMR", { encoding: "utf8" });
    return out.split("\n").map(s => s.trim()).filter(Boolean);
  } catch {
    return [];
  }
}

function getStagedDiff(filepath) {
  try {
    return execSync(`git diff --cached -U0 -- "${filepath}"`, { encoding: "utf8" });
  } catch {
    return "";
  }
}

function getAddedLinesFromDiff(diffText) {
  const lines = diffText.split(/\r?\n/);
  const added = [];
  for (const line of lines) {
    if (line.startsWith("+") && !line.startsWith("+++")) {
      added.push(line.substring(1));
    }
  }
  return added.join("\n");
}

export function runChecks({ mode = "staged", targetRange = null, files = null } = {}) {
  const violations = [];

  if (mode === "staged") {
    const stagedFiles = files || getStagedFiles();
    for (const file of stagedFiles) {
      const fileErr = checkBlockedFilename(file);
      if (fileErr) {
        violations.push({ filename: file, rule: "Blocked Sensitive File", preview: fileErr });
        continue;
      }

      const diffText = getStagedDiff(file);
      const addedContent = getAddedLinesFromDiff(diffText);
      const findings = scanTextContent(addedContent, file);
      violations.push(...findings);
    }
  } else if (mode === "pre-push") {
    const range = targetRange || "HEAD";
    let changedFiles = [];
    try {
      const out = execSync(`git diff --name-only --diff-filter=ACMR ${range}`, { encoding: "utf8" });
      changedFiles = out.split("\n").map(s => s.trim()).filter(Boolean);
    } catch {
      changedFiles = [];
    }

    for (const file of changedFiles) {
      const fileErr = checkBlockedFilename(file);
      if (fileErr) {
        violations.push({ filename: file, rule: "Blocked Sensitive File", preview: fileErr });
        continue;
      }

      try {
        const diffText = execSync(`git diff -U0 ${range} -- "${file}"`, { encoding: "utf8" });
        const addedContent = getAddedLinesFromDiff(diffText);
        const findings = scanTextContent(addedContent, file);
        violations.push(...findings);
      } catch {}
    }
  } else if (mode === "all") {
    let trackedFiles = [];
    try {
      const out = execSync("git ls-files", { encoding: "utf8" });
      trackedFiles = out.split("\n").map(s => s.trim()).filter(Boolean);
    } catch {
      trackedFiles = [];
    }

    for (const file of trackedFiles) {
      const fileErr = checkBlockedFilename(file);
      if (fileErr) {
        violations.push({ filename: file, rule: "Blocked Sensitive File", preview: fileErr });
        continue;
      }

      // 仅检查小于 1MB 且存在的文件，避免扫描超大二进制或资源文件
      try {
        if (!fs.existsSync(file)) continue;
        const stat = fs.statSync(file);
        if (stat.size > 1024 * 1024) continue;
        const ext = path.extname(file).toLowerCase();
        const binaryExts = [".png", ".jpg", ".jpeg", ".ico", ".gif", ".webp", ".zip", ".tar", ".gz", ".exe", ".dll", ".dylib", ".so", ".sqlite", ".db"];
        if (binaryExts.includes(ext)) continue;

        const content = fs.readFileSync(file, "utf8");
        const findings = scanTextContent(content, file);
        violations.push(...findings);
      } catch {}
    }
  }

  return violations;
}

// 仅在直接执行时调用
const isDirectExecution = process.argv[1] &&
  path.resolve(process.argv[1]) === path.resolve(decodeURI(new URL(import.meta.url).pathname));

if (isDirectExecution) {
  const args = process.argv.slice(2);
  let mode = "staged";
  let targetRange = null;

  if (args.includes("--pre-push")) {
    mode = "pre-push";
    const idx = args.indexOf("--pre-push");
    if (args.length > idx + 1 && !args[idx + 1].startsWith("--")) {
      targetRange = args[idx + 1];
    } else {
      try {
        const upstream = execSync("git rev-parse --abbrev-ref @{upstream}", { encoding: "utf8", stdio: ["pipe", "pipe", "ignore"] }).trim();
        targetRange = `${upstream}..HEAD`;
      } catch {
        targetRange = "HEAD";
      }
    }
  } else if (args.includes("--all")) {
    mode = "all";
  } else if (args.includes("--staged")) {
    mode = "staged";
  }

  const violations = runChecks({ mode, targetRange });

  if (violations.length > 0) {
    console.error("\n\x1b[31m[pony-agent] 拦截到未脱敏敏感信息或凭证文件！\x1b[0m");
    console.error("\x1b[33m违规明细：\x1b[0m");
    for (const v of violations) {
      if (v.line) {
        console.error(`  - \x1b[35m${v.filename}:${v.line}\x1b[0m [${v.rule}]`);
        console.error(`    脱敏掩码: ${v.masked}`);
        console.error(`    位置预览: ${v.preview}`);
      } else {
        console.error(`  - \x1b[35m${v.filename}\x1b[0m: ${v.preview}`);
      }
    }
    console.error("\n\x1b[36m处置建议：\x1b[0m");
    console.error("  1. 移除真实 Token 或私钥，使用 mock/dummy/placeholder 数据。");
    console.error("  2. 若为本地环境变量，请使用 .env.example 或将其移入 .gitignore。");
    console.error("  3. 若确需提交测试占位符，请确保前缀含有 'mock', 'test', 'draft' 或 'dummy'。\n");
    process.exit(1);
  }

  console.log("\x1b[32m[pony-agent] 敏感信息门禁检查通过 (0 leaks detected).\x1b[0m");
  process.exit(0);
}
