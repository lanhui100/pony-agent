# Wave-17 协议特派员（L2-P）交付收据

- 任务：`task-1`（protocol：NFR 基准表 + IME 守卫接缝 + 空桩纯度工具）
- 基线 commit：`5d7d691`（与契约 `.dev-team/contracts/wave-17-ime-and-numeric-jitter.md` 声明一致）
- 落盘时刻：`2026-10-16T13:18:42+08:00`
- 契约真源（只读）：`.dev-team/contracts/wave-17-ime-and-numeric-jitter.md` §2 C1 / §3 写域
- 门禁标注：**`[GATE EXTENDED: PROJECT-LINT(16-key) + WAVE-LINT(5-key)]`**
  （另：`src/lib/ime.ts` 空桩纯度门禁为 `[GATE EXTENDED: WAVE-STUB-LINT(TS-equivalent)]`）

---

## 1. 交付物清单（物理落盘）

| 交付物 | 行数 | sha256（前 16） | 说明 |
| :--- | ---: | :--- | :--- |
| `.dev-team/nfr-baseline-wave17.json` | 23 | `bf1e0bb8d2d90553` | 本波 5 键 NFR 基准（**旁路文件**，不动既有 16 键基准） |
| `.dev-team/tools/validate-nfr-baseline.mjs` | 307 | `358a3649b866386b` | 本波专用 5 键严格校验器 + `--self-test` 反例注入 |
| `.dev-team/tools/validate-stub-purity.mjs` | 149 | `8fa3bc4ae1973784` | TS 侧空桩纯度机械校验器 |
| `src/lib/ime.ts` | 26 | `de9e784018154730` | **纯空桩**：JSDoc 契约 + 签名 + `throw new Error("NotImplemented")` |
| `.dev-team/wave17-protocol-receipt.md` | 本文件 | — | 真实 stdout + EXIT 码 |

`src/lib/ime.ts` 空桩物理原文（末 3 行，函数体有效语句 **1/1**）：

```ts
export function isImeComposingEvent(_event: Event | null | undefined): boolean {
  throw new Error("NotImplemented");
}
```

---

## 2. 门禁命令：真实 stdout + EXIT 码

### CMD1 — 本波 5 键基准校验（`WAVE-LINT`）

```console
$ node .dev-team/tools/validate-nfr-baseline.mjs
[nfr-lint] target = .dev-team/nfr-baseline-wave17.json
[nfr-lint] root keys (5) = external_call_timeout_ms, retry, concurrency_lock, logging, resource_release
[nfr-lint] external_call_timeout_ms = 3000 (max 3000)
[nfr-lint] logging.format = "json", logging.trace_id = true
[nfr-lint] PASS: 5 键齐备 / 无未知键 / 类型与阈值全部满足 / applicable 诚实性通过
EXIT=0
```

### CMD1b — 校验器非恒真自证（`--self-test`，8 组反例注入）

```console
$ node .dev-team/tools/validate-nfr-baseline.mjs --self-test
SELFTEST[valid-fixture] PASS (被接受，符合预期)
SELFTEST[未知根键] PASS (被拒绝: 未知根键 "extra_key"（仅允许：external_call_timeout_ms, retry, concurrency_lock, logging, resource_release）)
SELFTEST[缺少必填键] PASS (被拒绝: 缺少必填键 "retry")
SELFTEST[超时越界(3001)] PASS (被拒绝: external_call_timeout_ms: 必须满足 1 <= v <= 3000（实际 3001）)
SELFTEST[超时非整数] PASS (被拒绝: external_call_timeout_ms: 必须为整数（实际 string）)
SELFTEST[logging.format 非 json] PASS (被拒绝: logging.format: 必须严格等于 "json"（实际 "text"）)
SELFTEST[logging.trace_id 非 true] PASS (被拒绝: logging.trace_id: 必须严格等于 true（实际 false）)
SELFTEST[applicable:false 却无 reason] PASS (被拒绝: retry.reason: applicable=false 时必须给出非空 reason（诚实标注不适用），实际 undefined)
SELFTEST[resource_release 空数组] PASS (被拒绝: resource_release: 不得为空数组（须显式声明适用性）)
SELFTEST SUMMARY: 9/9 通过；反例 8 组
SELFTEST RESULT: OK (校验器非恒真)
EXIT=0
```

### CMD2 — 空桩纯度校验（独立脚本，`WAVE-STUB-LINT`）

```console
$ node .dev-team/tools/validate-stub-purity.mjs
[stub-lint] target = src/lib/ime.ts
[stub-lint] export = isImeComposingEvent(Event | null | undefined): boolean
[stub-lint] body = throw new Error("NotImplemented") [1/1 语句]
[stub-lint] PASS: 纯空桩（无实现逻辑、无控制流、无属性访问），总行数 26
EXIT=0
```

### CMD3 — 空桩纯度校验（`node -e` 内联，task-1 指定形态）

```console
$ node -e '<inline stub purity check>'
STUB_PURITY_OK export=isImeComposingEvent body_lines=1 body=throw new Error("NotImplemented");
EXIT=0
```

### CMD4 — 空桩落盘时的类型门禁

```console
$ npx vue-tsc --noEmit
（无 stdout 输出 = 0 诊断）
EXIT=0
```

### CMD5 — 既有 16 键冻结基准回归（`PROJECT-LINT`，证明未被本波破坏）

