#!/usr/bin/env node
/**
 * C5 锁纪律守卫：`self.graph_runs.lock()` 作用域内的二次加锁调用检测。
 *
 * 契约（.dev-team/contract-matrix-deadlock.md C5）：
 *   任何 `self.graph_runs.lock()` 的作用域内（自加锁点起到其守卫释放点），
 *   不得再调用会二次加锁的方法；也不得在同一守卫生命周期内再次出现
 *   `self.graph_runs.lock()`（直接自死锁）。
 *
 * 禁用调用集合 = 自动发现 ∪ 显式基线（防止白名单过窄漏检）：
 *   1. **自动发现**：解析扫描根下全部 `*.rs`，对每个 `fn`（含 `pub fn` / 多行签名）
 *      用花括号配对求出其函数体，凡函数体内出现 `self.graph_runs.lock()` 的方法名
 *      即视为「会二次加锁」，全部进入禁用集合（当前模块 18 个）。
 *   2. **显式基线**（4 名，并集下限）：`resolve_graph_run_for_retrieval`、
 *      `load_graph_run`、`load_execution_checkpoint`、`load_active_graph_run_for_session`。
 *      其中 `resolve_graph_run_for_retrieval` 与 `load_execution_checkpoint` 自身不直接
 *      加锁，而是**传递性**加锁（经 load_graph_run / load_active_graph_run_for_session），
 *      自动发现抓不到，故必须保留为硬基线。
 *   3. **并集下限门禁**：`自动发现数 < 基线数(4)` ⇒ 判定为脚本自身解析退化（门禁会变松），
 *      立即 `Exit 2` 拒绝出结论。
 *
 * 判定语义（机械、括号级、无启发式）：
 *   1. 剥离注释（含嵌套块注释）与字符串字面量：按字符等长替换为空格，保证偏移量/行号对齐。
 *   2. 花括号配对求出每个 `{` 的匹配 `}`。
 *   3. 对每个 `self.graph_runs.lock()` 出现位置，取其最内层封闭块作为守卫作用域，
 *      再取 [加锁点, 守卫释放点) 作为守卫生命周期区间（加锁点之前的调用不构成违规）；
 *      守卫释放点 = `let <var> = lock()` 的显式 `drop(<var>)`，否则为最内层封闭块结束
 *      （临时守卫无变量名时收敛到当前语句的 `;`），从而避免对顺序加锁的误报。
 *   4. 在该区间内搜索禁用调用 → ERROR；搜索另一个 `self.graph_runs.lock()` → ERROR。
 *
 * 分级（最小闭包 + Lead 机械核实豁免，与 .dev-team/check-lock-discipline.py 冻结口径一致）：
 *   - ERROR（阻塞）：任何**未被机械核实豁免**的违规（含本次死锁路径文件
 *     history_commands.rs）→ 必须清零；
 *   - P1-SAFE（登记，非阻塞）：命中 VERIFIED_SAFE 精确豁免项的违规 → 打印豁免理由但默认
 *     不阻塞本次放行，可用 --strict 升级为阻塞。
 *
 * 豁免项必须精确到 file + 违规种类 + 被调用方法 + 加锁行号，且豁免表若有条目未命中会
 * 打印 STALE 告警（防止豁免变成永久免死金牌）。
 *
 * 退出码：
 *   0 = 无阻塞级违规（GREEN，P1-SAFE 已登记打印）
 *   1 = 存在阻塞级违规（RED）
 *   2 = 用法错误 / --self-test 自检失败
 *
 * 用法：
 *   node scripts/check-graph-run-lock-scope.mjs              # 默认：阻塞级门禁
 *   node scripts/check-graph-run-lock-scope.mjs --strict     # P1-SAFE 也升级为阻塞
 *   node scripts/check-graph-run-lock-scope.mjs --root=<dir> # 扫描其它目录（绿相模拟用）
 *   node scripts/check-graph-run-lock-scope.mjs --self-test  # 守卫自检（含违规样例，防假测试）
 */

