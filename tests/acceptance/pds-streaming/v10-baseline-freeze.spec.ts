/**
 * wave-pds-3 · V10：既有基线不回归（冻结 + **真实执行**）—— vitest
 *
 * ── wave-61 §5.1 追加授权（Lead 二轮裁决 R3，2026-11-03）──────────────────────
 * `FROZEN_FILES` 中 `tests/acceptance/wave59/wave59-f2-status-metrics-layout.spec.ts` 的 SHA-256 **重锚**。
 *   原意图：哈希冻结防止「改测试让它绿」——任何未经授权的改测试都会立刻变红。
 *   迁移理由：wave-61 §5 已由 Lead 一票授权迁移该文件的 token 行锚点（class ⇒ `status-metrics-tokens`），
 *             授权成立后锚必须随授权同步，否则 G-1 永红（冻结会退化为"禁止合法迁移"的死锁）。
 *   等价性：只重算**被 §5 授权改动的这一个文件**的哈希，其余 7 个锚逐字未动；
 *           V10-2/V10-3 的「子进程真实执行」判据不变。
 *
 * 契约源（只读）：`.dev-team/contracts/wave-pds-3-streaming-visual.md` §6 V10
 *   「既有 `tests/useStreamingPresentationState.spec.ts`(14) + `tests/acceptance/wave59/`(23) + 全量单测全绿」。
 *
 * 为什么不是重抄那 37 例：
 *   重抄等于给冻结基线留一份可漂移的副本（两份断言各自演化时，"不回归"就没有真值了）。
 *   本文件用**两道互补的真凭证**：
 *   ① V10-1 哈希锚定：冻结文件内容一旦被改动（哪怕只是新增一条断言）立即变红 —— 防止「改测试让它绿」；
 *   ② V10-2/V10-3 子进程**真实执行**：跑真正的 vitest，断言 exit code 与用例计数。
 *      不是读历史收据、不是「我跑过了」的口头结论。
 *
 * V10-3 的范围说明（刻意排除本目录）：红相阶段本目录的 V1/V2/V3/V7 用例按契约预期为红，
 *   若把本目录纳入「全量单测全绿」求解，V10 会因自身所在目录的预期红相而永远变红，
 *   从而掩盖「既有单测是否被本波搞坏」这一唯一有效信息。故 V10-3 的语义是
 *   「**既有**全量单测（排除 tests/acceptance/pds-streaming/**）全绿」；实现完成后由 Lead 用
 *   `npx vitest run` 覆盖含本目录的整包。
 */
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { REPO_ROOT } from "./pds-streaming-harness";

/**
 * 冻结文件内容锚（SHA-256，2026-11-02 红相锚定前实测）。
 * 这些文件**只读**：契约 §7 规定 Executor 禁止修改 `tests/**`，Test Agent 禁止修改 wave59 目录。
 */
