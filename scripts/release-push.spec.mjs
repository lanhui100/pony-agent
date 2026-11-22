#!/usr/bin/env node
// 负样本自检：release.mjs --push 必须是「只读校验 + 推已存在 tag」，任何失败都不能在本地留 commit/tag。
// 缺陷源：B1795338249961（2026-11-22 实测 --push 先 bump，校验失败后需手工 reset + tag -d）。
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const scriptPath = path.join(__dirname, "release.mjs");
const TAG = "v9.9.9";

let failures = 0;
function check(name, cond, detail = "") {
  if (cond) {
    console.log(`  ✓ ${name}`);
  } else {
    failures++;
    console.error(`  ✗ ${name}${detail ? `\n      ${detail.replace(/\n/g, "\n      ")}` : ""}`);
  }
}

const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "pony-release-push-spec-")));
const home = path.join(root, "home");
const origin = path.join(root, "origin.git");
const work = path.join(root, "work");
fs.mkdirSync(home, { recursive: true });

const env = {
  ...process.env,
  HOME: home,
  XDG_CONFIG_HOME: path.join(home, ".config"),
  GIT_CONFIG_NOSYSTEM: "1",
  GIT_AUTHOR_NAME: "spec",
  GIT_AUTHOR_EMAIL: "spec@example.com",
  GIT_COMMITTER_NAME: "spec",
  GIT_COMMITTER_EMAIL: "spec@example.com",
  GIT_TERMINAL_PROMPT: "0",
  PONY_RELEASE_REPO_ROOT: work
};

function sh(cmd, cwd = root) {
  const r = spawnSync("bash", ["-c", cmd], { cwd, encoding: "utf8", env });
  return { code: r.status, out: `${r.stdout || ""}${r.stderr || ""}` };
}

function must(cmd, cwd = root) {
  const r = sh(cmd, cwd);
  if (r.code !== 0) throw new Error(`命令失败(${r.code}): ${cmd}\n${r.out}`);
  return r.out.trim();
}

function snapshot() {
  return {
    head: must("git rev-parse HEAD", work),
    tags: must("git tag --list", work),
    status: must("git status --porcelain", work),
    remoteTags: must("git ls-remote --tags origin", work)
  };
}

function runPush() {
  return sh(`node "${scriptPath}" --push`, work);
}

try {
  must(`git init --bare -b main "${origin}"`);
  must(`git clone "${origin}" "${work}"`);
  fs.writeFileSync(
    path.join(work, ".version.json"),
    `${JSON.stringify({ core: { major: 0, minor: 1, patch: 1 }, tauri: { major: 9, minor: 9, patch: 9 }, history: [], lastBump: new Date().toISOString() }, null, 2)}\n`
  );
  fs.writeFileSync(path.join(work, "README.md"), "sandbox\n");
  must("git add -A && git commit -m init", work);
  must("git push -u origin main", work);

  // P1：本地无 tag => 明确拒绝，且不创建任何 commit/tag
  const before1 = snapshot();
  const r1 = runPush();
  const after1 = snapshot();
  check("P1 tag 缺失时 exit 1", r1.code === 1, `exit=${r1.code}\n${r1.out}`);
  check("P1 报错指明 --push 只推已存在 tag", r1.out.includes(`不存在 tag ${TAG}`), r1.out);
  check("P1 不产生 commit", after1.head === before1.head, `${before1.head} -> ${after1.head}`);
  check("P1 不产生 tag", after1.tags === before1.tags, `tags: [${after1.tags}]`);
  check("P1 不污染工作区", after1.status === before1.status, `status: [${after1.status}]`);

  // P2：tag 指向尚未进入 origin/main 的提交 => 拒绝，且远端无该 tag
  must("git commit --allow-empty -m local-only", work);
  const localOnly = must("git rev-parse HEAD", work);
  must(`git tag ${TAG}`, work);
  const before2 = snapshot();
  const r2 = runPush();
  const after2 = snapshot();
  check("P2 未合入 main 时 exit 1", r2.code === 1, `exit=${r2.code}\n${r2.out}`);
  check("P2 报错指明未进入 origin/main", r2.out.includes("尚未进入 origin/main"), r2.out);
  check("P2 提示无需清理", r2.out.includes("无需清理"), r2.out);
  check("P2 HEAD 不变", after2.head === before2.head && after2.head === localOnly, `${before2.head} -> ${after2.head}`);
  check("P2 未新增 tag", after2.tags === before2.tags, `tags: [${after2.tags}]`);
  check("P2 远端仍无 tag", after2.remoteTags === "", `remote: [${after2.remoteTags}]`);

  // 回到主干态，准备正样本
  must(`git tag -d ${TAG} && git reset --hard origin/main`, work);

  // P3：tag 落在 origin/main 历史上 => 只推 tag，exit 0
  must(`git tag ${TAG}`, work);
  const before3 = snapshot();
  const r3 = runPush();
  const after3 = snapshot();
  check("P3 合入 main 后 exit 0", r3.code === 0, `exit=${r3.code}\n${r3.out}`);
  check("P3 远端出现该 tag", after3.remoteTags.includes(`refs/tags/${TAG}`), `remote: [${after3.remoteTags}]`);
  check("P3 只推 tag：不推分支/不改 HEAD", after3.head === before3.head, `${before3.head} -> ${after3.head}`);
  check("P3 不产生 commit", after3.head === before3.head && after3.status === before3.status);

  // P4：幂等重跑 => exit 0 且不重复推送报错
  const r4 = runPush();
  check("P4 重跑 exit 0", r4.code === 0, `exit=${r4.code}\n${r4.out}`);
  check("P4 提示幂等跳过", r4.out.includes("幂等跳过"), r4.out);

  // P5：远端已存在同名 tag 但指向不同提交 => 拒绝覆盖
  must("git commit --allow-empty -m second && git push origin main", work);
  must(`git tag -f ${TAG}`, work);
  const r5 = runPush();
  check("P5 远端指向不一致时 exit 1", r5.code === 1, `exit=${r5.code}\n${r5.out}`);
  check("P5 报错指明远端已存在且指向不同", r5.out.includes(`远端 tag ${TAG} 已存在`) && r5.out.includes("本地指向"), r5.out);
  check(
    "P5 不覆盖远端 tag",
    must(`git ls-remote origin refs/tags/${TAG}`, work).split(/\s+/)[0] !== must(`git rev-parse ${TAG}`, work)
  );
} catch (err) {
  failures++;
  console.error(`  ✗ 自检异常终止：${err.message}`);
} finally {
  try {
    fs.rmSync(root, { recursive: true, force: true });
  } catch {
    // 清理失败不影响结论
  }
}

if (failures > 0) {
  console.error(`\nrelease-push 自检失败：${failures} 项`);
  process.exit(1);
}
console.log("\nrelease-push 自检通过。");