import fs from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

const LOCK = 'self.graph_runs.lock()';

/**
 * 显式基线（并集下限）：这 4 名是本次死锁契约点名的方法，必须永远在禁用集合内。
 * 其中 2 名会被自动发现（load_graph_run / load_active_graph_run_for_session），
 * 另 2 名是**传递性**加锁（resolve_graph_run_for_retrieval / load_execution_checkpoint），
 * 自动发现抓不到，故作为硬下限保留。自动发现数低于本基线长度即 Exit 2。
 */
const BASELINE_FORBIDDEN = [
  'resolve_graph_run_for_retrieval',
  'load_graph_run',
  'load_execution_checkpoint',
  'load_active_graph_run_for_session',
];

/** 本次「进入会话页卡死」死锁路径文件：其违规永远为阻塞级（不可豁免）。 */
const BLOCKING_FILES = new Set(['history_commands.rs']);

/**
 * Lead 机械核实为「非死锁」的精确豁免项。
 *
 * 键 = `${relPath}|${kind}|${detail}|${lockLine}`，其中 relPath 相对扫描根
 * （默认 crates/pony-agent-core/src/agent/control_plane），精确到加锁行号，
 * 防止豁免被同文件的新违规蹭用。
 * 值 = 豁免理由（会被原样打印）。
 */
const VERIFIED_SAFE = new Map([
  [
    'mod.rs|forbidden-call|self.load_execution_checkpoint(|2040',
    'Lead 机械核实为非死锁：该调用点 query.session_id=None 且 run_id=None → ' +
      'resolve_graph_run_for_retrieval 短路到 load_active_graph_run_for_session(None)，' +
      '后者在加锁前 early return（graph_projection.rs:378-382）→ 不会二次加锁',
  ],
]);

const SCRIPT_DIR = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(SCRIPT_DIR, '..');
const DEFAULT_SCAN_ROOT = 'crates/pony-agent-core/src/agent/control_plane';

/**
 * 把注释与字符串字面量替换为等长空格，保留全部偏移量与换行。
 * @param {string} src
 * @returns {string}
 */
function stripNonCode(src) {
  const out = src.split('');
  const n = src.length;
  const blank = (start, end) => {
    for (let k = start; k < Math.min(end, n); k += 1) {
      if (out[k] !== '\n') out[k] = ' ';
    }
  };

  let i = 0;
  while (i < n) {
    const c = src[i];
    const next = i + 1 < n ? src[i + 1] : '';
    // 行注释
    if (c === '/' && next === '/') {
      let j = src.indexOf('\n', i);
      if (j === -1) j = n;
      blank(i, j);
      i = j;
      continue;
    }
    // 块注释（Rust 支持嵌套）
    if (c === '/' && next === '*') {
      let depth = 0;
      let j = i;
      while (j < n) {
        if (src.startsWith('/*', j)) {
          depth += 1;
          j += 2;
        } else if (src.startsWith('*/', j)) {
          depth -= 1;
          j += 2;
          if (depth === 0) break;
        } else {
          j += 1;
        }
      }
      blank(i, j);
      i = j;
      continue;
    }
    // 字符串字面量（含转义；Rust 原始字符串在此仓代码中不用于锁路径，按普通串处理）
    if (c === '"') {
      let j = i + 1;
      while (j < n) {
        if (src[j] === '\\') {
          j += 2;
          continue;
        }
        if (src[j] === '"') {
          j += 1;
          break;
        }
        j += 1;
      }
      blank(i, j);
      i = j;
      continue;
    }
    i += 1;
  }
  return out.join('');
}

/**
 * 反向读取一个标识符（用于识别 `let <ident> = self.graph_runs.lock()` 的守卫变量名）。
 * @param {string} code
 * @param {number} endExclusive
 * @returns {{name:string, start:number}|null}
 */
