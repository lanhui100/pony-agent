#!/usr/bin/env node
/**
 * CLI 发布平台矩阵的唯一真值源。
 *
 * 为什么需要这个文件（2026-11-09，ADR 0108）：
 *   客户端 `crates/pony-agent-core/src/agent/updater.rs::current_platform()` 认识 5 个平台标识，
 *   而 `.github/workflows/release.yml` 历史上只产出 `linux-x86_64` 一个资产 —— 「承诺了但不发」
 *   在 macOS / ARM 机器上表现为 `pony update` 稳定报 UnsupportedPlatform，且没有任何门禁能发现。
 *   本文件把「发布哪些平台、在哪个 runner 上发、哪个平台必须成功」收成一处，由
 *   `scripts/check-cli-release-contract.mjs` 与 workflow + 客户端词表做三方对齐校验。
 *
 * 术语冻结（ADR 0108 决定 4）：
 *   - 「CLI 制品」指 `pony`（TUI 入口）二进制，资产名 `pony-tui_<ver>_<platform>`；
 *   - 清单资产名 `pony-cli-latest.json` 里的 `cli` 指**命令行形态**，不是 `ponycli` 那个 bin；
 *   - `ponycli` 是仓库内无头工具，不产出公开制品（见 crates/pony-agent-core/README.md）。
 */

/** 清单资产名：已烧进存量二进制，改名 = 老客户端永远发现不到更新。 */
export const CLI_MANIFEST_ASSET = "pony-cli-latest.json";

/** 清单 schema 字面量（客户端 `parse_cli_manifest` 硬校验）。 */
export const CLI_MANIFEST_SCHEMA = "pony-cli-manifest";

/** 制品资产名前缀。 */
export const CLI_ASSET_PREFIX = "pony-tui";

/**
 * 本轮发布必须成功的平台：在本机与既有 release 上验证过的路径。
 * 缺失即 publish 失败，不允许「发布成功但一个 CLI 制品都没有」。
 */
export const REQUIRED_PLATFORM = "linux-x86_64";

/**
 * 发布矩阵。`required: false` 的平台走 `continue-on-error`：编译不过时本轮该平台的资产缺席，
 * 客户端明确报 UnsupportedPlatform（错误串会列出清单已有平台），而不是把整次发版拖红。
 *
 * `runner` 用原生 runner 而非交叉编译：native-tls / keyring / dbus 的 C 依赖在交叉目标上
 * 需要成体系 sysroot，失败率高；原生 runner 同时更接近用户机器。
 */
export const CLI_PLATFORMS = [
  {
    id: "linux-x86_64",
    runner: "ubuntu-latest",
    required: true,
    linuxDeps: true,
  },
  {
    id: "linux-aarch64",
    runner: "ubuntu-24.04-arm",
    required: false,
    linuxDeps: true,
  },
  {
    id: "macos-aarch64",
    runner: "macos-latest",
    required: false,
    linuxDeps: false,
  },
];

/**
 * 客户端词表里存在、但**刻意不发布**的平台，附结构性理由。
 * 门禁要求「不发布集合 ∩ 发布集合 = ∅」。
 */
export const NOT_PUBLISHED_PLATFORMS = [
  {
    id: "windows-x86_64",
    reason: "运行中的 .exe 无法被自身替换（ADR 0087 决定 7）；Windows 走桌面端 NSIS 安装包",
  },
  {
    id: "macos-x86_64",
    reason: "GitHub 已无 Intel macOS runner 稳定供给，存量 Intel Mac 趋零；用户从源码构建",
  },
];

export const PUBLISHED_PLATFORM_IDS = CLI_PLATFORMS.map((entry) => entry.id);

/** 制品资产名：`pony-tui_<core 版本>_<平台标识>`。 */
export function assetNameFor(platformId, version) {
  return `${CLI_ASSET_PREFIX}_${version}_${platformId}`;
}

/** 从资产名反解平台标识；不合名的返回 null（fail-closed，不做宽容解析）。 */
export function platformFromAssetName(assetName, version) {
  const prefix = `${CLI_ASSET_PREFIX}_${version}_`;
  if (!assetName.startsWith(prefix)) return null;
  const suffix = assetName.slice(prefix.length);
  const known = PUBLISHED_PLATFORM_IDS.find((id) => id === suffix);
  return known ?? null;
}

/** 解析 release.yml 里 `matrix.include` 声明的 platform 标识集合。 */
export function parseWorkflowPlatformIds(workflowText) {
  const ids = [];
  for (const match of workflowText.matchAll(/^\s*-\s*platform:\s*([A-Za-z0-9_-]+)\s*$/gm)) {
    ids.push(match[1]);
  }
  return ids;
}

