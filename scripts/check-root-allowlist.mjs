#!/usr/bin/env node
/**
 * scripts/check-root-allowlist.mjs —— 根目录跟踪文件白名单门。
 *
 * 为什么需要（B1795831359338，ADR 2026-11-28-false-closure-adr-and-repo-hygiene-policy 第 4 条）：
 * 根目录曾被 git 跟踪 12 个一次性报告 JSON、4 篇散落调研 md、`playground/*.exe|*.pdb`、
 * 一份基线文件的 `.bak` 备份，以及一个与 `AGENTS.md` 正面冲突的单数形式记忆文件。
 * 这些东西不影响任何测试（所以「全绿」发现不了），但每个新会话都会把它们当事实读进去 ——
 * 事故成本是**上下文幻觉**，不是构建失败。故用白名单把「根目录能放什么」变成可判定事实：
 * 新增根级文件必须先在本脚本登记（登记即一次显式决定）。
 *
 * 判定口径：`git ls-files` 顶层条目（不含目录）逐个比对白名单；不在白名单 → exit 1。
 * 退出码：0 通过；1 有未登记根级文件；2 读取失败（如 git 不可用）。
 */
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/**
 * 根目录允许的跟踪文件（每条都写清为什么必须在根）。
 * 新增条目 = 一次显式治理决定，不是「先放这儿回头再收」。
 */
const ALLOWED = new Set([
  // 构建/工具链配置：工具按约定只从根读，移走即失效
  "Cargo.toml",
  "Cargo.lock",
  "rust-toolchain.toml",
  "package.json",
  "package-lock.json",
  "tsconfig.json",
  "vite.config.ts",
  "vitest.config.ts",
  "playwright.config.ts",
  "index.html",
  "components.json",
  "skills-lock.json",
  ".version.json",
  ".env.example",
  ".gitattributes",
  ".gitignore",
  ".gitmodules",
  // 常载入口与人读契约：agent 与人都从根开始读
  "AGENTS.md",
  "CLAUDE.md",
  "README.md",
  // 子模块挂载点（git 只在根登记 submodule）
  "codex-openai",
]);

function main() {
  const listed = spawnSync("git", ["ls-files"], { cwd: REPO_ROOT, encoding: "utf8" });
  if (listed.status !== 0) {
    console.error(`[root-allowlist] git ls-files 失败：${listed.stderr}`);
    process.exit(2);
  }
  const rootFiles = listed.stdout
    .split("\n")
    .filter((line) => line && !line.includes("/"))
    .sort();
  const unexpected = rootFiles.filter((file) => !ALLOWED.has(file));
  if (unexpected.length > 0) {
    console.error(`[root-allowlist] FAIL 根目录有 ${unexpected.length} 个未登记的被跟踪文件：`);
    for (const file of unexpected) console.error(`  ${file}`);
    console.error("  处置：①移到 docs/ 下的正确分类；②或确认必须留在根，登记到本脚本 ALLOWED 并说明理由。");
    console.error("  不要「先放根目录，回头再收」—— 下一个会话会把它们当事实读进去。");
    process.exit(1);
  }
  console.log(`[root-allowlist] PASS（根级跟踪文件 ${rootFiles.length} 个，全部在白名单内）`);
}

main();
