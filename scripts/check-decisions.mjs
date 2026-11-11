#!/usr/bin/env node
// 决策记录门禁（跨平台单一判据实现）。
// 判定真源即本文件；`npm run decisions:check` 与 pre-commit 都调它，
// `scripts/verify-decisions.ps1` 只做转发——一份判据两份实现必然漂移。
// 规范出处：docs/decisions/README.md（状态枚举 + 目录布局 + 索引 + 编号纪律）。
//
// 规则：
//   R1 Status 行必须存在（标题下前 5 行内）
//   R2 目录即状态：主目录 = implemented；proposed|superseded|rejected 子目录 = 同名状态
//   R3 `superseded by NNNN` 必须指向真实存在的 ADR 编号
//   R4 索引完整性：每篇 ADR 必须在其状态对应的索引分节里有一条链接
//   R5 索引无悬挂：索引区每条相对 .md 链接都必须指向真实文件
//   R6 编号唯一：同一 NNNN 不得对应两个文件（README：编号一经分配永不复用）
//   R7 索引不重复：同一篇 ADR 只能有一行主体行（行 = 第一个链接指向它）
//
// 退出码：0 = 通过；1 = 有违规；2 = 用法/IO 错误。
// `--fix-index` 只做机械回填（标题取自文件 H1，不代写摘要），用于一次性收口历史缺口。

import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const SECTIONS = { implemented: "活跃", proposed: "提案中", superseded: "已接替", rejected: "已拒绝" };
const SUBDIRS = ["proposed", "superseded", "rejected"];
const ADR_FILE = /^\d{4}-.+\.md$/;
const LINK = /\]\(([^)\s]+\.md)\)/g;

function parseArgs(argv) {
  const opts = { decisions: "docs/archive/decisions", fixIndex: false, json: false };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--fix-index") opts.fixIndex = true;
    else if (arg === "--json") opts.json = true;
    else if (arg === "--decisions") opts.decisions = argv[++i];
    else if (arg === "-h" || arg === "--help") {
      process.stdout.write(
        "用法：node scripts/check-decisions.mjs [--decisions <dir>] [--fix-index] [--json]\n",
      );
      process.exit(0);
    } else {
      process.stderr.write(`[check-decisions] 未知参数：${arg}\n`);
      process.exit(2);
    }
  }
  if (!opts.decisions) {
    process.stderr.write("[check-decisions] --decisions 需要非空目录\n");
    process.exit(2);
  }
  return opts;
}

function listAdrFiles(root) {
  const out = [];
  for (const entry of readdirSync(root)) {
    if (entry === "README.md" || entry === "AGENTS.md") continue;
    const abs = path.join(root, entry);
    if (statSync(abs).isDirectory()) {
      if (!SUBDIRS.includes(entry)) continue;
      for (const sub of readdirSync(abs)) {
        if (ADR_FILE.test(sub)) out.push(`${entry}/${sub}`);
      }
      continue;
    }
    if (ADR_FILE.test(entry)) out.push(entry);
  }
  return out.sort();
}

function readAdr(root, rel) {
  const lines = readFileSync(path.join(root, rel), "utf-8").split(/\r?\n/);
  return {
    rel,
    id: path.basename(rel).slice(0, 4),
    lines,
    statusLine: lines.slice(0, 5).find((line) => line.startsWith("Status:")) ?? null,
    h1: lines.find((line) => line.startsWith("# ")) ?? "",
  };
}

