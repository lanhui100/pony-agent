# TS/Rust 契约快照（W57-A）

前端 `src/types/runtime.ts` 与 Rust `crates/pony-agent-core/src/agent/runtime/types.rs` 之间的
线缆契约，用**机械门禁**代替人工评审。

## 为什么需要

wave-55 已实证一次真实漂移：Rust `TurnStreamEvent.step` 在 TS 声明里缺失，前端静默丢字段，
只能靠人工评审发现。任何一侧改字段、加字段都会重现这类静默丢失。

## 组成

| 文件 | 作用 | 写域 |
| --- | --- | --- |
| `docs/contracts/ts-contract.snapshot.json` | Rust DTO 的**真实 serde 线缆键**快照（`serde_json::to_value` 产出，禁止手抄/手改） | 由 Rust 测试生成 |
| `crates/pony-agent-core/tests/ts_contract_snapshot.rs` | 生成快照；并断言「实现当前 serde 键集合 == 已提交快照」（防一侧改字段忘同步） | `crates/pony-agent-core/tests/` |
| `scripts/check-ts-contract.mjs` | 比对快照 vs TS 声明，**只报「Rust 有、TS 无」**（漏声明） | `scripts/` |

## 用法

```bash
# 1) 日常门禁：只比 TS 声明（快，零依赖）
npm run contract:check

# 2) Rust 侧改了 DTO（字段/序列化属性）后：先重生成快照，再复核 TS
UPDATE_CONTRACT_SNAPSHOT=1 cargo test -p pony-agent-core --test ts_contract_snapshot
npm run contract:check
```

`npm run verify` 已包含 `npm run contract:check`（在 `build`/typecheck 之后）。

## 判定口径

- **只查「Rust 有、TS 无」**：TS 允许存在本地派生字段（TS 多出来的键不报错）。
- 缺失即 `exit 1`，并逐条打印 `类型（文件）缺少 Rust 已下发的字段：…`。
- 修复方向永远是**补 TS 声明**（camelCase，与 serde 线缆一致），不是改快照；若 Rust 字段
  本就不该下发，应在实现侧用 `#[serde(skip_serializing_if = ...)]` 或移除字段。

## 当前覆盖

| 类型 | Rust 源 | TS 声明 |
| --- | --- | --- |
| `TurnStreamEvent` | `runtime/types.rs` | `src/types/runtime.ts` |
| `TurnResult` | `runtime/types.rs` | `src/types/runtime.ts` |

`TurnInput` 暂未纳入：Rust 侧只有 `#[derive(Deserialize)]`（无 `Serialize`），无法用
`serde_json::to_value` 取真实键名；纳入前需先在实现侧补 `Serialize`（属实现写域）。

## 扩展新类型

1. 在 `ts_contract_snapshot.rs` 里加一个「代表样本」构造函数（注意 `skip_serializing_if`
   的字段必须给 `Some`，否则键不出现），并登记 `rustSource`；
2. 在快照的 `tsDeclarations` 里登记 TS 文件路径；
3. `UPDATE_CONTRACT_SNAPSHOT=1 cargo test -p pony-agent-core --test ts_contract_snapshot`
   重生成，再跑 `npm run contract:check`。
