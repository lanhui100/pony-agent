#!/usr/bin/env node
/**
 * Wave-17 NFR 基准机械校验器（**本波专用 5 键严格校验**）
 * =============================================================================
 * 定位：`[GATE EXTENDED: PROJECT-LINT(16-key) + WAVE-LINT(5-key)]`
 *   - 项目自带 `.dev-team/nfr-lint.py`（18KB，Draft-07 + PA-118 死锁/日志范围
 *     硬约束）继续对**既有 16 键冻结基准** `.dev-team/nfr-baseline.json` 生效，
 *     本波不修改该文件（Lead 裁定 B：旁路文件，最小闭包）。
 *   - 本脚本对**本波 5 键基准** `.dev-team/nfr-baseline-wave17.json` 做严格校验
 *     （必填且仅允许 5 键、无未知键、类型/阈值、applicable 诚实性）。
 *   两者职责不同、互补，需同时运行。
 *
 * 与「人工目检」的区别（防零假测试）：
 *   - 所有断言均为代码级硬断言，任一条不满足即 EXIT=1；
 *   - `--self-test` 注入 7 组反例（未知键 / 缺键 / 超时越界 / 类型错误 /
 *     logging.format 非 json / trace_id 非 true / applicable:false 却无 reason），
 *     逐条断言必须被拒绝 —— 证明本校验器不是恒真。
 *
 * 校验规则（对应 task-1）：
 *   1. 根必须是 JSON 对象；
 *   2. 必填且**仅允许** 5 个键：external_call_timeout_ms / retry /
 *      concurrency_lock / logging / resource_release（未知键一律拒绝）；
 *   3. external_call_timeout_ms 必须为整数且 1 <= v <= 3000；
 *   4. retry / concurrency_lock 必须为对象，且必须显式声明 applicable:boolean；
 *      applicable=false 时必须给出非空 reason（诚实标注不适用）；
 *      applicable=true 时禁止用 reason 搪塞，且必须给出真实机制键；
 *   5. logging 必须为对象且 format==="json"、trace_id===true，并同样受
 *      applicable/reason 诚实性约束；
 *   6. resource_release 必须为非空数组，元素为对象且同样受
 *      applicable/reason 诚实性约束。
 *
 * 用法：
 *   node .dev-team/tools/validate-nfr-baseline.mjs [目标文件]   # 默认 nfr-baseline-wave17.json
 *   node .dev-team/tools/validate-nfr-baseline.mjs --self-test
 * 退出码：0=通过；1=校验失败；2=用法/文件/解析错误。
 */

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(HERE, "..", "..");
const DEFAULT_TARGET = path.join(REPO_ROOT, ".dev-team", "nfr-baseline-wave17.json");

const ALLOWED_ROOT_KEYS = [
  "external_call_timeout_ms",
  "retry",
  "concurrency_lock",
  "logging",
  "resource_release",
];
const TIMEOUT_MAX_MS = 3000;

/** 各诚实性分支允许的键集（未知键一律拒绝，防「为凑 schema 编造机制」）。 */
const SHAPES = {
  retry: ["applicable", "reason", "max_attempts", "backoff_cap_ms"],
  concurrency_lock: ["applicable", "reason", "primitive", "scope"],
  logging: ["applicable", "reason", "format", "trace_id"],
  resource_release_item: ["applicable", "reason", "handle", "release"],
};

const isPlainObject = (v) => typeof v === "object" && v !== null && !Array.isArray(v);
const isNonEmptyString = (v) => typeof v === "string" && v.trim().length > 0;
const typeName = (v) =>
  v === null ? "null" : Array.isArray(v) ? "array" : typeof v;

/** 诚实性通用分支：applicable:boolean 必填；false 需 reason；true 需真实机制键。 */
function checkHonesty(node, label, allowedKeys, mechanismKeys, errors) {
  const keys = Object.keys(node);
  for (const k of keys) {
    if (!allowedKeys.includes(k)) {
      errors.push(`${label}: 未知键 "${k}"（允许：${allowedKeys.join(", ")}）`);
    }
  }
  if (typeof node.applicable !== "boolean") {
    errors.push(
      `${label}.applicable: 必填且必须为 boolean（实际 ${typeName(node.applicable)}）`,
    );
    return;
  }
  if (node.applicable === false) {
    if (!isNonEmptyString(node.reason)) {
      errors.push(
        `${label}.reason: applicable=false 时必须给出非空 reason（诚实标注不适用），实际 ${typeName(node.reason)}`,
      );
    }
    return;
  }
  // applicable === true：必须给出真实机制，不得用 reason 搪塞。
  if ("reason" in node) {
    errors.push(
      `${label}.reason: applicable=true 时禁止以 reason 搪塞，必须落盘真实机制字段`,
    );
  }
  const declared = mechanismKeys.filter((k) => k in node);
  if (declared.length === 0) {
    errors.push(
      `${label}: applicable=true 但未声明任何真实机制字段（需至少一个：${mechanismKeys.join(", ")}）`,
    );
  }
}

