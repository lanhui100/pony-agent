#!/usr/bin/env node
/**
 * 门禁：CLI 发布契约三方对齐（真值源 ↔ release.yml ↔ 客户端）。
 *
 * 拦住的现场故障（2026-11-09，ADR 0108）：客户端 `current_platform()` 认 5 个平台，
 * release.yml 只产出 linux-x86_64 一个资产 —— 这是「承诺了但不发」，macOS/ARM 用户
 * 每次 `pony update` 都吃 UnsupportedPlatform，而**没有任何命令会红**。反向同样致命：
 * CI 发布一个客户端不认识的平台，那份制品永远不会被下载（死制品）。
 *
 * 用法：
 *   node scripts/check-cli-release-contract.mjs [--json]
 *   退出码：0 对齐 / 1 漂移（非零退出即门禁）。
 */
import { readFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import {
  CLI_MANIFEST_ASSET,
  evaluateReleaseContract,
  parseClientManifestAssetConstant,
  parseClientPlatformVocabulary,
  parseWorkflowPlatformIds,
} from "./cli-platforms.mjs";

const REPO_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const UPDATER_REL = "crates/pony-agent-core/src/agent/updater.rs";
const WORKFLOW_REL = ".github/workflows/release.yml";

export function checkCliReleaseContract(repoRoot = REPO_ROOT) {
  const updaterSource = readFileSync(path.join(repoRoot, UPDATER_REL), "utf8");
  const workflowText = readFileSync(path.join(repoRoot, WORKFLOW_REL), "utf8");

  const verdict = evaluateReleaseContract({
    clientVocabulary: parseClientPlatformVocabulary(updaterSource),
    workflowPlatformIds: parseWorkflowPlatformIds(workflowText),
    assetConstant: parseClientManifestAssetConstant(updaterSource),
  });
  const problems = [...verdict.problems];

  // 清单必须由本仓脚本装配（单一实现）；内联 python/shell 重新长回来就等于绕开单测。
  if (!workflowText.includes("scripts/build-cli-manifest.mjs")) {
    problems.push(
      "release.yml 未调用 scripts/build-cli-manifest.mjs 装配清单：清单形状必须只有一个实现，" +
        "否则本机单测覆盖不到发布侧（ADR 0108 决定 2）"
    );
  }
  // 必需平台的构建必须真的编译 `pony`（而不是别的 bin）。
  if (!workflowText.includes("--bin pony")) {
    problems.push("release.yml 的 CLI 构建未使用 `cargo build --release --bin pony`");
  }

  return {
    ok: problems.length === 0,
    problems,
    published: parseWorkflowPlatformIds(workflowText),
    clientVocabulary: [...parseClientPlatformVocabulary(updaterSource)].sort(),
    assetConstant: parseClientManifestAssetConstant(updaterSource),
    frozenAsset: CLI_MANIFEST_ASSET,
  };
}

function main(argv) {
  const asJson = argv.includes("--json");
  let result;
  try {
    result = checkCliReleaseContract();
  } catch (error) {
    process.stderr.write(`CLI 发布契约门禁无法执行：${error.message}\n`);
    return 1;
  }
  if (asJson) {
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
  } else if (result.ok) {
    process.stdout.write(
      `CLI 发布契约 ok：发布平台 ${result.published.join(" / ")}；` +
        `客户端词表 ${result.clientVocabulary.join(" / ")}；清单资产名冻结为 ${result.assetConstant}\n`
    );
  } else {
    process.stderr.write("CLI 发布契约漂移：\n");
    for (const problem of result.problems) process.stderr.write(`  - ${problem}\n`);
  }
  return result.ok ? 0 : 1;
}

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url))) {
  process.exitCode = main(process.argv.slice(2));
}