/** 解析 updater.rs `current_platform()` 的 match 臂，取客户端认识的标识集合。 */
export function parseClientPlatformVocabulary(sourceText) {
  const ids = new Set();
  const pattern = /\("([a-z]+)",\s*"([A-Za-z0-9_]+)"\)\s*=>\s*Some\("([a-z]+-[A-Za-z0-9_]+)"\)/g;
  for (const match of sourceText.matchAll(pattern)) {
    const [, os, arch, id] = match;
    // 只认 `os + arch => Some(id)` 且 id 必须由两者拼成的臂，避免把测试里手抄的期望表
    // 也当成语义词表（测试表与实现同文，重复计入不影响集合，但口径上只应有一处真值）。
    if (id === `${os}-${arch}`) ids.add(id);
  }
  return ids;
}

/** 解析 updater.rs 里 `CLI_MANIFEST_ASSET` 的字符串字面量（找不到返回 null）。 */
export function parseClientManifestAssetConstant(sourceText) {
  const match = /pub const CLI_MANIFEST_ASSET:\s*&str\s*=\s*"([^"]+)"/.exec(sourceText);
  return match ? match[1] : null;
}

function sortedJson(values) {
  return JSON.stringify([...values].sort());
}

/**
 * 三方对齐判定（纯函数，供门禁与单测复用）。返回 `{ ok, problems }`。
 *
 * 规则：
 *  1. workflow 矩阵的平台集合必须与真值源**完全相等**（多发一个 = 静默漏上传；少发一个 = 承诺落空）；
 *  2. 真值源发布的每个平台必须在客户端词表内，否则该平台的资产没有任何客户端能选中（死制品）；
 *  3. `REQUIRED_PLATFORM` 必须既在真值源又在 workflow；
 *  4. `NOT_PUBLISHED_PLATFORMS` 里的平台不得出现在发布集合（Windows 自更新是被明令否决的路径）；
 *  5. 客户端 `CLI_MANIFEST_ASSET` 字面量必须等于真值源常量（改名会让存量二进制静默断更）。
 */
export function evaluateReleaseContract({ clientVocabulary, workflowPlatformIds, assetConstant }) {
  const problems = [];
  const published = PUBLISHED_PLATFORM_IDS;

  if (sortedJson(workflowPlatformIds ?? []) !== sortedJson(published)) {
    problems.push(
      `release.yml 矩阵 ${JSON.stringify([...(workflowPlatformIds ?? [])].sort())} ` +
        `与真值源 scripts/cli-platforms.mjs ${JSON.stringify([...published].sort())} 不一致`
    );
  }
  for (const id of published) {
    if (!clientVocabulary?.has(id)) {
      problems.push(
        `平台 ${id} 会发布制品，但客户端 current_platform() 不认识它 ⇒ 该资产永远不会被任何客户端选中（死制品）`
      );
    }
  }
  if (!published.includes(REQUIRED_PLATFORM)) {
    problems.push(`必需平台 ${REQUIRED_PLATFORM} 不在发布矩阵里`);
  }
  if (!workflowPlatformIds?.includes(REQUIRED_PLATFORM)) {
    problems.push(`release.yml 矩阵缺少必需平台 ${REQUIRED_PLATFORM}`);
  }
  for (const entry of NOT_PUBLISHED_PLATFORMS) {
    if (published.includes(entry.id)) {
      problems.push(`平台 ${entry.id} 已登记为「刻意不发布」（${entry.reason}），却出现在发布矩阵`);
    }
    if (workflowPlatformIds?.includes(entry.id)) {
      problems.push(
        `release.yml 把 ${entry.id} 排进了 build 矩阵，但该平台的发布方式是桌面安装包而非 CLI 制品：${entry.reason}`
      );
    }
  }
  const knownIds = new Set([...published, ...NOT_PUBLISHED_PLATFORMS.map((entry) => entry.id)]);
  for (const id of workflowPlatformIds ?? []) {
    if (!knownIds.has(id)) {
      problems.push(`release.yml 声明了未登记的平台 ${id}：必须同步写进 cli-platforms.mjs`);
    }
  }
  if (assetConstant !== CLI_MANIFEST_ASSET) {
    problems.push(
      `客户端清单资产名被改成 ${JSON.stringify(assetConstant)}；该常量已烧进存量二进制，` +
        `改名会让老客户端永远发现不到更新（冻结口径见 ADR 0108 决定 4）`
    );
  }
  return { ok: problems.length === 0, problems };
}
