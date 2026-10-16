/**
 * wave-18 F1 + C5 契约测试：前端 PonySentry 凭证不得硬编码回落
 *
 * 契约基线：
 *   - `.dev-team/contract-matrix-wave18.md` §5 F1（第三处泄漏点）；
 *   - Lead 裁定升版冻结契约 **C5（wave18-r2）**：`INGEST_URL` / `CLIENT_TOKEN`
 *     **仅**取自 `import.meta.env`，未设即空（`""`）；两者任一为空时上报路径必须
 *     early-return —— 不发 `fetch`、不带 `X-Client-Token`；
 *   - wave18-r2 §5.4 **反空洞断言**要求：每条 C5 验收断言都必须能因“恢复硬编码回落”
 *     而变红（mutation-sensitive），不得依赖 `src/lib/telemetry.ts` 的 test-mode
 *     静默守卫（`MODE === "test"` 等）独立达成的“fetch 零调用”。
 *
 * 本文件按 §5.4 组织为三族断言，每族都对“恢复硬编码回落”变异敏感：
 *   族 1 导出值断言      —— 未设 env 时导出常量必须严格等于 `""`；
 *   族 2 源码静态断言    —— 无 64-hex 字面量、无非空字面量回落形态、存在 early-return 守卫；
 *   族 3 守卫直测        —— 显式绕开 test-mode 静默路径，落在真实上报分支上断言零 egress。
 *
 * 红相基线（迁移前）：`src/lib/telemetry.ts:13-15` 的 `CLIENT_TOKEN` 与 `:10-12` 的
 * `INGEST_URL` 均带非空字面量回落，故无凭证时仍会发出未鉴权 ingest `fetch`。
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it, vi } from "vitest";

const specDir = path.dirname(fileURLToPath(import.meta.url));
const TELEMETRY_SOURCE_PATH = path.resolve(specDir, "../src/lib/telemetry.ts");

/** 硬编码端点回落的指纹（生产 ingest 域名）。 */
const HARDCODED_ENDPOINT_FINGERPRINT = "sentry.ponyjob.top";

/** 返回源码中“裸 64-hex 连续段”所在行号（只回显行号，不回显凭证本体）。 */
function bare64HexLines(source: string): number[] {
  const hits: number[] = [];
  source.split(/\r?\n/).forEach((line, index) => {
    if (/(?<![0-9a-fA-F])[0-9a-fA-F]{64}(?![0-9a-fA-F])/.test(line)) {
      hits.push(index + 1);
    }
  });
  return hits;
}

/** 提取 `export const <name> = ...;` 声明体（用于静态形状断言）。 */
function constDeclarationBody(source: string, name: string): string {
  const start = source.indexOf(`export const ${name}`);
  expect(start, `未找到 ${name} 的导出声明`).toBeGreaterThanOrEqual(0);
  const end = source.indexOf(";", start);
  return source.slice(start, end === -1 ? undefined : end);
}

/**
 * 声明体是否含“非空字面量回落”形态：`|| "<非空>"` / `?? '<非空>'` / 反引号变体。
 * `|| ""`（回落到空串，符合 C5）不视为回落。
 */
