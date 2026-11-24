#!/usr/bin/env node
// 负样本自检：install-cli.mjs 必须原子替换、失败非零退出，且在目标被运行中进程占用时仍能装成功并提示重启。
// 缺陷源：B1795338250024（2026-11-22 实测 cp 撞 Text file busy，安装静默失败但 exit 0）。
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawn, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const scriptPath = path.join(__dirname, "install-cli.mjs");

let failures = 0;
function check(name, cond, detail = "") {
  if (cond) {
    console.log(`  ✓ ${name}`);
  } else {
    failures++;
    console.error(`  ✗ ${name}${detail ? `\n      ${detail.replace(/\n/g, "\n      ")}` : ""}`);
  }
}

function runInstall(from, to, bins) {
  const args = [scriptPath, "--from", from, "--to", to, ...bins];
  const r = spawnSync(process.execPath, args, { encoding: "utf8" });
  return { code: r.status, out: `${r.stdout || ""}${r.stderr || ""}` };
}

const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "pony-install-cli-spec-")));
const from = path.join(root, "from");
const to = path.join(root, "bin");
fs.mkdirSync(from, { recursive: true });

const running = [];

try {
  // P1：首次安装
  fs.writeFileSync(path.join(from, "pony"), "v1-binary");
  fs.writeFileSync(path.join(from, "ponycli"), "v1-cli");
  const r1 = runInstall(from, to, ["pony", "ponycli"]);
  check("P1 首次安装 exit 0", r1.code === 0, `exit=${r1.code}\n${r1.out}`);
  check("P1 内容一致", fs.readFileSync(path.join(to, "pony"), "utf8") === "v1-binary");
  check("P1 可执行位", (fs.statSync(path.join(to, "pony")).mode & 0o777) === 0o755);
  check(
    "P1 无临时文件残留",
    fs.readdirSync(to).every((name) => !name.includes(".new-")),
    `dir: [${fs.readdirSync(to).join(", ")}]`
  );

  // P2：覆盖更新
  fs.writeFileSync(path.join(from, "pony"), "v2-binary");
  const r2 = runInstall(from, to, ["pony"]);
  check("P2 覆盖 exit 0", r2.code === 0, `exit=${r2.code}\n${r2.out}`);
  check("P2 内容已更新", fs.readFileSync(path.join(to, "pony"), "utf8") === "v2-binary");

  // P3：缺构建产物 => 必须非零退出（缺陷原状是静默成功）
  const r3 = runInstall(from, to, ["missing-bin"]);
  check("P3 缺产物 exit 1", r3.code === 1, `exit=${r3.code}\n${r3.out}`);
  check("P3 报错指明缺少产物", r3.out.includes("缺少构建产物"), r3.out);

  // P4：目标被运行中进程占用 => 仍需装成功并提示重启（真正的 Text file busy 场景）
  if (process.platform === "linux" && fs.existsSync("/bin/sleep")) {
    const dest = path.join(to, "sleep-holder");
    fs.copyFileSync("/bin/sleep", dest);
    fs.chmodSync(dest, 0o755);
    const child = spawn(dest, ["30"], { stdio: "ignore" });
    running.push(child);
    // 等待子进程完成 exec，确保 /proc/<pid>/exe 已指向 dest（消除固定 200ms 在高负载 runner 上的竞态）
    const deadline = Date.now() + 2000;
    let ready = false;
    while (Date.now() < deadline && child.exitCode === null) {
      try {
        if (fs.readlinkSync(`/proc/${child.pid}/exe`) === dest) {
          ready = true;
          break;
        }
      } catch {
        // 尚在 fork/exec 过渡或暂时不可读，继续轮询
      }
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
    check(
      "P4 占位进程已就绪且 /proc/pid/exe 指向目标",
      ready,
      `pid=${child.pid} exitCode=${child.exitCode} dest=${dest}`
    );

    fs.writeFileSync(path.join(from, "sleep-holder"), "replacement");
    const r4 = runInstall(from, to, ["sleep-holder"]);
    check("P4 目标被占用时 exit 0", r4.code === 0, `exit=${r4.code}\n${r4.out}`);
    check("P4 二进制已替换", fs.readFileSync(dest, "utf8") === "replacement");
    check("P4 提示需重启", r4.out.includes("重启"), r4.out);
    check("P4 提示含占用进程 pid", r4.out.includes(String(child.pid)), `pid=${child.pid}\n${r4.out}`);

    child.kill("SIGKILL");
  } else {
    console.log("  - P4 跳过（非 linux 或无 /bin/sleep）");
  }
} catch (err) {
  failures++;
  console.error(`  ✗ 自检异常终止：${err.message}`);
} finally {
  for (const child of running) {
    try {
      child.kill("SIGKILL");
    } catch {
      // 已退出
    }
  }
  try {
    fs.rmSync(root, { recursive: true, force: true });
  } catch {
    // 清理失败不影响结论
  }
}

if (failures > 0) {
  console.error(`\ninstall-cli 自检失败：${failures} 项`);
  process.exit(1);
}
console.log("\ninstall-cli 自检通过。");