/** 纯函数校验：返回错误字符串数组（空数组 = 通过）。 */
export function validateBaseline(doc) {
  const errors = [];
  if (!isPlainObject(doc)) {
    errors.push(`根节点必须是 JSON 对象（实际 ${typeName(doc)}）`);
    return errors;
  }

  // 规则 2：必填且仅允许 5 键
  const keys = Object.keys(doc);
  for (const k of ALLOWED_ROOT_KEYS) {
    if (!keys.includes(k)) errors.push(`缺少必填键 "${k}"`);
  }
  for (const k of keys) {
    if (!ALLOWED_ROOT_KEYS.includes(k)) {
      errors.push(`未知根键 "${k}"（仅允许：${ALLOWED_ROOT_KEYS.join(", ")}）`);
    }
  }

  // 规则 3：external_call_timeout_ms
  const t = doc.external_call_timeout_ms;
  if (t !== undefined) {
    if (!Number.isInteger(t)) {
      errors.push(
        `external_call_timeout_ms: 必须为整数（实际 ${typeName(t)}）`,
      );
    } else if (t < 1 || t > TIMEOUT_MAX_MS) {
      errors.push(
        `external_call_timeout_ms: 必须满足 1 <= v <= ${TIMEOUT_MAX_MS}（实际 ${t}）`,
      );
    }
  }

  // 规则 4：retry / concurrency_lock
  for (const [key, mechanismKeys] of [
    ["retry", ["max_attempts", "backoff_cap_ms"]],
    ["concurrency_lock", ["primitive", "scope"]],
  ]) {
    const node = doc[key];
    if (node === undefined) continue;
    if (!isPlainObject(node)) {
      errors.push(`${key}: 必须为对象（实际 ${typeName(node)}）`);
      continue;
    }
    checkHonesty(node, key, SHAPES[key], mechanismKeys, errors);
  }

  // 规则 5：logging
  const logging = doc.logging;
  if (logging !== undefined) {
    if (!isPlainObject(logging)) {
      errors.push(`logging: 必须为对象（实际 ${typeName(logging)}）`);
    } else {
      checkHonesty(
        logging,
        "logging",
        SHAPES.logging,
        ["format", "trace_id"],
        errors,
      );
      if (logging.format !== "json") {
        errors.push(
          `logging.format: 必须严格等于 "json"（实际 ${JSON.stringify(logging.format)}）`,
        );
      }
      if (logging.trace_id !== true) {
        errors.push(
          `logging.trace_id: 必须严格等于 true（实际 ${JSON.stringify(logging.trace_id)}）`,
        );
      }
    }
  }

  // 规则 6：resource_release
  const rr = doc.resource_release;
  if (rr !== undefined) {
    if (!Array.isArray(rr)) {
      errors.push(`resource_release: 必须为数组（实际 ${typeName(rr)}）`);
    } else if (rr.length === 0) {
      errors.push("resource_release: 不得为空数组（须显式声明适用性）");
    } else {
      rr.forEach((item, i) => {
        const label = `resource_release[${i}]`;
        if (!isPlainObject(item)) {
          errors.push(`${label}: 必须为对象（实际 ${typeName(item)}）`);
          return;
        }
        checkHonesty(
          item,
          label,
          SHAPES.resource_release_item,
          ["handle", "release"],
          errors,
        );
      });
    }
  }

  return errors;
}

const VALID_FIXTURE = {
  external_call_timeout_ms: 3000,
  retry: { applicable: false, reason: "no external IO in this wave" },
  concurrency_lock: { applicable: false, reason: "no shared mutable state" },
  logging: {
    applicable: false,
    reason: "no logging surface added",
    format: "json",
    trace_id: true,
  },
  resource_release: [{ applicable: false, reason: "no handles introduced" }],
};