function readIdentifierBackwards(code, endExclusive) {
  let i = endExclusive - 1;
  while (i >= 0 && /\s/.test(code[i])) i -= 1;
  const end = i + 1;
  while (i >= 0 && /[A-Za-z0-9_]/.test(code[i])) i -= 1;
  const start = i + 1;
  if (start >= end) return null;
  const name = code.slice(start, end);
  return /^[A-Za-z_]/.test(name) ? { name, start } : null;
}

/**
 * 若该加锁点形如 `let [mut] <ident> = self.graph_runs.lock()`，返回守卫变量名。
 * @param {string} code
 * @param {number} lockIndex
 * @returns {string|null}
 */
function guardVariableAt(code, lockIndex) {
  let i = lockIndex - 1;
  while (i >= 0 && /\s/.test(code[i])) i -= 1;
  if (i < 0 || code[i] !== '=') return null;

  const ident = readIdentifierBackwards(code, i);
  if (!ident) return null;

  let j = ident.start - 1;
  while (j >= 0 && /\s/.test(code[j])) j -= 1;
  const keywordEnd = j + 1;
  while (j >= 0 && /[A-Za-z0-9_]/.test(code[j])) j -= 1;
  let keyword = code.slice(j + 1, keywordEnd);
  if (keyword === 'mut') {
    let k = j;
    while (k >= 0 && /\s/.test(code[k])) k -= 1;
    const letEnd = k + 1;
    while (k >= 0 && /[A-Za-z0-9_]/.test(code[k])) k -= 1;
    keyword = code.slice(k + 1, letEnd);
  }
  return keyword === 'let' ? ident.name : null;
}

/**
 * 守卫生命周期终点：`let <var> = lock()` 时收敛到显式 `drop(<var>)`；
 * 临时守卫（无变量名）收敛到当前语句的 `;`；两者都再与最内层封闭块取最小值。
 * @param {string} code
 * @param {number} lockIndex
 * @param {number} scopeEnd
 * @returns {number}
 */
function guardLifetimeEnd(code, lockIndex, scopeEnd) {
  const guardVar = guardVariableAt(code, lockIndex);
  if (guardVar !== null) {
    const dropIndex = code.indexOf(`drop(${guardVar})`, lockIndex);
    if (dropIndex !== -1 && dropIndex < scopeEnd) return dropIndex;
    return scopeEnd;
  }
  const statementEnd = code.indexOf(';', lockIndex);
  if (statementEnd !== -1 && statementEnd < scopeEnd) return statementEnd;
  return scopeEnd;
}

/**
 * @param {string} src 原始源码
 * @param {number} index 偏移量
 * @returns {number} 1-based 行号
 */
function lineAt(src, index) {
  let line = 1;
  for (let i = 0; i < index && i < src.length; i += 1) {
    if (src[i] === '\n') line += 1;
  }
  return line;
}

/**
 * 花括号配对：返回 open 偏移量 -> close 偏移量。
 * @param {string} code 已剥离注释/字符串的代码
 * @returns {Map<number, number>}
 */
function braceMap(code) {
  const stack = [];
  const closeOf = new Map();
  for (let i = 0; i < code.length; i += 1) {
    const ch = code[i];
    if (ch === '{') {
      stack.push(i);
    } else if (ch === '}') {
      const open = stack.pop();
      if (open !== undefined) closeOf.set(open, i);
    }
  }
  return closeOf;
}

/**
 * 自动发现「会获取 self.graph_runs 锁」的方法名。
 *
 * 机制：对每个 `fn <name>` 找到其函数体（签名后第一个 `{` 起的配对块），
 * 若函数体内出现 `self.graph_runs.lock()` 则记入禁用集合。
 * 防误配：签名区间内不得再出现 `fn` 关键字（否则该 `{` 属于后续函数）。
 *
 * @param {string} src 原始源码
 * @returns {{name:string, line:number}[]}
 */
