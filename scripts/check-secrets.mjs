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

// wave-18 C4（r4）：裸 64-hex 规则的模块级辅助（非导出）。
// R3′：一切与 match 无关的上下文判定按「行」记忆化一次（键为 lineNumber），
// 行提取亦按行记忆化，消除 O(matches × 行长) 二次方。
const HEX64 = "[0-9a-fA-F]{64}";
// C4-R4（r4）：覆盖 client_token / client-token / client token / clienttoken / ClientToken
// 与 access token / api key 等空格、连字符、下划线、驼峰形态；含裸 `token`（词尾边界防 tokenizer）。
const CRED_WORD_RE = /(?:(?:client|access|refresh|id|auth)[\s_-]*)?token\b|api[\s_-]*key\b|secret|password|passwd|credential|authorization|bearer/i;
const BENIGN_RE = /(sha-?256|sha-?512|checksum|integrity|digest|computedhash|fingerprint|pristine_proof|\bhash\b|校验和|指纹)/i;
const SUMPREFIX_RE = new RegExp(`^\\s*(${HEX64})\\s+\\S`);
const CELL_RE = new RegExp(`\\|\\s*(${HEX64})\\s*\\|`);
const BACKTICK_RE = new RegExp("`(" + HEX64 + ")`", "g");

const firstIdx = (re, line) => {
  const m = re.exec(line);
  return m ? m.index : Infinity; // 无命中 => +∞（恒不小于任何 matchIndex）
};
const slotStart = (re, line) => {
  const m = re.exec(line);
  if (!m) return -1;
  return m.index + m[0].indexOf(m[1]); // 该「位置槽」在行内的起始下标
};

// R3：按「行」记忆化全部与 match 无关的上下文判定；有界缓存（≤4 条）防无界增长。
function hexLineFacts(line) {
  return {
    credIdx: firstIdx(CRED_WORD_RE, line),     // 首个凭证语义词下标
    benignIdx: firstIdx(BENIGN_RE, line),      // 首个良性摘要关键词下标
    benignAny: BENIGN_RE.test(line),           // 行内是否含良性摘要关键词
    prefixSlot: slotStart(SUMPREFIX_RE, line), // sha256sum 前缀槽起始下标（-1 = 无）
    cellSlot: slotStart(CELL_RE, line),        // Markdown 单元格槽起始下标（-1 = 无）
    backtickHexes: new Set([...line.matchAll(BACKTICK_RE)].map((x) => x[1].toLowerCase())),
  };
}

// C4-R3'：按 lineNumber 记忆化「当前行 + facts」；isSafe 内不得每 match 重做整行切分。
const _lineCache = new Map();
function ctxLineFacts(context, lineNumber) {
  const hit = _lineCache.get(lineNumber);
  if (hit && hit.context === context) return hit; // 同对象恒等比较 => O(1)
  const line = context.split("\n").pop();
  const entry = { context, line, facts: hexLineFacts(line) };
  if (_lineCache.size >= 4) _lineCache.clear();
  _lineCache.set(lineNumber, entry);
  return entry;
}

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
  // wave-18 C4（r4）：裸 64-hex 高熵凭证（SHA-256 尺寸 client token）
  // 契约基线：`.dev-team/contract-matrix-wave18.md` §4.2 / §4.3 / §4.6 / §4.7 / §4.8。
  // 长度窗口严格 64：用 `(?<![0-9a-fA-F])` / `(?![0-9a-fA-F])` 而非 `\b`，
  // 既能命中 `token_<64hex>`，又拒绝 63/65/128 位 hex 长串与 7/8/12/40 位 git SHA。
  {
    name: "Bare 64-Hex High-Entropy Credential (SHA-256 sized client token)",
    regex: /(?<![0-9a-fA-F])([0-9a-fA-F]{64})(?![0-9a-fA-F])/g,
    isSafe: (match, context = "", matchIndex = -1, lineNumber = -1) => {
      const { facts } = ctxLineFacts(context, lineNumber); // R3'：每行一次，之后 O(1)
      const m = match.toLowerCase();
      // C4-R1'（位置敏感）：仅当凭证语义词位于**该 hex 之前**、且其前**无**良性摘要关键词时判违规。
      // 目的：摘要收据形态（hex 在前 / 良性关键词在 hex 之前）不得因文件名、字段名或说明文字含
      // secret/token/credential 等词而误报（消除 r3 新误报回归）。
      if (facts.credIdx < matchIndex && !(facts.benignIdx < matchIndex)) return false;
      if (facts.benignAny) return true;                  // 豁免 1：行内良性摘要关键词
      // C4-R2'（位置锚定）：按**槽位下标**判定，不再按值判定 —— 同行其他位置的同值 hex 不继承豁免。
      if (matchIndex === facts.prefixSlot) return true;  // 豁免 2：sha256sum 前缀槽
      if (matchIndex === facts.cellSlot) return true;    // 豁免 3：Markdown 独立单元格槽
      if (facts.backtickHexes.has(m)) return true;       // 豁免 4：反引号独占包裹
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
        // r4：追加 match.index（位置敏感 R1′ / 位置锚定 R2′）与 lineNum（O(1) 行记忆化 R3′）。
        //     其余 6 条规则的 isSafe 忽略多余实参，签名向后兼容。
        if (rule.isSafe && rule.isSafe(secretVal, contextWindow, match.index, lineNum)) {
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