function hasLiteralFallback(declBody: string): boolean {
  return /\|\||\?\?/.test(declBody) && /(?:\|\||\?\?)\s*(["'`])(?:(?!\1)[\s\S])+\1/.test(declBody);
}

/**
 * `reportError` 中是否存在 C5 要求的 early-return 守卫：
 * 在 `fetch(` 之前出现 `if (... INGEST_URL ... CLIENT_TOKEN ...) { ... return ... }`
 * （两种标识符顺序均接受）。
 */
function hasC5EarlyReturnGuard(source: string): boolean {
  const reportStart = source.indexOf("export async function reportError");
  if (reportStart < 0) return false;
  const reportBody = source.slice(reportStart);
  const fetchIndex = reportBody.indexOf("fetch(");
  const region = fetchIndex >= 0 ? reportBody.slice(0, fetchIndex) : reportBody;
  const patterns = [
    /if\s*\([^;{]*INGEST_URL[^;{]*CLIENT_TOKEN[^;{]*\)\s*\{?[\s\S]{0,120}?\breturn\b/,
    /if\s*\([^;{]*CLIENT_TOKEN[^;{]*INGEST_URL[^;{]*\)\s*\{?[\s\S]{0,120}?\breturn\b/,
  ];
  return patterns.some((pattern) => pattern.test(region));
}

/**
 * 绕开 `reportError` 的 test-mode 静默守卫（`MODE === "test"` / `TEST` / `VITEST`），
 * 让断言落在**真实浏览器 fetch 分支**上 —— 这是 §5.4 反空洞断言的关键前置。
 */
function enableBrowserEgressPath(): void {
  vi.stubEnv("MODE", "production");
  vi.stubEnv("TEST", "");
  vi.stubEnv("VITEST", "");
}

/** 以当前 env 快照重新求值 telemetry 模块（`CLIENT_TOKEN` 等为模块级常量）。 */
async function loadTelemetryFresh() {
  vi.resetModules();
  return await import("../src/lib/telemetry");
}

function stubFetchMock() {
  const fetchMock = vi.fn().mockResolvedValue({ ok: true, status: 200 });
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

/** 是否已有任何 fetch 调用携带了 `X-Client-Token` 头。 */
function carriesClientTokenHeader(fetchMock: ReturnType<typeof stubFetchMock>): boolean {
  return fetchMock.mock.calls.some((call) => {
    const init = (call as unknown as [string, RequestInit?])[1];
    const headers = (init?.headers ?? {}) as Record<string, string>;
    return Boolean(headers["X-Client-Token"]);
  });
}

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
  vi.resetModules();
});

describe("wave-18 C5 §5.4: 前端 PonySentry 凭证门禁（变异敏感断言）", () => {
  // -------------------------------------------------------------------------
  // 族 1：导出值断言（未设 env ⇒ 严格空串）
  // -------------------------------------------------------------------------

  it("族1 导出值：未设 VITE_PONYSENTRY_CLIENT_TOKEN 时 CLIENT_TOKEN === \"\"", async () => {
    // Arrange
    vi.stubEnv("VITE_PONYSENTRY_CLIENT_TOKEN", "");

    // Act
    const telemetry = await loadTelemetryFresh();

    // Assert：严格 `=== ""`；用布尔比较 + 长度而非直接 toBe("")，
    // 避免失败输出回显凭证本体（仅暴露长度）
    expect(telemetry.CLIENT_TOKEN === "").toBe(true);
    expect(telemetry.CLIENT_TOKEN.length).toBe(0);
  });

  it("族1 导出值：未设 VITE_PONYSENTRY_INGEST_URL 时 INGEST_URL === \"\"", async () => {
    // Arrange
    vi.stubEnv("VITE_PONYSENTRY_INGEST_URL", "");

    // Act
    const telemetry = await loadTelemetryFresh();

    // Assert：严格 `=== ""`；布尔比较 + 长度，失败输出不回显端点取值
    expect(telemetry.INGEST_URL === "").toBe(true);
    expect(telemetry.INGEST_URL.length).toBe(0);
  });

  // -------------------------------------------------------------------------
  // 族 2：源码静态断言（无字面量回落 / 无硬编码端点 / 有 early-return 守卫）
  // -------------------------------------------------------------------------

  it("族2 静态：src/lib/telemetry.ts 不得含任何裸 64-hex 字面量", () => {
    // Arrange
    const source = fs.readFileSync(TELEMETRY_SOURCE_PATH, "utf8");

    // Act
    const offenders = bare64HexLines(source);

    // Assert
    expect(offenders).toEqual([]);
  });

  it("族2 静态：CLIENT_TOKEN / INGEST_URL 声明不得含非空字面量回落，也不得硬编码端点", () => {
    // Arrange
    const source = fs.readFileSync(TELEMETRY_SOURCE_PATH, "utf8");
    const tokenDecl = constDeclarationBody(source, "CLIENT_TOKEN");
    const ingestDecl = constDeclarationBody(source, "INGEST_URL");

    // Act
    const tokenHasFallback = hasLiteralFallback(tokenDecl);
    const ingestHasFallback = hasLiteralFallback(ingestDecl);
    const endpointHardcoded = source.includes(HARDCODED_ENDPOINT_FINGERPRINT);

    // Assert（断言布尔而非直接 match 字符串，避免失败输出回显凭证本体）
    expect(tokenDecl).toContain("VITE_PONYSENTRY_CLIENT_TOKEN");
    expect(ingestDecl).toContain("VITE_PONYSENTRY_INGEST_URL");
    expect(tokenHasFallback).toBe(false);
    expect(ingestHasFallback).toBe(false);
    expect(endpointHardcoded).toBe(false);
  });

  it("族2 静态：reportError 必须在 fetch 之前具备 INGEST_URL/CLIENT_TOKEN 的 early-return 守卫", () => {
    // Arrange
    const source = fs.readFileSync(TELEMETRY_SOURCE_PATH, "utf8");

    // Act
    const guardPresent = hasC5EarlyReturnGuard(source);

    // Assert
    expect(guardPresent).toBe(true);
  });

  // -------------------------------------------------------------------------
  // 族 3：守卫直测（绕开 test-mode 静默路径，落在真实上报分支）
  // -------------------------------------------------------------------------

  it("族3 守卫直测：未设 token 时不得发出任何 ingest fetch（零 egress，调用次数 0）", async () => {
    // Arrange：解除 test-mode 静默 + 关闭 Tauri 分支 ⇒ 断言不再依赖静默守卫
    enableBrowserEgressPath();
    vi.stubEnv("VITE_PONYSENTRY_CLIENT_TOKEN", "");
    vi.stubEnv("VITE_PONYSENTRY_INGEST_URL", "http://127.0.0.1:9");
    const fetchMock = stubFetchMock();
    const telemetry = await loadTelemetryFresh();

    // Act
    await telemetry.reportError({ message: "wave18 zero-egress probe (no token)" });

    // Assert：零调用，且任何已发生的调用都不得携带 X-Client-Token
    expect(fetchMock).toHaveBeenCalledTimes(0);
    expect(carriesClientTokenHeader(fetchMock)).toBe(false);
  });

  it("族3 守卫直测：未设 ingest url 时不得发出任何 ingest fetch（C5，调用次数 0）", async () => {
    // Arrange
    enableBrowserEgressPath();
    vi.stubEnv("VITE_PONYSENTRY_CLIENT_TOKEN", "test-token-123");
    vi.stubEnv("VITE_PONYSENTRY_INGEST_URL", "");
    const fetchMock = stubFetchMock();
    const telemetry = await loadTelemetryFresh();

    // Act
    await telemetry.reportError({ message: "wave18 zero-egress probe (no endpoint)" });

    // Assert：零调用，且任何已发生的调用都不得携带 X-Client-Token
    expect(fetchMock).toHaveBeenCalledTimes(0);
    expect(carriesClientTokenHeader(fetchMock)).toBe(false);
  });

  it("族3 反空洞对照：显式注入 token + endpoint 时必须恰好 1 次 fetch 且携带 X-Client-Token", async () => {
    // Arrange：证明上面的“零 fetch”不是靠整体禁用上报（或静默守卫）换来的
    enableBrowserEgressPath();
    vi.stubEnv("VITE_PONYSENTRY_CLIENT_TOKEN", "test-token-123");
    vi.stubEnv("VITE_PONYSENTRY_INGEST_URL", "http://127.0.0.1:9");
    const fetchMock = stubFetchMock();
    const telemetry = await loadTelemetryFresh();

    // Act
    await telemetry.reportError({ message: "wave18 positive control" });

    // Assert
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const call = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(String(call[0])).toContain("/api/v1/ingest");
    const headers = (call[1]?.headers ?? {}) as Record<string, string>;
    expect(headers["X-Client-Token"]).toBe("test-token-123");
  });
});
