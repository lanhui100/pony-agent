#!/usr/bin/env node
// B1795338250024（2026-11-22 本地安装 v0.1.203 实测）：`cp target/release/pony ~/.local/bin/pony`
// 在 TUI 正在运行时抛 `Text file busy`，但 npm 链式命令掩盖了失败，安装静默无效（exit 0）。
// 改为「同目录临时文件 + rename」原子替换：正在运行的进程继续持旧 inode，新进程拿到新二进制；
// 装完校验大小，失败一律非零退出，并提示需要重启才生效。
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "..");

function parseArgs(argv) {
  const opts = {
    from: path.join(repoRoot, "target/release"),
    to: path.join(os.homedir(), ".local/bin"),
    bins: []
  };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--from" && argv[i + 1]) opts.from = path.resolve(argv[++i]);
    else if (arg === "--to" && argv[i + 1]) opts.to = path.resolve(argv[++i]);
    else if (arg === "--help" || arg === "-h") opts.help = true;
    else if (arg.startsWith("-")) throw new Error(`未知参数: ${arg}`);
    else opts.bins.push(arg);
  }
  if (opts.bins.length === 0) opts.bins = ["pony", "ponycli"];
  return opts;
}

// 找出仍在运行旧二进制的进程（POSIX：/proc/<pid>/exe 指向被替换前的 inode 路径）。
function findRunningHolders(destPath) {
  const pids = [];
  if (process.platform !== "linux") return pids;
  let entries = [];
  try {
    entries = fs.readdirSync("/proc");
  } catch {
    return pids;
  }
  for (const entry of entries) {
    if (!/^\d+$/.test(entry)) continue;
    const pid = Number(entry);
    if (pid === process.pid) continue;
    try {
      if (fs.readlinkSync(`/proc/${entry}/exe`) === destPath) pids.push(pid);
    } catch {
      // 进程已退出或无权限读取，忽略
    }
  }
  return pids;
}

function installOne(srcPath, destPath) {
  if (!fs.existsSync(srcPath)) {
    throw new Error(`缺少构建产物 ${srcPath}，先跑 cargo build --release --bin ${path.basename(srcPath)}`);
  }
  fs.mkdirSync(path.dirname(destPath), { recursive: true });

  // 必须在 rename 之前采集：替换后旧 inode 指针变成 "<path> (deleted)"，进程仍在跑旧二进制。
  const holders = findRunningHolders(destPath);

  const tmpPath = `${destPath}.new-${process.pid}`;
  try {
    fs.copyFileSync(srcPath, tmpPath);
    fs.chmodSync(tmpPath, 0o755);
    fs.renameSync(tmpPath, destPath); // 同目录 rename：对运行中进程安全，且不会出现半截文件
  } catch (err) {
    try {
      fs.rmSync(tmpPath, { force: true });
    } catch {
      // 清理失败不掩盖原始错误
    }
    throw new Error(`原子替换 ${destPath} 失败：${err.message}`);
  }

  const srcSize = fs.statSync(srcPath).size;
  const destSize = fs.statSync(destPath).size;
  if (srcSize !== destSize) {
    throw new Error(`安装后校验失败：${destPath} 为 ${destSize} 字节，期望 ${srcSize} 字节`);
  }
  return holders;
}

/**
 * B261 后半：读取已安装二进制的自报版本。
 *
 * 两 bin（`pony` / `ponycli`）同包发布，自更新后可能出现「一个更新了、另一个还是旧的」的静默漂移；
 * 安装收尾处打印两者版本并在一不一致时告警，是把「漂移」从事后猜测变成安装时可见的判据。
 *
 * 宽容语义：旧版本 `ponycli` 不支持 `--version`（B261 前半才补上），探测失败只提示不失败——
 * 安装本身已经成功，不能因为探测不了版本就把安装判红。
 */
function probeVersion(binPath) {
  try {
    const out = execFileSync(binPath, ["--version"], {
      encoding: "utf8",
      timeout: 5000,
      stdio: ["ignore", "pipe", "ignore"]
    });
    const line = out.trim().split("\n")[0].trim();
    return extractVersionToken(line);
  } catch {
    return null;
  }
}

/**
 * B1795832157497：从 `--version` 首行提取 semver 形状的版本号。
 *
 * 早期实现取「行尾 token」，但 `pony --version` 的首行是
 * `pony 0.1.179 · linux-x86_64`——行尾是平台三元组（含数字，故被判为合法版本），
 * 于是 `pony` 被判成 `linux-x86_64`、`ponycli` 被判成 `0.1.179`，
 * 每次安装收尾都误报「两 bin 版本漂移」。
 *
 * 现口径：首行里第一个匹配 `\d+\.\d+\.\d+` 的 token 才是版本号；取不到即视为不支持。
 * 只认三段式 semver（与 `version:check` 的四处同步口径一致），避免把 `2026-11-28` 之类当版本。
 */
export function extractVersionToken(line) {
  if (typeof line !== "string") return null;
  const match = line.match(/\d+\.\d+\.\d+/);
  return match ? match[0] : null;
}

function reportInstalledVersions(bins, toDir) {
  const probed = bins.map((bin) => ({ bin, version: probeVersion(path.join(toDir, bin)) }));
  for (const { bin, version } of probed) {
    if (version) console.log(`[install-cli] ${bin} ${version}`);
    else console.warn(`[install-cli] ${bin}: 无法探测版本（该二进制不支持 --version）`);
  }
  const known = probed.filter((entry) => entry.version);
  const distinct = [...new Set(known.map((entry) => entry.version))];
  if (distinct.length > 1) {
    const detail = known.map((entry) => `${entry.bin} ${entry.version}`).join(" ≠ ");
    console.warn(`[install-cli] 警告：两 bin 版本漂移（${detail}）—— 请重跑 npm run cli:install 使两者同步。`);
  }
}

function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help) {
    console.log("用法: node scripts/install-cli.mjs [--from <dir>] [--to <dir>] [bin ...]");
    return;
  }

  const holders = [];
  for (const bin of opts.bins) {
    const pids = installOne(path.join(opts.from, bin), path.join(opts.to, bin));
    console.log(`[install-cli] ${path.join(opts.from, bin)} -> ${path.join(opts.to, bin)}`);
    if (pids.length > 0) holders.push({ bin, pids });
  }

  for (const { bin, pids } of holders) {
    console.warn(
      `[install-cli] ${bin}: 旧进程仍在运行（pid ${pids.join(", ")}），重启当前实例后新二进制才生效。`
    );
  }
  reportInstalledVersions(opts.bins, opts.to);
}

// 只有被直接执行时才跑 main；被 spec 导入（测 extractVersionToken）时保持纯函数。
const selfUrl = pathToFileURL(fs.realpathSync(fileURLToPath(import.meta.url))).href;
const invokedUrl = process.argv[1]
  ? pathToFileURL(fs.realpathSync(process.argv[1])).href
  : null;
if (invokedUrl === selfUrl) {
  try {
    main();
  } catch (err) {
    console.error(`[install-cli] 失败：${err.message}`);
    process.exit(1);
  }
}