const FROZEN_FILES: Array<{ path: string; sha256: string; note: string }> = [
  {
    path: "tests/useStreamingPresentationState.spec.ts",
    sha256: "cb4a5c7985aae8dc5fce5297d28bcf6e5a060bf8fd1fc88aa7b02dc6ce3ba5a6",
    note: "14 例等价性基线"
  },
  {
    path: "tests/acceptance/wave59/wave59-harness.ts",
    sha256: "2348f1f471ea465444aaf179093fdb49c968ebe7e8bbf997943d1a4d3c4ba663",
    note: "wave-59 冻结夹具"
  },
  {
    path: "tests/acceptance/wave59/wave59-f1-waiting-animation.spec.ts",
    sha256: "0dd9238ddcb4741cd439a9e279de037caef37fb6a727d2c93e22d5108df59078",
    note: "wave-59 F1（5 例）"
  },
  {
    path: "tests/acceptance/wave59/wave59-f2-status-metrics-layout.spec.ts",
    sha256: "5e1e73872390fec02e5ba79bcdd7fa00563d5b5b5dcab3bd62ae514e922ededb",
    note: "wave-59 F2（4 例）· wave-61 授权重锚"
  },
  {
    path: "tests/acceptance/wave59/wave59-f3-streaming-dom-stability.spec.ts",
    sha256: "fc52d8d535ec62f98c46c8a9fb8cbd5118fd2ef6c0759cfda822496f5808c3de",
    note: "wave-59 F3（6 例）"
  },
  {
    path: "tests/acceptance/wave59/wave59-f4-runtime-status-accuracy.spec.ts",
    sha256: "7325c87cb6460bf2ff1cfecefdec19356ba65abfab92fdc28aa7258291396dfc",
    note: "wave-59 F4（8 例）"
  },
  {
    path: "tests/e2e/wave59-stream-stability.spec.ts",
    sha256: "d01e4b21ca999150162fed6f078516d35cdc9c0c9866e63b4d83d900d5be04f4",
    note: "wave-59 F3-5 浏览器冻结用例"
  },
  {
    path: "tests/pds-integration-parity.spec.ts",
    sha256: "d243a6013d379464a529d72b4a89f2de7e500df29c0a102dad6be6327b833f75",
    note: "PDS-SM 接入对拍 + charsPerTick=4 黄金表（与契约 §4 存在冲突，见交付说明）"
  }
];

const VITEST_BIN = resolve(REPO_ROOT, "node_modules/.bin/vitest");

interface RunReceipt {
  exitCode: number | null;
  stdout: string;
  stderr: string;
  combined: string;
}

/** 真实子进程执行 vitest（不是读历史收据，也不是「跑过了」的自述）。 */
function runVitest(args: string[], timeoutMs: number): RunReceipt {
  const result = spawnSync(VITEST_BIN, args, {
    cwd: REPO_ROOT,
    encoding: "utf8",
    timeout: timeoutMs,
    env: { ...process.env, CI: "1", FORCE_COLOR: "0", NO_COLOR: "1" }
  });
  const stdout = result.stdout ?? "";
  const stderr = result.stderr ?? "";
  return {
    exitCode: result.status,
    stdout,
    stderr,
    combined: `${stdout}\n${stderr}`
  };
}

function sha256Of(relativePath: string): string {
  return createHash("sha256").update(readFileSync(resolve(REPO_ROOT, relativePath))).digest("hex");
}

