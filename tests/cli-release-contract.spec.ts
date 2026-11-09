import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";

import {
  CLI_MANIFEST_ASSET,
  PUBLISHED_PLATFORM_IDS,
  REQUIRED_PLATFORM,
  assetNameFor,
  evaluateReleaseContract,
  parseClientManifestAssetConstant,
  parseClientPlatformVocabulary,
  parseWorkflowPlatformIds,
} from "../scripts/cli-platforms.mjs";
import {
  buildCliManifest,
  collectArtifactEntries,
  sidecarContent,
} from "../scripts/build-cli-manifest.mjs";
import { checkCliReleaseContract } from "../scripts/check-cli-release-contract.mjs";

const UPDATER_REL = "crates/pony-agent-core/src/agent/updater.rs";
const WORKFLOW_REL = ".github/workflows/release.yml";

function sha256(buffer: Buffer) {
  return createHash("sha256").update(buffer).digest("hex");
}

/**
 * 契约（ADR 0108）：客户端词表、发布矩阵真值源、release.yml 三处必须对齐。
 * 这组用例是 P0 现场故障的回归 —— 「客户端认 5 个平台、CI 只发 1 个」此前没有任何命令会红。
 */
describe("cli release contract (ADR 0108)", () => {
  it("HEAD 上三方对齐", () => {
    const verdict = checkCliReleaseContract();
    expect(verdict.problems).toEqual([]);
    expect(verdict.ok).toBe(true);
    expect(verdict.published.sort()).toEqual([...PUBLISHED_PLATFORM_IDS].sort());
  });

  it("release.yml 真的为每个发布平台准备了矩阵条目与 runner", () => {
    const workflowText = readFileSync(WORKFLOW_REL, "utf8");
    const ids = parseWorkflowPlatformIds(workflowText);
    expect(ids).toContain(REQUIRED_PLATFORM);
    expect(ids).toContain("linux-aarch64");
    expect(ids).toContain("macos-aarch64");
    expect(ids).not.toContain("windows-x86_64");
    // 清单必须由脚本装配（内联 python/shell 重新长回来 = 绕开本文件这些用例）
    expect(workflowText).toContain("scripts/build-cli-manifest.mjs");
  });

  it("客户端词表覆盖全部发布平台，且已冻结的清单资产名未被改写", () => {
    const source = readFileSync(UPDATER_REL, "utf8");
    const vocabulary = parseClientPlatformVocabulary(source);
    for (const id of PUBLISHED_PLATFORM_IDS) {
      expect(vocabulary.has(id), `客户端必须认识发布平台 ${id}`).toBe(true);
    }
    expect(parseClientManifestAssetConstant(source)).toBe(CLI_MANIFEST_ASSET);
  });
});

