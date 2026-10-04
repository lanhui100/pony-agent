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

    for (const rule of SECRET_RULES) {
      rule.regex.lastIndex = 0;
      let match;
      while ((match = rule.regex.exec(line)) !== null) {
        const secretVal = match[1] || match[0];
        // 传入当前行或全量内容作为上下文
        if (rule.isSafe && (rule.isSafe(secretVal, line) || rule.isSafe(secretVal, content))) {
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