// `# ADR 0042：标题` / `# 0025 标题` / `# ADR-0048: 标题` → 标题
function titleOf(adr) {
  const stripped = adr.h1.replace(/^#\s*/, "").replace(/^ADR[\s-]*/i, "");
  const withoutId = stripped.startsWith(adr.id) ? stripped.slice(adr.id.length) : stripped;
  return withoutId.replace(/^[:：\s]+/, "").trim();
}

// 索引区（`## 索引` 之下）的列表行 → { line, section, links }
function parseIndex(text) {
  const rows = [];
  let section = null;
  let inIndex = false;
  text.split(/\r?\n/).forEach((line, idx) => {
    const h2 = /^##\s+(.*)$/.exec(line);
    if (h2) {
      inIndex = h2[1].trim() === "索引";
      section = null;
      return;
    }
    const h3 = /^###\s+(.*)$/.exec(line);
    if (h3) {
      section = h3[1].trim();
      return;
    }
    if (!inIndex || !line.startsWith("- ")) return;
    const links = [...line.matchAll(LINK)].map((m) => m[1]);
    if (links.length) rows.push({ line: idx, section, links });
  });
  return rows;
}

function statusOf(adr) {
  if (!adr.statusLine) return null;
  const value = adr.statusLine.slice("Status:".length).trim();
  for (const key of Object.keys(SECTIONS)) if (value.startsWith(key)) return key;
  return value;
}

function dirStatusOf(rel) {
  const [first] = rel.split("/");
  return SUBDIRS.includes(first) ? first : "implemented";
}

function linkId(link) {
  return /^(\d{4})-/.exec(link.replace(/^\.\//, ""))?.[1] ?? null;
}

function collectViolations(adrs, rows, root) {
  const violations = [];
  // 索引行以「主体」计：每行第一个链接才是这条行登记的那篇 ADR。
  // 行尾的修订互链（如 0098 行里指向 0105 的链接）不算登记，否则删掉 0105 自己的行也查不出来。
  const subjects = new Map();
  for (const row of rows) {
    const key = row.links[0].replace(/^\.\//, "");
    if (!subjects.has(key)) subjects.set(key, []);
    subjects.get(key).push(row.section);
  }
  const knownIds = new Set();
  const byId = new Map();
  for (const adr of adrs) {
    byId.set(adr.id, [...(byId.get(adr.id) ?? []), adr.rel]);
    knownIds.add(adr.id);
  }
  for (const [id, rels] of [...byId.entries()].sort()) {
    if (rels.length > 1) violations.push(`R6 编号重复（NNNN 必须唯一）: ${id} -> ${rels.join("、")}`);
  }
  for (const [link, sections] of [...subjects.entries()].sort()) {
    if (sections.length > 1) {
      violations.push(`R7 索引重复行: ${link}（同一篇 ADR 只允许一行，实际 ${sections.length} 行）`);
    }
  }

  for (const adr of adrs) {
    const expected = dirStatusOf(adr.rel);
    const status = statusOf(adr);
    if (!status) {
      violations.push(`R1 缺少 Status 行（标题下五行内）: ${adr.rel}`);
      continue;
    }
    if (status !== expected) {
      violations.push(
        `R2 目录与状态不一致（目录即状态）: ${adr.rel} -> ${adr.statusLine}（该目录要求 ${expected}）`,
      );
    }
    const supersede = /superseded by\s*\[?(\d{4})\]?/i.exec(adr.statusLine);
    if (supersede && !knownIds.has(supersede[1])) {
      violations.push(`R3 superseded by ${supersede[1]} 指向不存在的 ADR: ${adr.rel}`);
    }
    const wantSection = SECTIONS[expected];
    const sections = subjects.get(adr.rel);
    if (!sections) {
      violations.push(`R4 索引缺行: ${adr.rel}（未登记进 README 索引）`);
    } else if (!sections.includes(wantSection)) {
      violations.push(
        `R4 索引分节错误: ${adr.rel} 应在「${wantSection}」，实际在「${[...new Set(sections)].join("、")}」`,
      );
    }
  }

  for (const row of rows) {
    for (const link of row.links) {
      if (/[a-z]+:\/\//i.test(link) || path.isAbsolute(link)) continue;
      if (!existsSync(path.join(root, link))) {
        violations.push(`R5 索引悬挂链接: ${link}（第 ${row.line + 1} 行）`);
      }
    }
  }
  return violations;
}

function insertRows(text, pending) {
  let out = text;
  for (const item of pending) {
    const section = SECTIONS[item.status];
    const row = `- [${item.id} ${item.title}](${item.rel})`;
    const body = out.split(/\r?\n/);
    const rows = parseIndex(out).filter((r) => r.section === section);
    if (rows.length === 0) {
      // 分节标题已在但没有行 → 落进该分节；只有标题也不存在时才新建分节。
      const heading = body.findIndex((l) => l.trim() === `### ${section}`);
      if (heading >= 0) {
        const at = body[heading + 1] === "" ? heading + 2 : heading + 1;
        const needsGap = body[at] !== undefined && !body[at].startsWith("- ");
        body.splice(at, 0, ...(needsGap ? [row, ""] : [row]));
        if (body[heading + 1] !== "") body.splice(heading + 1, 0, "");
      } else {
        const anchor = body.findIndex((l) => /^###\s+/.test(l));
        const block = [`### ${section}`, "", row];
        if (anchor < 0) body.push("## 索引", "", ...block);
        else body.splice(anchor, 0, ...block, "");
      }
    } else {
      // 排序键取行主体（每行第一个链接）；行尾的修订互链不参与，否则会把新行插错位置。
      const after = rows.find((r) => {
        const id = linkId(r.links[0]);
        return id !== null && id > item.id;
      });
      const at = after ? after.line : Math.max(...rows.map((r) => r.line)) + 1;
      body.splice(at, 0, row);
    }
    out = body.join("\n");
  }
  return out;
}

function loadAdrs(root) {
  return listAdrFiles(root).map((rel) => readAdr(root, rel));
}

/** 对单个决策目录做全量判定；返回 { checked, violations }。 */
export function checkDecisions(decisionsDir) {
  const root = path.resolve(decisionsDir);
  const readmePath = path.join(root, "README.md");
  if (!existsSync(root) || !statSync(root).isDirectory()) throw new Error(`未找到决策目录：${root}`);
  if (!existsSync(readmePath)) throw new Error(`未找到索引：${readmePath}`);
  const adrs = loadAdrs(root);
  return {
    checked: adrs.length,
    violations: collectViolations(adrs, parseIndex(readFileSync(readmePath, "utf-8")), root),
  };
}

/** 机械回填索引缺口；返回补写的编号列表。 */
export function fixIndex(decisionsDir) {
  const root = path.resolve(decisionsDir);
  const readmePath = path.join(root, "README.md");
  const adrs = loadAdrs(root);
  let text = readFileSync(readmePath, "utf-8");
  const pending = [];
  for (const adr of adrs) {
    const status = statusOf(adr) ?? dirStatusOf(adr.rel);
    if (!SECTIONS[status]) continue;
    const rows = parseIndex(text).filter((r) => r.section === SECTIONS[status]);
    if (!rows.some((r) => r.links.some((l) => l.replace(/^\.\//, "") === adr.rel))) {
      pending.push({ id: adr.id, rel: adr.rel, status, title: titleOf(adr) || adr.rel });
    }
  }
  if (pending.length) writeFileSync(readmePath, insertRows(text, pending), "utf-8");
  return pending.map((p) => p.id);
}

function main() {
  const opts = parseArgs(process.argv.slice(2));
  const root = path.resolve(opts.decisions);
  if (!existsSync(root) || !statSync(root).isDirectory()) {
    process.stderr.write(`[check-decisions] 未找到决策目录：${root}\n`);
    return 2;
  }
  if (!existsSync(path.join(root, "README.md"))) {
    process.stderr.write(`[check-decisions] 未找到索引：${path.join(root, "README.md")}\n`);
    return 2;
  }

  const adrs = loadAdrs(root);
  const readmePath = path.join(root, "README.md");
  let violations = checkDecisions(root).violations;

  if (opts.fixIndex) {
    const filled = fixIndex(root);
    process.stdout.write(
      `[check-decisions] 索引回填 ${filled.length} 行（${filled.join(", ") || "无"}）\n`,
    );
    violations = checkDecisions(root).violations;
  }
  const checked = adrs.length;

  if (opts.json) {
    process.stdout.write(`${JSON.stringify({ checked, violations }, null, 2)}\n`);
  } else if (violations.length) {
    process.stdout.write(`[check-decisions] 决策记录校验失败（${checked} 篇）：\n`);
    for (const item of violations) process.stdout.write(`  - ${item}\n`);
    process.stdout.write("[check-decisions] 规范见 docs/decisions/README.md\n");
  } else {
    process.stdout.write(`[check-decisions] 通过：${checked} 篇 ADR 状态/目录/索引一致\n`);
  }
  return violations.length ? 1 : 0;
}

// 供测试直接驱动判据；命令行入口只在被直接执行时生效。
export { collectViolations, dirStatusOf, insertRows, linkId, parseIndex, statusOf, titleOf };

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) process.exit(main());