/** 变异用例：任何一条规则被绕过都必须能红，否则门禁只是装饰。 */
describe("evaluateReleaseContract 变异", () => {
  const good = {
    clientVocabulary: new Set([
      "linux-x86_64",
      "linux-aarch64",
      "macos-aarch64",
      "macos-x86_64",
      "windows-x86_64",
    ]),
    workflowPlatformIds: [...PUBLISHED_PLATFORM_IDS],
    assetConstant: CLI_MANIFEST_ASSET,
  };

  it("CI 少发一个平台（回到「只发 linux-x86_64」）⇒ 红", () => {
    const verdict = evaluateReleaseContract({ ...good, workflowPlatformIds: ["linux-x86_64"] });
    expect(verdict.ok).toBe(false);
    expect(verdict.problems.join("\n")).toContain("与真值源");
  });

  it("CI 偷偷加一个客户端不认识的平台 ⇒ 红（死制品）", () => {
    const verdict = evaluateReleaseContract({
      clientVocabulary: new Set(["linux-x86_64", "linux-aarch64"]),
      workflowPlatformIds: ["linux-x86_64", "linux-aarch64"],
      assetConstant: CLI_MANIFEST_ASSET,
    });
    expect(verdict.ok).toBe(false);
    expect(verdict.problems.join("\n")).toContain("死制品");
  });

  it("workflow 写了真值源里没登记的平台 ⇒ 红，并点名要改 cli-platforms.mjs", () => {
    const verdict = evaluateReleaseContract({
      ...good,
      workflowPlatformIds: [...PUBLISHED_PLATFORM_IDS, "linux-riscv64"],
    });
    expect(verdict.ok).toBe(false);
    expect(verdict.problems.join("\n")).toContain("cli-platforms.mjs");
  });

  it("把 windows-x86_64 放进发布矩阵 ⇒ 红（运行中的 exe 不能被自身替换，ADR 0087 决定 7）", () => {
    const workflowText = [
      "          - platform: linux-x86_64",
      "          - platform: windows-x86_64",
    ].join("\n");
    expect(parseWorkflowPlatformIds(workflowText)).toEqual(["linux-x86_64", "windows-x86_64"]);
    const verdict = evaluateReleaseContract({
      ...good,
      workflowPlatformIds: ["linux-x86_64", "windows-x86_64"],
    });
    expect(verdict.ok).toBe(false);
    const joined = verdict.problems.join("\n");
    expect(joined).toContain("矩阵");
    expect(joined).toContain("windows-x86_64 排进了");
    expect(joined).toContain("桌面端 NSIS 安装包");
  });

  it("清单资产名被改名 ⇒ 红（存量二进制会静默断更）", () => {
    const verdict = evaluateReleaseContract({ ...good, assetConstant: "pony-tui-latest.json" });
    expect(verdict.ok).toBe(false);
    expect(verdict.problems.join("\n")).toContain("发现不到更新");
  });
});

describe("buildCliManifest 装配契约", () => {
  const entry = (overrides: Record<string, unknown> = {}) => ({
    platform: "linux-x86_64",
    asset: assetNameFor("linux-x86_64", "0.1.159"),
    sha256: "a".repeat(64),
    size: 1234,
    ...overrides,
  });
  const base = { version: "0.1.159", generatedAt: "2026-11-09T00:00:00Z" };

  it("产出 schema/version/generated_at/platforms，形状与客户端 CliManifest 一致", () => {
    const manifest = buildCliManifest({ ...base, entries: [entry()] });
    expect(manifest).toEqual({
      schema: "pony-cli-manifest",
      version: "0.1.159",
      generated_at: "2026-11-09T00:00:00Z",
      platforms: {
        "linux-x86_64": { asset: "pony-tui_0.1.159_linux-x86_64", sha256: "a".repeat(64), size: 1234 },
      },
    });
  });

  it("多平台按 key 排序，且每个平台的资产名必须带自己的平台后缀", () => {
    const manifest = buildCliManifest({
      ...base,
      entries: [
        entry(),
        entry({
          platform: "macos-aarch64",
          asset: assetNameFor("macos-aarch64", "0.1.159"),
          sha256: "b".repeat(64),
          size: 10,
        }),
        entry({
          platform: "linux-aarch64",
          asset: assetNameFor("linux-aarch64", "0.1.159"),
          sha256: "c".repeat(64),
          size: 20,
        }),
      ],
    });
    expect(Object.keys(manifest.platforms)).toEqual(["linux-aarch64", "linux-x86_64", "macos-aarch64"]);
    for (const [id, asset] of Object.entries(manifest.platforms)) {
      expect(asset.asset).toBe(assetNameFor(id, "0.1.159"));
    }
  });

  it("fail-closed：空条目 / 非法版本 / 非小写 hex / 非正 size / 重复平台 / 资产名漂移 / 未登记平台 / 时间戳非法", () => {
    expect(() => buildCliManifest({ ...base, entries: [] })).toThrow(/没有任何平台制品/);
    expect(() => buildCliManifest({ ...base, version: "0.1", entries: [entry()] })).toThrow(/版本非法/);
    expect(() => buildCliManifest({ ...base, entries: [entry({ sha256: "A".repeat(64) })] })).toThrow(
      /64 位小写/
    );
    expect(() => buildCliManifest({ ...base, entries: [entry({ size: 0 })] })).toThrow(/size 非法/);
    expect(() => buildCliManifest({ ...base, entries: [entry(), entry()] })).toThrow(/重复条目/);
    expect(() =>
      buildCliManifest({ ...base, entries: [entry({ asset: "pony-tui_0.1.159_linux" })] })
    ).toThrow(/资产名漂移/);
    expect(() =>
      buildCliManifest({
        ...base,
        entries: [entry({ platform: "windows-x86_64", asset: "pony-tui_0.1.159_windows-x86_64" })],
      })
    ).toThrow(/未登记的平台/);
    expect(() =>
      buildCliManifest({ version: "0.1.159", generatedAt: "2026-11-09 00:00", entries: [entry()] })
    ).toThrow(/RFC3339/);
  });

  it("必需平台缺失 ⇒ 拒绝出清单（不允许只带边缘平台发布）", () => {
    expect(() =>
      buildCliManifest({
        ...base,
        entries: [
          entry({
            platform: "macos-aarch64",
            asset: assetNameFor("macos-aarch64", "0.1.159"),
            sha256: "d".repeat(64),
            size: 5,
          }),
        ],
      })
    ).toThrow(/必需平台/);
  });

  it("旁证文件格式 = GNU sha256sum 原生 `<hex>  <文件名>`（两空格 + 换行）", () => {
    expect(sidecarContent("pony-tui_0.1.159_linux-x86_64", "f".repeat(64))).toBe(
      `${"f".repeat(64)}  pony-tui_0.1.159_linux-x86_64\n`
    );
  });
});