describe("wave-pds-3 V10：既有基线不回归", () => {
  it("V10-1: 冻结基线文件内容未被触碰（SHA-256 锚定）", () => {
    // Arrange
    const missing = FROZEN_FILES.filter((entry) => !existsSync(resolve(REPO_ROOT, entry.path)));

    // Act + Assert
    expect(missing.map((entry) => entry.path), "V10-1：冻结基线文件不得缺失").toHaveLength(0);

    const changed = FROZEN_FILES
      .map((entry) => ({ path: entry.path, note: entry.note, actual: sha256Of(entry.path), expected: entry.sha256 }))
      .filter((entry) => entry.actual !== entry.expected);

    expect(
      changed,
      `V10-1：以下冻结文件的哈希已变化（契约 §7 规定无人可改 tests/ 冻结基线；`
        + `若确需变更，须由 Lead 走契约修订并同步本锚）：${JSON.stringify(changed, null, 2)}`
    ).toHaveLength(0);
  });

  it(
    "V10-2: 实跑冻结基线（14 例 + wave-59 23 例）—— exit code 0",
    () => {
      // Arrange
      expect(existsSync(VITEST_BIN), `V10-2：找不到 vitest 可执行文件：${VITEST_BIN}`).toBe(true);

      // Act：真实执行
      const receipt = runVitest(
        [
          "run",
          // default：只有 default reporter 会逐文件打印 `(N tests)`（verbose 逐条、dot 不打印），
          // 而我们**必须**核对 14 / 23 这两个计数（否则"文件存在"就能冒充"用例仍在跑"）。
          "--reporter=default",
          "--exclude",
          "tests/acceptance/pds-streaming/**",
          "tests/useStreamingPresentationState.spec.ts",
          "tests/acceptance/wave59"
        ],
        300_000
      );

      // Assert ①：退出码
      expect(
        receipt.exitCode,
        `V10-2：冻结基线实跑必须 exit 0；实际 exit=${receipt.exitCode}\n${receipt.combined.slice(-4000)}`
      ).toBe(0);

      // Assert ②：14 例基线计数（真实跑到 14 例，而不是"文件存在"）
      const baselineMatch = receipt.combined.match(/tests\/useStreamingPresentationState\.spec\.ts \((\d+) tests?\)/);
      expect(
        baselineMatch,
        `V10-2：输出中必须出现 tests/useStreamingPresentationState.spec.ts 的用例计数；实际输出尾部：\n${receipt.combined.slice(-2000)}`
      ).not.toBeNull();
      expect(
        Number(baselineMatch?.[1] ?? 0),
        "V10-2：useStreamingPresentationState 基线必须恰好 14 例（少一例等于有例被删/被 skip）"
      ).toBe(14);

      // Assert ③：wave-59 四个文件的用例计数之和 = 23
      const wave59Counts = ["f1", "f2", "f3", "f4"]
        .map((id) => receipt.combined.match(new RegExp(`wave59-${id}-[^)]*\\.spec\\.ts \\((\\d+) tests?\\)`)))
        .map((match) => Number(match?.[1] ?? Number.NaN));
      expect(
        wave59Counts,
        `V10-2：wave-59 四个冻结文件的用例计数解析失败（实际：${JSON.stringify(wave59Counts)}）`
      ).not.toContain(Number.NaN);
      expect(
        wave59Counts.reduce((sum, count) => sum + count, 0),
        `V10-2：wave-59 冻结用例必须恰好 23 例（实际分布：${JSON.stringify(wave59Counts)}）`
      ).toBe(23);
    },
    360_000
  );

  it(
    "V10-3: 实跑既有全量单测（排除本波新增验收目录）—— exit code 0",
    () => {
      // Arrange
      expect(existsSync(VITEST_BIN), `V10-3：找不到 vitest 可执行文件：${VITEST_BIN}`).toBe(true);

      // Act
      // 降低子进程自身的并行度：本用例是「全量套件里再跑一遍全量套件」，
      // 与外部 worker 抢 CPU 时会把仓库里 5s 超时的重型用例（如 check-secrets 的全仓扫描）
      // 挤到超时 → 制造与实现无关的假红（实测 1 次）。这不改变断言强度（仍是 exit 0 + 用例数自检）。
      const receipt = runVitest(
        [
          "run",
          "--no-file-parallelism",
          "--maxWorkers=2",
          "--reporter=dot",
          "--exclude",
          "tests/acceptance/pds-streaming/**"
        ],
        600_000
      );

      // Assert ①：退出码
      expect(
        receipt.exitCode,
        `V10-3：既有全量单测必须 exit 0；实际 exit=${receipt.exitCode}\n${receipt.combined.slice(-6000)}`
      ).toBe(0);

      // Assert ②：失败计数必须为 0 且确实执行了用例（防止「0 文件 0 用例」也算绿）
      const failures = receipt.combined.match(/(\d+) failed/);
      expect(
        failures,
        `V10-3：既有单测出现失败用例：${failures?.[0] ?? "（未解析到）"}\n${receipt.combined.slice(-6000)}`
      ).toBeNull();

      const passedMatch = receipt.combined.match(/Tests\s+(\d+) passed/);
      expect(
        passedMatch,
        `V10-3：必须解析到通过的用例总数；实际输出尾部：\n${receipt.combined.slice(-3000)}`
      ).not.toBeNull();
      expect(
        Number(passedMatch?.[1] ?? 0),
        "V10-3 自检：既有全量单测通过数必须显著大于 0（防止空跑当绿）"
      ).toBeGreaterThan(500);
    },
    660_000
  );
});
