#!/usr/bin/env node
// B1795338250024（2026-11-22 本地安装 v0.1.203 实测）：`cp target/release/pony ~/.local/bin/pony`
// 在 TUI 正在运行时抛 `Text file busy`，但 npm 链式命令掩盖了失败，安装静默无效（exit 0）。
// 改为「同目录临时文件 + rename」原子替换：正在运行的进程继续持旧 inode，新进程拿到新二进制；
// 装完校验大小，失败一律非零退出，并提示需要重启才生效。
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

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
}

try {
  main();
} catch (err) {
  console.error(`[install-cli] 失败：${err.message}`);
  process.exit(1);
}