function discoverInSource(src) {
  const code = stripNonCode(src);
  const closeOf = braceMap(code);
  const hits = [];
  const fnRe = /\bfn\s+([A-Za-z_][A-Za-z0-9_]*)/g;
  let match;
  while ((match = fnRe.exec(code)) !== null) {
    const name = match[1];
    const brace = code.indexOf('{', match.index + match[0].length);
    if (brace === -1) continue;
    if (/\bfn\b/.test(code.slice(match.index + match[0].length, brace))) continue;
    const bodyEnd = closeOf.get(brace);
    if (bodyEnd === undefined) continue;
    if (code.slice(brace, bodyEnd).includes(LOCK)) {
      hits.push({ name, line: lineAt(src, match.index) });
    }
  }
  return hits;
}

/**
 * 对扫描根下全部 `*.rs` 执行自动发现。
 * @param {string} scanRoot
 * @returns {{names:string[], sites:Map<string,string[]>}}
 */
function discoverLockAcquiringMethods(scanRoot) {
  const sites = new Map();
  if (!fs.existsSync(scanRoot)) return { names: [], sites };
  for (const file of listRustFiles(scanRoot)) {
    const src = fs.readFileSync(file, 'utf8');
    const repoRelative = path.relative(REPO_ROOT, file).split(path.sep).join('/');
    const displayPath = repoRelative.startsWith('..') ? file : repoRelative;
    for (const { name, line } of discoverInSource(src)) {
      if (!sites.has(name)) sites.set(name, []);
      sites.get(name).push(`${displayPath}:${line}`);
    }
  }
  return { names: [...sites.keys()].sort(), sites };
}

/**
 * 括号级扫描单个源码文本。
 * @param {string} src
 * @param {string[]} forbidden 禁用方法名集合（自动发现 ∪ 基线）
 * @returns {{lockLine:number, kind:string, detail:string, callLine:number}[]}
 */
function analyzeSource(src, forbidden) {
  const code = stripNonCode(src);
  const violations = [];

  // 花括号配对：open 偏移量 -> close 偏移量
  const closeOf = braceMap(code);

  const lockPositions = [];
  let cursor = code.indexOf(LOCK);
  while (cursor !== -1) {
    lockPositions.push(cursor);
    cursor = code.indexOf(LOCK, cursor + LOCK.length);
  }

  for (const lockIndex of lockPositions) {
    // 最内层封闭块
    let enclosingOpen = null;
    for (const [open, close] of closeOf) {
      if (open < lockIndex && lockIndex < close) {
        if (enclosingOpen === null || open > enclosingOpen) enclosingOpen = open;
      }
    }
    const scopeEnd = enclosingOpen === null ? code.length : closeOf.get(enclosingOpen);
    // 守卫生命周期：自加锁点起（加锁点之前的调用不在守卫内），到守卫释放点/最内层封闭块结束。
    const guardEnd = guardLifetimeEnd(code, lockIndex, scopeEnd);
    const guardWindow = code.slice(lockIndex, guardEnd);
    const lockLine = lineAt(src, lockIndex);

    for (const name of forbidden) {
      const needle = `self.${name}(`;
      let hit = guardWindow.indexOf(needle);
      while (hit !== -1) {
        violations.push({
          lockLine,
          kind: 'forbidden-call',
          detail: `self.${name}(`,
          callLine: lineAt(src, lockIndex + hit),
        });
        hit = guardWindow.indexOf(needle, hit + needle.length);
      }
    }

    for (const other of lockPositions) {
      if (other > lockIndex && other < guardEnd) {
        violations.push({
          lockLine,
          kind: 'nested-lock',
          detail: LOCK,
          callLine: lineAt(src, other),
        });
      }
    }
  }

  return violations;
}

/**
 * @param {string} filePath
 * @param {string} scanRoot
 * @param {string[]} forbidden
 * @returns {{displayPath:string, relPath:string, violations:object[]}}
 */
