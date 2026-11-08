#!/usr/bin/env node
/**
 * 发布侧门禁：本次发布的 CLI（`pony-agent-core`）版本必须**严格大于**上一个已发布清单
 * `pony-cli-latest.json` 里声明的 `version`。
 *
 * 为什么需要它（现场遗留项，2026-11-08）：
 *   客户端 `pony update` 的发现口径是「取第一个含 `pony-cli-latest.json` 的 release，版本以清单
 *   `version` 为准」。清单里的 version 就是**该 tag 那次构建的 core 版本**。若某次发版只 bump
 *   `tauri` 而没 bump `core`（`node scripts/release.mjs --target tauri` / `bump-version.ps1 -Target
 *   tauri` 都允许），清单 version 与上一版相同 ⇒ 所有已是最新的机器都判定「已是最新」，新 CLI
 *   永远不会被下载 —— 这种失败在客户端侧无任何报错可看，只能靠发布侧 fail-closed 拦住。
 *
 * 用法：
 *   node scripts/check-cli-manifest-advances.mjs --current <ver> [--previous <ver>|none]
 *   无 `--previous`（或 `none`）= 仓库里还没有任何含清单的 release，直接放行并打印说明。
 *
 * 退出码：0 通过 / 1 未递增或版本非法 / 2 用法错误。
 */
import process from "node:process";

/** 解析 `X.Y.Z`（允许 `v`/`V` 前缀）；非法返回 `null`（绝不补零、绝不猜测）。 */
export function parseCliVersion(raw) {
  if (typeof raw !== "string") return null;
  const match = /^[vV]?(\d+)\.(\d+)\.(\d+)$/.exec(raw.trim());
  if (!match) return null;
  return [Number(match[1]), Number(match[2]), Number(match[3])];
}

/**
 * 纯判定：返回 `{ ok, reason }`。
 * - `previous` 为空 / `none` ⇒ ok（首个清单）；
 * - 任一版本非法 ⇒ 不 ok（fail-closed，不做任何宽容解析）；
 * - `current` 必须严格大于 `previous`（相等也不 ok：相等正是「永远已是最新」的病根）。
 */
export function evaluateCliAdvance(current, previous) {
  const currentTriple = parseCliVersion(current);
  if (!currentTriple) {
    return { ok: false, reason: `本轮 CLI 版本非法：${JSON.stringify(current)}` };
  }
  if (previous === null || previous === undefined || previous === "" || previous === "none") {
    return { ok: true, reason: "上游无历史清单（首个 CLI 清单），跳过递增校验" };
  }
  const previousTriple = parseCliVersion(previous);
  if (!previousTriple) {
    return {
      ok: false,
      reason: `上游清单 version 非法：${JSON.stringify(previous)}（无法证明递增，fail-closed）`,
    };
  }
  for (let index = 0; index < 3; index += 1) {
    if (currentTriple[index] > previousTriple[index]) {
      return { ok: true, reason: `${previous} → ${current}（严格递增）` };
    }
    if (currentTriple[index] < previousTriple[index]) {
      return {
        ok: false,
        reason: `${current} 低于上游清单版本 ${previous}（回退版本不能被客户端发现）`,
      };
    }
  }
  return {
    ok: false,
    reason:
      `${current} 与上游清单版本相同 —— 客户端会一直判定「已是最新」，` +
      "本次 CLI 构建永远不会被下载。请用 `node scripts/release.mjs`（默认 target=all）发版，" +
      "或显式 `node scripts/release.mjs --target core` 单独递增 core 版本线",
  };
}

function parseArgs(argv) {
  const options = { current: null, previous: "none" };
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    if (flag === "--current") {
      options.current = argv[++index];
    } else if (flag === "--previous") {
      options.previous = argv[++index];
    } else if (flag === "--help" || flag === "-h") {
      return { help: true };
    } else {
      return { error: `未知参数：${flag}` };
    }
  }
  if (!options.current) {
    return { error: "缺少 --current <ver>" };
  }
  return options;
}

function main(argv) {
  const parsed = parseArgs(argv);
  if (parsed.help) {
    console.log(
      "用法: node scripts/check-cli-manifest-advances.mjs --current <ver> [--previous <ver>|none]"
    );
    return 0;
  }
  if (parsed.error) {
    console.error(`[check-cli-manifest-advances] ${parsed.error}`);
    return 2;
  }
  const verdict = evaluateCliAdvance(parsed.current, parsed.previous);
  if (verdict.ok) {
    console.log(`[check-cli-manifest-advances] PASS: ${verdict.reason}`);
    return 0;
  }
  console.error(`[check-cli-manifest-advances] FAIL: ${verdict.reason}`);
  return 1;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  process.exit(main(process.argv.slice(2)));
}
