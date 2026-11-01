#!/usr/bin/env node
/**
 * W57-A TS/Rust 契约漂移门禁（纯 Node，零依赖）。
 *
 * 输入：`docs/contracts/ts-contract.snapshot.json` —— 由 Rust 侧
 *   `cargo test -p pony-agent-core --test ts_contract_snapshot`（`UPDATE_CONTRACT_SNAPSHOT=1`）
 *   用 `serde_json::to_value` 真实序列化产出，键名禁止手抄。
 * 判定：快照里出现的每个 serde 线缆键，必须在对应的 TS 声明（`export type/interface`）里存在。
 *   **只查「Rust 有、TS 无」**（漏声明 → 前端静默丢字段）；不查反向（TS 允许有本地派生字段）。
 * 退出码：0 = 无漂移；1 = 有漏声明 / 快照或声明缺失。
 */

import { readFileSync, existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SNAPSHOT_PATH = resolve(repoRoot, "docs/contracts/ts-contract.snapshot.json");

function fail(message) {
  console.error(`[contract:check] ERROR: ${message}`);
  process.exit(1);
}

if (!existsSync(SNAPSHOT_PATH)) {
  fail(
    `契约快照缺失：${SNAPSHOT_PATH}\n` +
      "  请先运行：UPDATE_CONTRACT_SNAPSHOT=1 cargo test -p pony-agent-core --test ts_contract_snapshot",
  );
}

let snapshot;
try {
  snapshot = JSON.parse(readFileSync(SNAPSHOT_PATH, "utf8"));
} catch (error) {
  fail(`契约快照不是合法 JSON：${error.message}`);
}

const types = snapshot.types ?? {};
const tsDeclarations = snapshot.tsDeclarations ?? {};
const typeNames = Object.keys(types);

if (typeNames.length === 0) {
  fail("契约快照里没有任何类型（types 为空）");
}

/**
 * 抽取某个 `export type X = { ... }` / `export interface X { ... }` 的**第一层**属性键。
 * 用花括号深度计数，避免把嵌套对象字面量里的键误算成本类型字段。
 */
function extractTopLevelKeys(source, typeName) {
  const declarationPattern = new RegExp(
    `export\\s+(?:type|interface)\\s+${typeName}\\b[^{]*\\{`,
    "m",
  );
  const match = declarationPattern.exec(source);
  if (!match) {
    return null;
  }

  const bodyStart = match.index + match[0].length;
  let depth = 1;
  let index = bodyStart;
  const keys = [];
  let lineStart = bodyStart;

  const flushLine = (lineEnd) => {
    if (depth !== 1) {
      return;
    }
    const line = source.slice(lineStart, lineEnd);
    // 只识别 "key:" / "key?:" / "readonly key:" / 带引号的键；忽略注释行与嵌套块。
    const keyMatch = /^\s*(?:readonly\s+)?["']?([A-Za-z_$][\w$]*)["']?\s*\??\s*:/.exec(line);
    if (keyMatch) {
      keys.push(keyMatch[1]);
    }
  };

  while (index < source.length) {
    const char = source[index];
    if (char === "\n") {
      flushLine(index);
      lineStart = index + 1;
    } else if (char === "{") {
      depth += 1;
    } else if (char === "}") {
      flushLine(index);
      depth -= 1;
      if (depth === 0) {
        break;
      }
      lineStart = index + 1;
    }
    index += 1;
  }

  // 最后一行（无换行收尾）也纳入
  flushLine(index);
  return keys;
}

const cache = new Map();
const problems = [];

for (const typeName of typeNames) {
  const serdeKeys = types[typeName]?.serdeKeys ?? [];
  const tsFile = tsDeclarations[typeName];
  if (!tsFile) {
    problems.push(`${typeName}: 快照缺少 tsDeclarations 映射，无法定位 TS 声明`);
    continue;
  }

  const absolutePath = resolve(repoRoot, tsFile);
  if (!existsSync(absolutePath)) {
    problems.push(`${typeName}: TS 声明文件不存在 → ${tsFile}`);
    continue;
  }

  if (!cache.has(tsFile)) {
    cache.set(tsFile, readFileSync(absolutePath, "utf8"));
  }
  const source = cache.get(tsFile);
  const declaredKeys = extractTopLevelKeys(source, typeName);
  if (declaredKeys === null) {
    problems.push(`${typeName}: 在 ${tsFile} 中找不到 \`export type/interface ${typeName}\``);
    continue;
  }

  const declared = new Set(declaredKeys);
  const missing = serdeKeys.filter((key) => !declared.has(key));
  if (missing.length > 0) {
    problems.push(
      `${typeName}（${tsFile}）缺少 Rust 已下发的字段：${missing.join(", ")}\n` +
        `           Rust 源：${types[typeName]?.rustSource ?? "unknown"}`,
    );
  }

  console.log(
    `[contract:check] ${typeName}: Rust ${serdeKeys.length} 键 / TS 声明 ${declared.size} 键 ` +
      `${missing.length === 0 ? "OK" : `缺失 ${missing.length}`}`,
  );
}

if (problems.length > 0) {
  console.error("\n[contract:check] 契约漂移（Rust 有、TS 无）：");
  for (const problem of problems) {
    console.error(`  - ${problem}`);
  }
  console.error(
    "\n修复方式：在 TS 声明中补上缺失字段（camelCase，与 serde 线缆一致），" +
      "而不是改快照。若 Rust 侧字段确实不该下发，请在实现侧改用 skip_serializing。",
  );
  process.exit(1);
}

console.log("[contract:check] 通过：无「Rust 有、TS 无」的字段漂移。");