function scanFile(filePath, scanRoot, forbidden) {
  const src = fs.readFileSync(filePath, 'utf8');
  const repoRelative = path.relative(REPO_ROOT, filePath).split(path.sep).join('/');
  return {
    displayPath: repoRelative.startsWith('..') ? filePath : repoRelative,
    relPath: path.relative(scanRoot, filePath).split(path.sep).join('/'),
    violations: analyzeSource(src, forbidden),
  };
}

function listRustFiles(dir) {
  return fs
    .readdirSync(dir)
    .filter((name) => name.endsWith('.rs'))
    .sort()
    .map((name) => path.join(dir, name));
}

function isBlocking(relPath) {
  return BLOCKING_FILES.has(path.basename(relPath));
}

/**
 * 精确豁免键：file + 违规种类 + 被调用方法 + 加锁行号。
 * @param {{relPath:string, kind:string, detail:string, lockLine:number}} entry
 * @returns {string}
 */
function exemptionKey(entry) {
  return `${entry.relPath}|${entry.kind}|${entry.detail}|${entry.lockLine}`;
}

/**
 * 分级：死锁路径文件永远阻塞；命中精确豁免表的为 P1-SAFE（非阻塞）；其余一律阻塞。
 * @param {{relPath:string, kind:string, detail:string, lockLine:number}} entry
 * @returns {'ERROR'|'P1-SAFE'}
 */
function classify(entry) {
  if (isBlocking(entry.relPath)) return 'ERROR';
  return VERIFIED_SAFE.has(exemptionKey(entry)) ? 'P1-SAFE' : 'ERROR';
}

/**
 * 解析禁用集合：自动发现 ∪ 显式基线。
 *
 * 并集下限门禁：自动发现数 < 基线数 ⇒ 解析退化（门禁会变松）⇒ 直接 Exit 2。
 * @param {string} scanRoot
 * @returns {{forbidden:string[], discovered:string[], sites:Map<string,string[]>}}
 */
function resolveForbiddenSet(scanRoot) {
  const { names: discovered, sites } = discoverLockAcquiringMethods(scanRoot);
  if (discovered.length < BASELINE_FORBIDDEN.length) {
    console.error(
      `[lock-scope] SCRIPT-ERROR: auto-discovery degraded (discovered=${discovered.length} < ` +
        `baseline=${BASELINE_FORBIDDEN.length}); refusing to emit a verdict from a loosened gate. ` +
        `discovered=[${discovered.join(', ')}]`,
    );
    process.exit(2);
  }
  const forbidden = [...new Set([...BASELINE_FORBIDDEN, ...discovered])].sort();
  return { forbidden, discovered, sites };
}

/** 遍历扫描根下所有 .rs 文件并返回分级后的违规。 */
function collectViolations(scanRoot, forbidden) {
  if (!fs.existsSync(scanRoot)) {
    console.error(`scan root not found: ${scanRoot}`);
    process.exit(2);
  }
  const files = listRustFiles(scanRoot);
  const blocking = [];
  const verifiedSafe = [];
  for (const file of files) {
    const { displayPath, relPath, violations } = scanFile(file, scanRoot, forbidden);
    for (const violation of violations) {
      const entry = { displayPath, relPath, ...violation };
      if (classify(entry) === 'ERROR') blocking.push(entry);
      else verifiedSafe.push(entry);
    }
  }
  return { files, blocking, verifiedSafe };
}