```console
$ python3 .dev-team/nfr-lint.py
[nfr-lint] jsonschema=4.10.3
[nfr-lint] schema=/home/dm/pony-agent/.dev-team/nfr-baseline.schema.json
[nfr-lint] instance=/home/dm/pony-agent/.dev-team/nfr-baseline.json
[nfr-lint] ok: schema 校验通过；未知字段 0 个；lock_scope_bounded=True lock_reentrancy_forbidden=True lock_acquisition_timeout_ms=2000 deadlock_watchdog_timeout_ms=5000 lock_forbidden_calls=4 logging_scope='new_structured_logging_surface_only' logging_legacy_eprintln_exempt=True logging_debt_ref='PA-119' logging_eprintln_exempt_covers_new_lines=1 logging_compliance_claimed=False
[nfr-lint] NOTE(logging): logging{format=json,trace_id_required} 当前**无代码级检查点**（全仓库无结构化日志面），属 PA-119 债务；本波**不主张该 NFR 已满足**，仅主张「已显式登记债务 + 有到期条款 + 有恒真防护（logging_compliance_claimed=false）」。禁止把该字段读作恒真放行。
[nfr-lint] RESULT: PASS
EXIT=0
```

### CMD6 — 只读边界自证（既有基准零字节改动）

```console
$ git diff --name-only -- .dev-team/nfr-baseline.json
stdout=[]
EXIT=0
```

---

## 3. 关键裁定与偏离登记（诚实标注）

1. **NFR 基准落盘路径（Lead 裁定 B）**：`.dev-team/nfr-baseline.json` 为 PA-118 冻结的 16 键基准，被项目自有 `.dev-team/nfr-lint.py`、`docs/release/2026-10-14-pa118-deadlock-fix/`、`docs/decisions/0024`、两个 Rust 死锁测试注释共同引用。**本波未覆盖该文件**（CMD6 证明 `git diff` 为空），改用旁路文件 `.dev-team/nfr-baseline-wave17.json`（沿用 wave-15 `nfr-baseline-wave15.json` 先例）。
2. **`applicable:false` 诚实性**：本波为纯前端渲染层改动（C1 事件守卫 + C2 CSS 字体变体），无外部 IO、无并发共享态、无资源句柄、无新增日志面。`retry` / `concurrency_lock` / `logging` / `resource_release[0]` 均以 `applicable:false` + `reason` 显式标注，**未编造** `max_attempts` / `backoff_cap_ms` / `primitive` / `scope` / `handle` / `release` 等不存在的机制；`resource_release` 保持 `array` 类型（元素为诚实标注对象），`logging` 保留 `format:"json"` + `trace_id:true` 仅作全局基准声明，并在 `reason` 中声明本波不主张其已满足。
3. **形参绑定名 `_event`（唯一签名层偏离，需 Lead 知悉）**：`tsconfig.json` 同时开启 `noUnusedParameters:true`。空桩函数体按纯度契约只允许 `throw new Error("NotImplemented")`，形参必然未被读取，TS 会报 `TS6133: 'event' is declared but its value is never read` → 破坏 CMD4 的 EXIT=0。实测三种写法的物理结果：`event` + 仅 throw → `TS6133`（EXIT=2）；`event` + `void event;` + throw → EXIT=0 但**违反「函数体只有 throw」纯度硬约束**；`_event` + 仅 throw → EXIT=0 **且纯度成立**。故取 `_event`。**TS 形参绑定名不属于函数类型签名**，类型签名 `(Event | null | undefined) => boolean` 与契约逐字一致，调用方零影响；executor-a 实现时可直接改回 `event`。
4. **工具可用性更正（Lead 已裁定）**：项目**已自带** `.dev-team/nfr-lint.py`（18,536 B，16 键 Draft-07 + 死锁/日志范围硬约束）与 `.dev-team/check-lock-discipline.py`。二者职责与本波 5 键严格校验不同，**两条门禁均已实际运行且 EXIT=0**（CMD1 / CMD5）。标注为 `[GATE EXTENDED]`，非 TOOL-MISSING。
5. **沙箱参考件差异（已取证，非本波缺陷）**：`/tmp/dev-team-sandbox/scripts/dev-nfr-lint.py`（skill 沙箱参考件，非项目文件）校验的 5 键与 task-1 完全同名，但其 logging 分支要求 `logging.required_fields` 数组含 `"trace_id"`，而 task-1 冻结契约为 `logging.trace_id === true`（键名不同）。实测：`SANDBOX_REF_EXIT=1`，报 `NFR LINT ERROR: logging must specify format='json' and require 'trace_id'`。**处置**：以 task-1 冻结契约为准（不新增 `required_fields` 以免「为凑 schema」偏离），该差异登记为工具侧命名差异；`/tmp/dev-team-sandbox/scripts/dev-stub-lint.py` 基于 Python `ast` 仅解析 `.py`，对 `src/lib/ime.ts` 物理不可用，故 TS 侧纯度门禁为等价自建实现（CMD2/CMD3）。

---

## 4. 写域与边界自证

- 本任务实际写入（`git status --porcelain` 只读观测）：
  ```console
  $ git status --porcelain -- .dev-team/nfr-baseline-wave17.json .dev-team/tools .dev-team/wave17-protocol-receipt.md src/lib/ime.ts .dev-team/nfr-baseline.json
  ?? .dev-team/nfr-baseline-wave17.json
  ?? .dev-team/tools/
  ?? src/lib/ime.ts
  ```
- 未触碰：`.dev-team/contracts/**`（只读引用）、`.dev-team/nfr-baseline.json`、`.dev-team/nfr-lint.py`、任何 `src/components/**`、任何 `tests/**`。
- **零 git 写操作**：未执行 `git add` / `git commit` / `git checkout` / `git stash`（git 写主为 Lead）；上述 `git status` / `git diff` 均为只读观测。
- 空桩未含任何实现逻辑，`src/lib/ime.ts` 的实现归属 executor-a（契约 §3 task-3）。