/** 反例注入：证明校验器不是恒真（防零假测试）。 */
const MUTATIONS = [
  ["未知根键", (d) => ({ ...d, extra_key: 1 })],
  ["缺少必填键", (d) => { const c = { ...d }; delete c.retry; return c; }],
  ["超时越界(3001)", (d) => ({ ...d, external_call_timeout_ms: 3001 })],
  ["超时非整数", (d) => ({ ...d, external_call_timeout_ms: "3000" })],
  ["logging.format 非 json", (d) => ({ ...d, logging: { ...d.logging, format: "text" } })],
  ["logging.trace_id 非 true", (d) => ({ ...d, logging: { ...d.logging, trace_id: false } })],
  ["applicable:false 却无 reason", (d) => ({ ...d, retry: { applicable: false } })],
  ["resource_release 空数组", (d) => ({ ...d, resource_release: [] })],
];

function runSelfTest() {
  let failed = 0;
  const base = validateBaseline(VALID_FIXTURE);
  if (base.length === 0) {
    console.log("SELFTEST[valid-fixture] PASS (被接受，符合预期)");
  } else {
    failed += 1;
    console.log(`SELFTEST[valid-fixture] FAIL: 合法样本被误拒 -> ${base.join(" | ")}`);
  }
  for (const [name, mutate] of MUTATIONS) {
    const errs = validateBaseline(mutate(structuredClone(VALID_FIXTURE)));
    if (errs.length > 0) {
      console.log(`SELFTEST[${name}] PASS (被拒绝: ${errs[0]})`);
    } else {
      failed += 1;
      console.log(`SELFTEST[${name}] FAIL: 反例被放行 -> 校验器恒真`);
    }
  }
  const total = MUTATIONS.length + 1;
  console.log(
    `SELFTEST SUMMARY: ${total - failed}/${total} 通过；反例 ${MUTATIONS.length} 组`,
  );
  if (failed > 0) {
    console.error(`SELFTEST RESULT: FAIL (${failed} 项未达预期)`);
    process.exit(1);
  }
  console.log("SELFTEST RESULT: OK (校验器非恒真)");
  process.exit(0);
}

function main(argv) {
  if (argv.includes("--self-test")) return runSelfTest();
  if (argv.includes("--help") || argv.includes("-h")) {
    console.log(
      "用法: node .dev-team/tools/validate-nfr-baseline.mjs [目标JSON] | --self-test",
    );
    process.exit(0);
  }
  const positional = argv.filter((a) => !a.startsWith("-"));
  const target = positional.length > 0 ? path.resolve(process.cwd(), positional[0]) : DEFAULT_TARGET;

  let raw;
  try {
    raw = readFileSync(target, "utf8");
  } catch (e) {
    console.error(`[nfr-lint] ERROR: 无法读取 ${target}: ${e.message}`);
    process.exit(2);
  }
  let doc;
  try {
    doc = JSON.parse(raw);
  } catch (e) {
    console.error(`[nfr-lint] ERROR: ${target} 不是合法 JSON: ${e.message}`);
    process.exit(2);
  }

  const errors = validateBaseline(doc);
  const keys = isPlainObject(doc) ? Object.keys(doc) : [];
  console.log(`[nfr-lint] target = ${path.relative(REPO_ROOT, target)}`);
  console.log(`[nfr-lint] root keys (${keys.length}) = ${keys.join(", ")}`);
  console.log(
    `[nfr-lint] external_call_timeout_ms = ${JSON.stringify(doc.external_call_timeout_ms)} (max ${TIMEOUT_MAX_MS})`,
  );
  console.log(
    `[nfr-lint] logging.format = ${JSON.stringify(doc.logging?.format)}, logging.trace_id = ${JSON.stringify(doc.logging?.trace_id)}`,
  );
  if (errors.length === 0) {
    console.log(
      `[nfr-lint] PASS: 5 键齐备 / 无未知键 / 类型与阈值全部满足 / applicable 诚实性通过`,
    );
    process.exit(0);
  }
  for (const e of errors) console.error(`[nfr-lint] FAIL: ${e}`);
  console.error(`[nfr-lint] RESULT: FAIL (${errors.length} 项)`);
  process.exit(1);
}

main(process.argv.slice(2));