function runSelfTest() {
  const failing = [];

  // ── 自动发现引擎自检（合成样例，独立于仓库状态）────────────────────────────────────
  const syntheticModule = `
impl Foo {
    fn hidden_lock_helper(&self) -> Option<Bar> {
        let guard = self.graph_runs.lock().unwrap();
        guard.load_run("x")
    }
    fn multi_line_signature(&self, a: u8) -> Result<(), String>
    where
        u8: Copy,
    {
        let _guard = self.graph_runs.lock().unwrap();
        Ok(())
    }
    fn no_lock(&self) -> u8 {
        1
    }
    fn mention_only(&self) -> String {
        "self.graph_runs.lock()".to_string()
    }
}
`;
  const syntheticNames = discoverInSource(syntheticModule).map((hit) => hit.name).sort();
  const expectedSynthetic = ['hidden_lock_helper', 'multi_line_signature'];
  if (JSON.stringify(syntheticNames) !== JSON.stringify(expectedSynthetic)) {
    failing.push(
      `discovery must find ${JSON.stringify(expectedSynthetic)} (multi-line signature included, ` +
        `string literal excluded), got ${JSON.stringify(syntheticNames)}`,
    );
  }

  // ── 并集下限门禁自检（解析退化必须 Exit 2 而不是静默变松）──────────────────────────
  if (!(0 < BASELINE_FORBIDDEN.length)) {
    failing.push('baseline forbidden list must be non-empty');
  }
  const degraded = discoverInSource('fn nothing(&self) {}').length;
  if (!(degraded < BASELINE_FORBIDDEN.length)) {
    failing.push('degradation predicate must trigger when discovery finds fewer than the baseline');
  }

  // ── 真实仓库：自动发现集合 + 用「新发现方法名」构造违规样例 ─────────────────────────
  const liveRoot = path.join(REPO_ROOT, DEFAULT_SCAN_ROOT);
  if (!fs.existsSync(liveRoot)) {
    failing.push(`live scan root missing, cannot validate auto-discovery: ${liveRoot}`);
  } else {
    const { names: liveDiscovered } = discoverLockAcquiringMethods(liveRoot);
    if (liveDiscovered.length < BASELINE_FORBIDDEN.length) {
      failing.push(
        `live auto-discovery must find >= ${BASELINE_FORBIDDEN.length} lock-acquiring methods, ` +
          `got ${liveDiscovered.length}`,
      );
    }
    const widened = liveDiscovered.filter((name) => !BASELINE_FORBIDDEN.includes(name));
    if (widened.length === 0) {
      failing.push('auto-discovery must widen the forbidden set beyond the 4-name baseline');
    } else {
      const newName = widened[0];
      const liveUnion = [...new Set([...BASELINE_FORBIDDEN, ...liveDiscovered])].sort();
      const wideningSample = `
fn widen(&self) {
    let guard = self.graph_runs.lock().unwrap();
    let _ = self.${newName}(query);
}
`;
      const withUnion = analyzeSource(wideningSample, liveUnion);
      const withBaselineOnly = analyzeSource(wideningSample, BASELINE_FORBIDDEN);
      if (withUnion.length !== 1 || withUnion[0].kind !== 'forbidden-call') {
        failing.push(
          `violation built from newly discovered method \`${newName}\` must be reported by the ` +
            `union set, got ${JSON.stringify(withUnion)}`,
        );
      }
      if (withBaselineOnly.length !== 0) {
        failing.push(
          `baseline-only set must MISS \`${newName}\` (proves the widening is not vacuous), ` +
            `got ${JSON.stringify(withBaselineOnly)}`,
        );
      }
    }
  }

  // 违规样例：加锁后二次加锁调用 → 必须报违规（证明守卫不是假测试）。
  const violating = `
fn bad(&self) {
    let store = self.graph_runs.lock().unwrap_or_else(|e| e.into_inner());
    let run = self.resolve_graph_run_for_retrieval(None, Some("s"));
    let _ = run;
}
`;
  const violatingHits = analyzeSource(violating, BASELINE_FORBIDDEN);
  if (violatingHits.length !== 1 || violatingHits[0].kind !== 'forbidden-call') {
    failing.push(
      `violating sample must report exactly 1 forbidden-call, got ${JSON.stringify(violatingHits)}`,
    );
  }
  if (violatingHits[0] && violatingHits[0].callLine !== 4) {
    failing.push(`violating sample call line must be 4, got ${violatingHits[0].callLine}`);
  }

  // 嵌套块内违规：证明括号级作用域判定真的生效。
  const nested = `
fn nested(&self) {
    let store = self.graph_runs.lock().unwrap();
    if store.list_runs().is_empty() {
        let _ = self.load_execution_checkpoint(query);
    }
}
`;
  const nestedHits = analyzeSource(nested, BASELINE_FORBIDDEN);
  if (nestedHits.length !== 1 || nestedHits[0].kind !== 'forbidden-call') {
    failing.push(`nested sample must report 1 forbidden-call, got ${JSON.stringify(nestedHits)}`);
  }

  // 同守卫生命周期内二次加锁 → 必须报 nested-lock（直接自死锁）。
  const doubleLock = `
fn twice(&self) {
    let store = self.graph_runs.lock().unwrap();
    let again = self.graph_runs.lock().unwrap();
    let _ = (store, again);
}
`;
  const doubleLockHits = analyzeSource(doubleLock, BASELINE_FORBIDDEN);
  if (doubleLockHits.length !== 1 || doubleLockHits[0].kind !== 'nested-lock') {
    failing.push(`double-lock sample must report 1 nested-lock, got ${JSON.stringify(doubleLockHits)}`);
  }

  // 合法样例：加锁前的调用 + 显式 drop 后的顺序加锁 → 必须 0 违规（防误报）。
  const clean = `
fn good(&self) {
    let run = self.resolve_graph_run_for_retrieval(None, Some("s"));
    let store = self.graph_runs.lock().unwrap();
    let _ = store.load_run("x");
    drop(store);
    let mut store = self.graph_runs.lock().unwrap();
    let _ = run;
    let _ = store;
}
`;
  const cleanHits = analyzeSource(clean, BASELINE_FORBIDDEN);
  if (cleanHits.length !== 0) {
    failing.push(`clean sample must report 0 violations, got ${JSON.stringify(cleanHits)}`);
  }

  // 注释/字符串内出现锁模式 → 必须 0 违规（防剥离缺陷导致的误报）。
  const commented = `
// self.graph_runs.lock() then self.resolve_graph_run_for_retrieval(None, None)
fn quiet(&self) {
    let note = "self.graph_runs.lock() self.load_graph_run(";
    let _ = note;
}
`;
  const commentedHits = analyzeSource(commented, BASELINE_FORBIDDEN);
  if (commentedHits.length !== 0) {
    failing.push(`comment/string sample must report 0 violations, got ${JSON.stringify(commentedHits)}`);
  }

  // 分级判定自检：证明门禁非恒真（未豁免违规必须阻塞、精确豁免项才放行）。
  const blockingSample = {
    relPath: 'history_commands.rs',
    kind: 'forbidden-call',
    detail: 'self.resolve_graph_run_for_retrieval(',
    lockLine: 433,
  };
  if (classify(blockingSample) !== 'ERROR') {
    failing.push('deadlock-path violation must classify as ERROR');
  }
  const unannotatedOtherFile = {
    relPath: 'some_new_file.rs',
    kind: 'forbidden-call',
    detail: 'self.load_graph_run(',
    lockLine: 10,
  };
  if (classify(unannotatedOtherFile) !== 'ERROR') {
    failing.push('unannotated violation in any file must classify as ERROR (gate must not be vacuous)');
  }
  const annotated = {
    relPath: 'mod.rs',
    kind: 'forbidden-call',
    detail: 'self.load_execution_checkpoint(',
    lockLine: 2040,
  };
  if (classify(annotated) !== 'P1-SAFE') {
    failing.push('Lead-verified-safe violation must classify as P1-SAFE');
  }
  const annotatedWrongLine = { ...annotated, lockLine: 2041 };
  if (classify(annotatedWrongLine) !== 'ERROR') {
    failing.push('exemption must be pinned to the exact lock line (no blanket file exemption)');
  }
  const annotatedWrongDetail = { ...annotated, detail: 'self.load_graph_run(' };
  if (classify(annotatedWrongDetail) !== 'ERROR') {
    failing.push('exemption must be pinned to the exact called method');
  }

  if (failing.length > 0) {
    console.error('[self-test] FAILED');
    for (const line of failing) console.error(`  - ${line}`);
    return 2;
  }
  console.log(
    '[self-test] PASS: 自动发现引擎（含多行签名/字符串排除）、并集下限门禁、新发现方法名违规样例、' +
      '合法/注释/顺序加锁零误报、豁免表精确到行号/方法 —— 全部通过',
  );
  return 0;
}

