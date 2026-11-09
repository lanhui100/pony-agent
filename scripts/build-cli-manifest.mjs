#!/usr/bin/env node
/**
 * 装配 CLI 更新清单 `pony-cli-latest.json`（发布侧唯一实现，CI 与本机共用）。
 *
 * 取代 release.yml 里的内联 python（v0.1.18x 起）：清单形状与 sha256 口径从此可在本机跑单测，
 * 不再只有推 tag 才知道对不对。macOS/Linux 的 `shasum` / `sha256sum` 差异也一并消掉 ——
 * 摘要只由这一个实现产出。
 *
 * 用法：
 *   node scripts/build-cli-manifest.mjs --version <core 版本> --dir <制品目录> --out <清单路径>
 *     [--generated-at <RFC3339 UTC>] [--platforms <逗号分隔白名单>] [--no-required]
 *   退出码：0 成功 / 1 契约或 IO 失败 / 2 用法错误。
 *
 * 产出：清单本体 + 每个平台的 `<asset>.sha256` 旁证文件（GNU `sha256sum` 原生格式
 * `<hex>  <filename>`，两空格分隔），供人工核对与 `sha256sum -c`。
 */
import { createHash } from "node:crypto";
import { createReadStream, promises as fs } from "node:fs";
import path from "node:path";
import process from "node:process";

import { parseCliVersion } from "./check-cli-manifest-advances.mjs";
import {
  CLI_MANIFEST_ASSET,
  CLI_MANIFEST_SCHEMA,
  PUBLISHED_PLATFORM_IDS,
  REQUIRED_PLATFORM,
  assetNameFor,
} from "./cli-platforms.mjs";

const RFC3339 = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(\.\d+)?Z$/;

/** 流式 sha256（制品是 30MB 级二进制，禁止整文件进内存）。 */
export function sha256File(filePath) {
  return new Promise((resolve, reject) => {
    const hash = createHash("sha256");
    const stream = createReadStream(filePath);
    stream.on("error", reject);
    stream.on("data", (chunk) => hash.update(chunk));
    stream.on("end", () => resolve(hash.digest("hex")));
  });
}

/** GNU `sha256sum` 原生格式：`<hex>  <文件名>`（两个空格），结尾换行。 */
export function sidecarContent(assetName, sha256Hex) {
  return `${sha256Hex}  ${assetName}\n`;
}

/**
 * 纯装配：把每平台制品条目装配成清单对象。任何不合契约的输入都 throw（fail-closed）。
 * `entries` = `[{ platform, asset, sha256, size }]`。
 */
export function buildCliManifest({
  version,
  generatedAt,
  entries,
  requirePlatform = REQUIRED_PLATFORM,
}) {
  if (!parseCliVersion(version)) {
    throw new Error(`core 版本非法：${JSON.stringify(version)}（清单 version 必须是 X.Y.Z）`);
  }
  if (!RFC3339.test(String(generatedAt))) {
    throw new Error(`generated_at 非 RFC3339 UTC：${JSON.stringify(generatedAt)}`);
  }
  if (!Array.isArray(entries) || entries.length === 0) {
    throw new Error("没有任何平台制品可装配：清单不得写成空 platforms（客户端会判清单非法）");
  }

  const platforms = new Map();
  for (const entry of entries) {
    const { platform, asset, sha256, size } = entry;
    if (!PUBLISHED_PLATFORM_IDS.includes(platform)) {
      throw new Error(
        `未登记的平台 ${JSON.stringify(platform)}：新增发布平台必须同步写进 scripts/cli-platforms.mjs`
      );
    }
    const expectedAsset = assetNameFor(platform, version);
    if (asset !== expectedAsset) {
      throw new Error(`资产名漂移：平台 ${platform} 期望 ${expectedAsset}，实际 ${asset}`);
    }
    if (platforms.has(platform)) {
      throw new Error(`平台 ${platform} 出现重复条目（清单必须是每平台一项）`);
    }
    if (!/^[0-9a-f]{64}$/.test(String(sha256))) {
      throw new Error(`${platform} 的 sha256 不是 64 位小写 hex：${JSON.stringify(sha256)}`);
    }
    if (!Number.isInteger(size) || size <= 0) {
      throw new Error(`${platform} 的 size 非法：${JSON.stringify(size)}（必须为正整数字节数）`);
    }
    platforms.set(platform, { asset, sha256, size });
  }

  if (requirePlatform && !platforms.has(requirePlatform)) {
    throw new Error(
      `必需平台 ${requirePlatform} 本轮无制品 —— 发布不得只带边缘平台（客户端发现会落到更旧 release）`
    );
  }

  const sorted = {};
  for (const key of [...platforms.keys()].sort()) {
    sorted[key] = platforms.get(key);
  }
  return {
    schema: CLI_MANIFEST_SCHEMA,
    version,
    generated_at: generatedAt,
    platforms: sorted,
  };
}

