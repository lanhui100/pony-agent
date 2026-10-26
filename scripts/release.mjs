import fs from "node:fs";
import path from "node:path";
import { execSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");
const versionFile = path.join(repoRoot, ".version.json");
const coreCargo = path.join(repoRoot, "crates/pony-agent-core/Cargo.toml");
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
  updateCargoVersion(coreCargo, getVersionString(state.core.major, state.core.minor, state.core.patch));
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
  execSync("git add crates/pony-agent-core/Cargo.toml", { stdio: "inherit", cwd: repoRoot });
}

execSync(`git commit -m "chore(release): bump version to ${tauriVer}"`, { stdio: "inherit", cwd: repoRoot });
execSync(`git tag ${tagName}`, { stdio: "inherit", cwd: repoRoot });
console.log(`[bump-version] Created git commit and tag ${tagName}`);

if (pushAfter) {
  console.log(`[bump-version] Pushing main and tags to origin...`);
  execSync("git push origin main --tags", { stdio: "inherit", cwd: repoRoot });
  console.log(`[bump-version] Release push completed.`);
} else {
  console.log(`[bump-version] NOTE: Run 'git push origin main --tags' to trigger GitHub release build, or pass --push.`);
}
