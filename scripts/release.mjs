import fs from "node:fs";
import path from "node:path";
import { execSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
// PONY_RELEASE_REPO_ROOT 仅作自检沙箱注入点（scripts/release-push.spec.mjs），生产调用不设置。
const repoRoot = process.env.PONY_RELEASE_REPO_ROOT
  ? path.resolve(process.env.PONY_RELEASE_REPO_ROOT)
  : path.resolve(__dirname, "..");
const versionFile = path.join(repoRoot, ".version.json");
const coreCargoFiles = [
  "crates/pony-agent-core/Cargo.toml",
  "crates/pony-tui/Cargo.toml",
  "crates/pony-cli/Cargo.toml",
  "crates/pony-protocol/Cargo.toml",
  "crates/pony-observability/Cargo.toml",
  "crates/pony-provider/Cargo.toml",
  "crates/pony-storage/Cargo.toml",
  "crates/pony-tools/Cargo.toml",
  "crates/pony-acp/Cargo.toml",
];
const tauriCargo = path.join(repoRoot, "src-tauri/Cargo.toml");
const packageJson = path.join(repoRoot, "package.json");
const tauriConf = path.join(repoRoot, "src-tauri/tauri.conf.json");

function getVersionString(major, minor, patch) {
  return `${major}.${minor}.${patch}`;
}

const args = process.argv.slice(2);
let target = "all";
let segment = "patch";
let pushAfter = false;

for (let i = 0; i < args.length; i++) {
  if (args[i] === "--target" && args[i + 1]) target = args[++i];
  if (args[i] === "--segment" && args[i + 1]) segment = args[++i];
  if (args[i] === "--push") pushAfter = true;
}

if (!fs.existsSync(versionFile)) {
  console.error(".version.json not found!");
  process.exit(1);
}

const state = JSON.parse(fs.readFileSync(versionFile, "utf8"));

function runGit(gitArgs, { allowFail = false } = {}) {
  try {
    const out = execSync(`git ${gitArgs}`, {
      cwd: repoRoot,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      env: { ...process.env, GIT_TERMINAL_PROMPT: "0" }
    });
    return out.trim();
  } catch (err) {
    if (allowFail) return null;
    throw err;
  }
}

function failPush(lines) {
  console.error("");
  for (const line of lines) console.error(line);
  console.error("");
  process.exit(1);
}

// B1795338249961（2026-11-22 v0.1.203 发版实测）：--push 过去先走完整 bump 再校验 HEAD 是否在 origin/main，
// 校验失败时本地已多出一个未推送的 bump commit 与 tag，只能手工 reset + tag -d 恢复。
// 现在 --push 是纯操作：只读校验 + 推送已存在的 tag，任何一步失败都不写工作区/不改 HEAD。
function pushTagOnly() {
  const tagName = `v${getVersionString(state.tauri.major, state.tauri.minor, state.tauri.patch)}`;
  console.log(`[release:push] 目标 tag: ${tagName}（取自 .version.json 的 tauri 版本）`);

  if (runGit("status --porcelain", { allowFail: true })) {
    console.warn("[release:push] WARN: 工作区有未提交改动；tag 只指向已提交内容，这些改动不会进入本次发布。");
  }

  const tagCommit = runGit(`rev-parse --verify --quiet ${tagName}^{commit}`, { allowFail: true });
  if (!tagCommit) {
    failPush([
      `[release:push] 拒绝执行：本地不存在 tag ${tagName}。`,
      "  --push 只推已存在的 tag，不会替代 bump（这样误触也不会在 main 上留下脏 commit/tag）。",
      "  完整发版流程：",
      "    1. git switch -c chore/release-v<version> origin/main",
      "    2. npm run release                              # 本地 bump + commit + tag",
      "    3. git push -u origin <branch> && gh pr create",
      "    4. 人工用 merge commit（不要 squash）合并，保证 tag 指向 main 历史",
      "    5. npm run release:push                         # 只推 tag，触发发布"
    ]);
  }

  runGit("fetch origin main --quiet", { allowFail: true });
  if (!runGit("rev-parse --verify --quiet origin/main", { allowFail: true })) {
    failPush([
      "[release:push] 拒绝执行：读不到 origin/main，无法校验 tag 是否已在主干历史上。",
      "  网络/权限修好后重跑；本地未做任何改动。",
      "  手工核对：git fetch origin main && git merge-base --is-ancestor " + tagName + " origin/main"
    ]);
  }
  if (runGit(`merge-base --is-ancestor ${tagCommit} origin/main`, { allowFail: true }) === null) {
    failPush([
      `[release:push] 拒绝推送 tag：${tagName} 指向的提交尚未进入 origin/main。`,
      "  main 受保护（禁止直推），版本 bump 必须经 PR 合并后再推 tag：",
      "    1. git push -u origin chore/release-v<version> && gh pr create",
      "    2. 人工用 merge commit（不要 squash）合并",
      "    3. npm run release:push",
      "  本地未做任何改动（无新 commit、无新 tag），无需清理。"
    ]);
  }

  const remoteLines = (runGit(`ls-remote --tags origin refs/tags/${tagName}`, { allowFail: true }) || "")
    .split("\n")
    .filter(Boolean);
  if (remoteLines.length > 0) {
    const remoteSha = (remoteLines.find((l) => l.endsWith("^{}")) || remoteLines[0]).split(/\s+/)[0];
    if (remoteSha === tagCommit) {
      console.log(`[release:push] tag ${tagName} 已在远端且指向 ${tagCommit.slice(0, 7)}，幂等跳过（本地无改动）。`);
      return;
    }
    failPush([
      `[release:push] 拒绝推送：远端 tag ${tagName} 已存在，但指向 ${remoteSha.slice(0, 7)}；本地指向 ${tagCommit.slice(0, 7)}。`,
      "  同名 tag 重指会覆盖发布历史，需人工确认，脚本不代劳：",
      `    确认本地为准：git push --force origin refs/tags/${tagName}   # 慎用`,
      `    确认远端为准：git tag -d ${tagName} && git fetch origin tag ${tagName}`
    ]);
  }

  console.log(`[release:push] 推送 tag ${tagName} -> origin（只推 tag，不推分支）`);
  try {
    execSync(`git push origin refs/tags/${tagName}`, { stdio: "inherit", cwd: repoRoot, env: { ...process.env, GIT_TERMINAL_PROMPT: "0" } });
  } catch {
    failPush([
      `[release:push] git push 失败：tag ${tagName} 未推送。`,
      "  本地未做任何改动，排除网络/权限问题后重跑本命令即可。"
    ]);
  }
  console.log(`[release:push] tag ${tagName} 已推送，release workflow 将从该 tag 构建。`);
}

if (pushAfter) {
  pushTagOnly();
  process.exit(0);
}

const now = new Date().toISOString();
const bumpTargets = target === "all" ? ["core", "tauri"] : [target];
const bumpLog = [];

for (const comp of bumpTargets) {
  const ver = state[comp];
  const from = getVersionString(ver.major, ver.minor, ver.patch);
  if (segment === "major") {
    ver.major++;
    ver.minor = 0;
    ver.patch = 0;
  } else if (segment === "minor") {
    ver.minor++;
    ver.patch = 0;
  } else {
    ver.patch++;
  }
  const to = getVersionString(ver.major, ver.minor, ver.patch);
  bumpLog.push({ component: comp, from, to, segment });
}

state.lastBump = now;
state.history.push({
  commit: "",
  timestamp: now,
  bumps: bumpLog.map(b => ({
    from: b.from,
    to: b.to,
    component: b.component,
    segment: b.segment
  }))
});
fs.writeFileSync(versionFile, JSON.stringify(state, null, 2) + "\n");

function updateCargoVersion(filePath, verStr) {
  if (!fs.existsSync(filePath)) return;
  let content = fs.readFileSync(filePath, "utf8");
  content = content.replace(/^version\s*=\s*"[0-9.]+"/m, `version = "${verStr}"`);
  fs.writeFileSync(filePath, content);
}

if (bumpTargets.includes("core")) {
  const coreVerStr = getVersionString(state.core.major, state.core.minor, state.core.patch);
  for (const rel of coreCargoFiles) {
    updateCargoVersion(path.join(repoRoot, rel), coreVerStr);
  }
}

if (bumpTargets.includes("tauri")) {
  const tauriVerStr = getVersionString(state.tauri.major, state.tauri.minor, state.tauri.patch);
  updateCargoVersion(tauriCargo, tauriVerStr);

  const pkg = JSON.parse(fs.readFileSync(packageJson, "utf8"));
  pkg.version = tauriVerStr;
  fs.writeFileSync(packageJson, JSON.stringify(pkg, null, 2) + "\n");

  const conf = JSON.parse(fs.readFileSync(tauriConf, "utf8"));
  conf.version = tauriVerStr;
  fs.writeFileSync(tauriConf, JSON.stringify(conf, null, 2) + "\n");
}

console.log("[bump-version] Version bumped successfully:");
for (const b of bumpLog) {
  console.log(`  - ${b.component}: ${b.from} -> ${b.to} (${b.segment})`);
}

// ADR 0011: 同步 Cargo.lock 中的 workspace 本地成员版本（Node 版替换 PowerShell 版时遗漏了这一步）。
// 历史教训：曾出现只改 Cargo.toml 而不同步根 Cargo.lock 的提交，导致 --locked 构建失败。
// `cargo metadata` 只刷新 workspace 本地成员的 lock 条目，不会升级第三方依赖；失败降级为警告不阻塞 bump。
try {
  execSync("cargo metadata --format-version 1", { stdio: "ignore", cwd: repoRoot });
  console.log("[bump-version] Synced Cargo.lock (workspace member versions)");
} catch (err) {
  console.warn(`[bump-version] WARN: cargo metadata failed: ${err.message} — Cargo.lock NOT synced.`);
}

// Check and verify version sync
execSync("node scripts/check-version-sync.mjs", { stdio: "inherit", cwd: repoRoot });

const tauriVer = getVersionString(state.tauri.major, state.tauri.minor, state.tauri.patch);
const tagName = `v${tauriVer}`;

// Commit and tag (ADR 0011: git add 包含 Cargo.lock)
execSync("git add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json .version.json Cargo.lock", { stdio: "inherit", cwd: repoRoot });
if (bumpTargets.includes("core")) {
  execSync(`git add ${coreCargoFiles.join(" ")}`, { stdio: "inherit", cwd: repoRoot });
}

execSync(`git commit -m "chore(release): bump version to ${tauriVer}"`, { stdio: "inherit", cwd: repoRoot });
execSync(`git tag ${tagName}`, { stdio: "inherit", cwd: repoRoot });
console.log(`[bump-version] Created git commit and tag ${tagName}`);

// ADR 2026-11-18-multi-session-git-contract：main 已受保护，bump 这一步不推送任何东西。
// 推 tag 只在 `npm run release:push`（pushTagOnly）里发生，且必须先经 PR 合并。
console.log(
  `[bump-version] NOTE: tag ${tagName} 已本地创建。main 受保护，请先把 bump 提交经 PR 合并，再 'npm run release:push' 只推 tag。`
);
