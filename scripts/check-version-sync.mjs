import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");
const versionFile = path.join(repoRoot, ".version.json");
const coreCargo = path.join(repoRoot, "crates/pony-agent-core/Cargo.toml");
const tauriCargo = path.join(repoRoot, "src-tauri/Cargo.toml");
const packageJson = path.join(repoRoot, "package.json");
const tauriConf = path.join(repoRoot, "src-tauri/tauri.conf.json");

function extractCargoVersion(filePath) {
  if (!fs.existsSync(filePath)) {
    throw new Error(`File not found: ${filePath}`);
  }
  const content = fs.readFileSync(filePath, "utf8");
  const match = content.match(/^version\s*=\s*"(\d+\.\d+\.\d+)"/m);
  if (match) {
    return match[1];
  }
  throw new Error(`Could not extract version from ${filePath}`);
}

let hasError = false;

if (!fs.existsSync(versionFile)) {
  console.error("[check-version-sync] ERROR: .version.json not found.");
  process.exit(1);
}

const state = JSON.parse(fs.readFileSync(versionFile, "utf8"));
const stateCoreVer = `${state.core.major}.${state.core.minor}.${state.core.patch}`;
const stateTauriVer = `${state.tauri.major}.${state.tauri.minor}.${state.tauri.patch}`;

const coreCargoVer = extractCargoVersion(coreCargo);
if (coreCargoVer !== stateCoreVer) {
  console.error(`[check-version-sync] Core mismatch: crates/pony-agent-core/Cargo.toml (${coreCargoVer}) != .version.json (${stateCoreVer})`);
  hasError = true;
} else {
  console.log(`[check-version-sync] Core version ok: ${stateCoreVer}`);
}

const tauriCargoVer = extractCargoVersion(tauriCargo);
const pkg = JSON.parse(fs.readFileSync(packageJson, "utf8"));
const conf = JSON.parse(fs.readFileSync(tauriConf, "utf8"));

const tauriVersions = {
  ".version.json": stateTauriVer,
  "src-tauri/Cargo.toml": tauriCargoVer,
  "package.json": pkg.version,
  "src-tauri/tauri.conf.json": conf.version,
};

const uniqueTauri = new Set(Object.values(tauriVersions));
if (uniqueTauri.size !== 1) {
  console.error("[check-version-sync] Tauri version mismatch across files:");
  for (const [k, v] of Object.entries(tauriVersions)) {
    console.error(`  - ${k}: ${v}`);
  }
  hasError = true;
} else {
  console.log(`[check-version-sync] Tauri version ok (4 places in sync): ${stateTauriVer}`);
}

if (hasError) {
  console.error("[check-version-sync] FAILED: Repository version files are out of sync.");
  process.exit(1);
}

console.log("[check-version-sync] PASSED: All version files are synchronized.");