function main() {
  const args = process.argv.slice(2);
  if (args.includes('--self-test')) {
    process.exit(runSelfTest());
  }
  const strict = args.includes('--strict');
  const rootArgs = args.filter((a) => a.startsWith('--root='));
  const unknown = args.filter((a) => a !== '--strict' && !a.startsWith('--root='));
  if (unknown.length > 0 || rootArgs.length > 1) {
    if (unknown.length > 0) console.error(`unknown argument(s): ${unknown.join(', ')}`);
    if (rootArgs.length > 1) console.error('--root may be given at most once');
    console.error(
      'usage: node scripts/check-graph-run-lock-scope.mjs [--strict] [--root=<dir>] [--self-test]',
    );
    process.exit(2);
  }
  const scanRoot =
    rootArgs.length === 1
      ? path.resolve(process.cwd(), rootArgs[0].slice('--root='.length))
      : path.join(REPO_ROOT, DEFAULT_SCAN_ROOT);

  const { forbidden, discovered } = resolveForbiddenSet(scanRoot);
  const { files, blocking: classified, verifiedSafe } = collectViolations(scanRoot, forbidden);
  const blocking = strict ? [...classified, ...verifiedSafe] : classified;
  const registered = strict ? [] : verifiedSafe;

  console.log(
    `[lock-scope] scanned ${files.length} file(s) under ${scanRoot} (mode=${strict ? 'strict' : 'blocking-only'})`,
  );
  console.log(
    `[lock-scope] forbidden set: ${forbidden.length} name(s) = baseline ${BASELINE_FORBIDDEN.length} ∪ discovered ${discovered.length}`,
  );
  console.log(`[lock-scope] auto-discovered (${discovered.length}): ${discovered.join(', ')}`);

  for (const entry of blocking) {
    console.error(
      `[ERROR] ${entry.displayPath}:${entry.callLine} ${entry.kind}: \`${entry.detail}\` inside \`${LOCK}\` scope opened at line ${entry.lockLine}`,
    );
  }
  for (const entry of registered) {
    console.warn(
      `[P1-SAFE] ${entry.displayPath}:${entry.callLine} ${entry.kind}: \`${entry.detail}\` inside \`${LOCK}\` scope opened at line ${entry.lockLine}` +
        `\n          reason: ${VERIFIED_SAFE.get(exemptionKey(entry))}` +
        `\n          (registered, non-blocking; use --strict to escalate)`,
    );
  }

  // 豁免表自审计：未被命中的豁免项告警，防止豁免腐化成永久免死金牌。
  const matchedKeys = new Set(verifiedSafe.map(exemptionKey));
  for (const key of VERIFIED_SAFE.keys()) {
    if (!matchedKeys.has(key)) {
      console.warn(`[P1-SAFE-STALE] exemption entry no longer matches any violation: ${key}`);
    }
  }

  if (blocking.length > 0) {
    console.error(
      `[lock-scope] RED: ${blocking.length} blocking violation(s), ${registered.length} registered P1-SAFE violation(s)`,
    );
    return 1;
  }
  console.log(
    `[lock-scope] GREEN: 0 blocking violation(s), ${registered.length} registered P1-SAFE violation(s)`,
  );
  return 0;
}

process.exit(main());