/**
 * 扫描制品目录，按发布矩阵收集条目。目录里出现的 `pony-tui_*` 文件若不合名（未知平台 / 版本不符），
 * 直接 throw —— 静默忽略会让「CI 多写了一个平台名」变成清单里少一个平台且无人报错。
 */
export async function collectArtifactEntries(dir, version) {
  const wanted = new Map(PUBLISHED_PLATFORM_IDS.map((id) => [assetNameFor(id, version), id]));
  const names = (await fs.readdir(dir)).sort();
  const entries = [];
  for (const name of names) {
    if (name.endsWith(".sha256") || name === CLI_MANIFEST_ASSET) continue;
    const platform = wanted.get(name);
    if (!platform) {
      if (name.startsWith("pony-tui_")) {
        throw new Error(
          `制品目录里有未登记的资产 ${name}：期望名 pony-tui_${version}_<平台>，` +
            `平台 ∈ ${PUBLISHED_PLATFORM_IDS.join(" / ")}`
        );
      }
      continue;
    }
    const stat = await fs.stat(path.join(dir, name));
    if (!stat.isFile()) continue;
    entries.push({
      platform,
      asset: name,
      sha256: await sha256File(path.join(dir, name)),
      size: stat.size,
    });
  }
  return entries;
}

function parseArgs(argv) {
  const options = { version: null, dir: null, out: null, generatedAt: null, require: REQUIRED_PLATFORM };
  for (let index = 0; index < argv.length; index += 1) {
    const flag = argv[index];
    if (flag === "--version") options.version = argv[++index];
    else if (flag === "--dir") options.dir = argv[++index];
    else if (flag === "--out") options.out = argv[++index];
    else if (flag === "--generated-at") options.generatedAt = argv[++index];
    else if (flag === "--no-required") options.require = null;
    else if (flag === "--help" || flag === "-h") return { help: true };
    else return { error: `未知参数：${flag}` };
  }
  if (!options.version || !options.dir || !options.out) {
    return { error: "必须提供 --version / --dir / --out" };
  }
  return { options };
}

function usage() {
  return [
    "用法：node scripts/build-cli-manifest.mjs --version <core 版本> --dir <制品目录> --out <清单路径>",
    "  [--generated-at <RFC3339 UTC>] [--no-required]",
    `  发布矩阵真值源：scripts/cli-platforms.mjs（当前 ${PUBLISHED_PLATFORM_IDS.join(" / ")}）`,
  ].join("\n");
}

export async function main(argv = process.argv.slice(2)) {
  const parsed = parseArgs(argv);
  if (parsed.help) {
    process.stdout.write(`${usage()}\n`);
    return 0;
  }
  if (parsed.error) {
    process.stderr.write(`${parsed.error}\n${usage()}\n`);
    return 2;
  }
  const { version, dir, out, generatedAt, require: requirePlatform } = parsed.options;
  const stamp = generatedAt ?? new Date().toISOString().replace(/\.\d{3}Z$/, "Z");

  let manifest;
  try {
    const entries = await collectArtifactEntries(dir, version);
    manifest = buildCliManifest({ version, generatedAt: stamp, entries, requirePlatform });
  } catch (error) {
    process.stderr.write(`清单装配失败：${error.message}\n`);
    return 1;
  }

  const missing = PUBLISHED_PLATFORM_IDS.filter((id) => !manifest.platforms[id]);
  if (missing.length > 0) {
    process.stderr.write(
      `[WARN] 本轮缺失平台：${missing.join(" / ")} ⇒ 这些机器上 ` +
        "`pony update` 会报 UnsupportedPlatform（错误串会列出清单已有平台）\n"
    );
  }

  for (const [id, asset] of Object.entries(manifest.platforms)) {
    await fs.writeFile(path.join(dir, `${asset.asset}.sha256`), sidecarContent(asset.asset, asset.sha256));
    process.stdout.write(`platform=${id} asset=${asset.asset} sha256=${asset.sha256} size=${asset.size}\n`);
  }
  await fs.writeFile(out, `${JSON.stringify(manifest, null, 2)}\n`);
  process.stdout.write(`manifest=${out} version=${manifest.version} platforms=${Object.keys(manifest.platforms).length}\n`);
  return 0;
}

const invokedPath = process.argv[1] ? path.resolve(process.argv[1]) : "";
if (invokedPath === path.resolve(new URL(import.meta.url).pathname)) {
  main().then((code) => process.exit(code));
}