describe("collectArtifactEntries 扫描制品目录", () => {
  function fixtureDir(files: Record<string, string>) {
    const dir = mkdtempSync(path.join(tmpdir(), "cli-manifest-"));
    for (const [name, content] of Object.entries(files)) {
      writeFileSync(path.join(dir, name), content);
    }
    return dir;
  }

  it("按发布矩阵收条目，sha256 与 size 来自真实字节；旁证/清单文件忽略", async () => {
    const payload = "pony-binary";
    const dir = fixtureDir({
      [assetNameFor("linux-x86_64", "0.1.159")]: payload,
      [assetNameFor("macos-aarch64", "0.1.159")]: "mac-payload",
      [`${assetNameFor("linux-x86_64", "0.1.159")}.sha256`]: "stale",
      [CLI_MANIFEST_ASSET]: "stale",
      "unrelated.txt": "x",
    });
    const entries = await collectArtifactEntries(dir, "0.1.159");
    expect(entries.map((item) => item.platform).sort()).toEqual(["linux-x86_64", "macos-aarch64"]);
    const linux = entries.find((item) => item.platform === "linux-x86_64");
    expect(linux?.sha256).toBe(sha256(Buffer.from(payload)));
    expect(linux?.size).toBe(Buffer.byteLength(payload));
  });

  it("目录里出现未登记的 pony-tui_* 资产 ⇒ 抛错（静默忽略会变成清单少一个平台且无人报错）", async () => {
    const dir = fixtureDir({
      [assetNameFor("linux-x86_64", "0.1.159")]: "x",
      "pony-tui_0.1.159_windows-x86_64": "y",
    });
    await expect(collectArtifactEntries(dir, "0.1.159")).rejects.toThrow(/未登记的资产/);
  });

  it("必需平台没有制品 ⇒ 与 buildCliManifest 串联后整轮拦下", async () => {
    const dir = fixtureDir({ [assetNameFor("macos-aarch64", "0.1.159")]: "y" });
    const entries = await collectArtifactEntries(dir, "0.1.159");
    expect(() =>
      buildCliManifest({ version: "0.1.159", generatedAt: "2026-11-09T00:00:00Z", entries })
    ).toThrow(/必需平台/);
  });
});
