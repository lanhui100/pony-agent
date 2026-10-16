#!/usr/bin/env node
/**
 * Wave-17 空桩纯度机械校验器（TypeScript 等价实现）
 * =============================================================================
 * 定位：`[GATE EXTENDED: WAVE-STUB-LINT(TS-equivalent)]`
 *   仓库内**无** TS 空桩纯度门禁：仅存在 `/tmp/dev-team-sandbox/scripts/
 *   dev-stub-lint.py`（skill 沙箱参考件，基于 Python `ast` 解析 **.py** 源码，
 *   对 `src/lib/ime.ts` 物理不可用）。故本脚本为 TypeScript 侧的等价机械校验，
 *   断言语义对齐该参考件（body 语句数受限 + 只允许 raise/throw NotImplemented）。
 *
 * 目的：证明 `src/lib/ime.ts` 在协议特派员交付时刻是**纯空桩**，
 * 不含任何实现逻辑（实现归 executor-a）。
 *
 * 断言（任一不满足 -> EXIT=1）：
 *   1. 文件存在，且恰好导出 1 个函数 `isImeComposingEvent`；
 *   2. 参数类型为 `Event | null | undefined`，返回类型为 `boolean`；
 *   3. 函数体（去空行后）**有且仅有 1 条语句**：`throw new Error("NotImplemented");`
 *      —— 机械意义上的「空桩纯度」；函数体有效行数 <= 3；
 *   4. 函数体内不得出现任何控制流/类型判定/属性访问等实现痕迹
 *      （if/else/return/typeof/instanceof/switch/for/while/try/catch/&&/||/??/
 *        isComposing/keyCode/event.）；
 *   5. 必须存在 JSDoc 契约注释（`/** ... *\/`）。
 *
 * 用法：node .dev-team/tools/validate-stub-purity.mjs [目标文件]
 * 退出码：0=纯度通过；1=纯度失败；2=文件缺失/读取错误。
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, "..", "..");
const DEFAULT_TARGET = path.join(REPO_ROOT, "src", "lib", "ime.ts");

const FN_NAME = "isImeComposingEvent";
const PURE_STATEMENT = /^throw new Error\("NotImplemented"\);$/;
const MAX_BODY_LINES = 3;
const LOGIC_TOKENS =
  /\b(if|else|return|typeof|instanceof|switch|case|for|while|do|try|catch|finally|new\s+KeyboardEvent)\b|&&|\|\||\?\?|isComposing|keyCode|event\.|\.key\b/;

function check(source) {
  const errors = [];

  // 断言 1：恰好 1 个导出函数，名字正确
  const exportRe = new RegExp(
    `export\\s+function\\s+([A-Za-z_$][\\w$]*)\\s*\\(([^)]*)\\)\\s*:\\s*([^{]+)\\{`,
    "g",
  );
  const exports = [...source.matchAll(exportRe)];
  if (exports.length !== 1) {
    errors.push(`导出函数数量必须为 1，实际 ${exports.length}`);
  }
  const hit = exports[0];
  if (!hit) return errors;

  if (hit[1] !== FN_NAME) {
    errors.push(`导出函数名必须为 ${FN_NAME}，实际 ${hit[1]}`);
  }

  // 断言 2：签名类型
  const params = hit[2].trim();
  if (!/^[A-Za-z_$][\w$]*\s*:\s*Event\s*\|\s*null\s*\|\s*undefined$/.test(params)) {
    errors.push(`参数签名必须为 "event: Event | null | undefined"，实际 "${params}"`);
  }
  const returnType = hit[3].trim();
  if (returnType !== "boolean") {
    errors.push(`返回类型必须为 boolean，实际 "${returnType}"`);
  }

  // 断言 3：函数体只有一条 NotImplemented 抛出
  const bodyStart = hit.index + hit[0].length;
  let depth = 1;
  let end = -1;
  for (let i = bodyStart; i < source.length; i += 1) {
    const ch = source[i];
    if (ch === "{") depth += 1;
    else if (ch === "}") {
      depth -= 1;
      if (depth === 0) {
        end = i;
        break;
      }
    }
  }
  if (end === -1) {
    errors.push("函数体大括号不闭合（解析失败）");
    return errors;
  }
  const bodyLines = source
    .slice(bodyStart, end)
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean);
  if (bodyLines.length === 0) {
    errors.push("函数体为空，必须为 throw new Error(\"NotImplemented\")");
  } else if (bodyLines.length > MAX_BODY_LINES) {
    errors.push(`函数体有效行数 ${bodyLines.length} > ${MAX_BODY_LINES}`);
  }
  if (bodyLines.length !== 1 || !PURE_STATEMENT.test(bodyLines[0])) {
    errors.push(
      `函数体必须仅含 1 条 throw new Error("NotImplemented")，实际 ${JSON.stringify(bodyLines)}`,
    );
  }

  // 断言 4：不得出现实现痕迹
  const logic = bodyLines.join(" ");
  const logicHit = logic.match(LOGIC_TOKENS);
  if (logicHit) {
    errors.push(`函数体内检测到实现逻辑痕迹: "${logicHit[0]}"`);
  }

  // 断言 5：JSDoc 契约注释存在
  const jsdocIdx = source.lastIndexOf("/**", hit.index);
  if (jsdocIdx === -1 || hit.index - jsdocIdx > 2000) {
    errors.push("缺少紧邻函数的 JSDoc 契约注释");
  }

  return errors;
}

const target = process.argv[2]
  ? path.resolve(process.cwd(), process.argv[2])
  : DEFAULT_TARGET;

let source;
try {
  source = readFileSync(target, "utf8");
} catch (e) {
  console.error(`[stub-lint] ERROR: 无法读取 ${target}: ${e.message}`);
  process.exit(2);
}

const errors = check(source);
const rel = path.relative(REPO_ROOT, target);
if (errors.length === 0) {
  const body = source.slice(source.indexOf(") : boolean {") + 14);
  console.log(`[stub-lint] target = ${rel}`);
  console.log(`[stub-lint] export = ${FN_NAME}(Event | null | undefined): boolean`);
  console.log(`[stub-lint] body = throw new Error("NotImplemented") [1/1 语句]`);
  console.log(
    `[stub-lint] PASS: 纯空桩（无实现逻辑、无控制流、无属性访问），总行数 ${source.split("\n").length}`,
  );
  void body;
  process.exit(0);
}
for (const e of errors) console.error(`[stub-lint] FAIL: ${e}`);
console.error(`[stub-lint] RESULT: FAIL (${errors.length} 项)`);
process.exit(1);
