# Agent Note: runtime store 未覆盖 action 的等价测试口径（Wave 2）

Status: implemented

## Problem

Wave 1 拆解后，`concurrency-lifecycle.spec.ts` 用「已覆盖 57 ∪ 未覆盖 40 == 全部 97 action」
的机械对账冻结了覆盖自评，但 40 个未覆盖 action 的 reason 大多是「需宿主 IPC / 启动面 / UI 驱动」。
这些理由解释的是「为什么不进 6 类 golden 场景」，并未回答「它们的行为是否等价」——
golden 只冻结事件走完后的全量快照，不覆盖 action 的入参归一、宿主命令名与参数、
短路条件与失败降级路径。结果是这批 action 的契约只靠人工 review，任何一处参数拼错或
短路条件写反都不会变红。

## Decision

Wave 2 采用**两层判据**，按成本从低到高落地：

1. **等价测试层**（本波次）：对每个未覆盖 action 冻结「宿主命令边界 + 状态转移」——
   用 `FakeBackend.route/routeValue` 注入宿主返回，断言 `command` 名、`args` 形状、
   store 落值、短路与失败降级。不与 golden 重复断言同一快照。
2. **golden 层**（维持现状）：仅对「事件驱动 + 全量快照」的 6 类场景生效，
   新场景才有必要扩夹具。

分层落地节奏：Wave 2a（本 PR）覆盖 D8 能力目录 6 条 + 用量治理 2 条 + 停止链 2 条 +
会话通知 1 条 + 待发送附件 3 条 = **14 条**；剩余 26 条按 D1/D2 目录写操作与宿主分支
留在 2b/2c。每落一批同步维护 `COVERED_ACTIONS` / `UNCOVERED_ACTIONS` 两份清单与计数断言，
保证「已覆盖 ∪ 未覆盖 == 97」不靠自评。

测试替身边界：只替换**读字节**一层（`readFileAsArrayBuffer`），因为 jsdom 的 `FileReader`
在假定时器下不落定、且 `File` 缺 `text()/arrayBuffer()`；附件规则（类型解析、大小/图片上限、
去重、魔数嗅探、宿主导入）保持真实实现，避免把被测逻辑一起桩掉。

## Alternatives considered

1. **把 40 条全塞进 6 类 golden 场景**：需要为每条造事件序列与全量夹具，成本是等价测试的
   数倍，且 golden 的判据（全量快照）对「命令名/参数是否正确」并不敏感，投入产出不成立。
2. **只断言「action 可调用不抛错」**：这类测试不会红，等于把覆盖自评变成形式主义；
   `contract-surface.spec.ts` 已经覆盖了类型面可调用性，再写一遍是重复计量。
3. **mock 整个 `@/lib/runtime/file-attachments`**：会把类型解析、上限、去重、嗅探一并桩掉，
   被测逻辑与替身边界混淆；只替换 `readFileAsArrayBuffer` 保留了规则层真实性。
4. **一次性补齐 40 条再提 PR**：单个 PR 会跨 D1/D2/D8/治理/附件/停止链六个域，
   review 无法按住主题，且任何一条红都阻塞其余 39 条。按域切 3 个 PR 换取可回滚粒度。

## Consequences

- 本 PR 让 14 条 action 从「靠 review」变为「机器可判」：`verify` 会跑
  `equivalence-wave2a.spec.ts`（20 用例）与更新后的覆盖对账（71 / 26）。
- 未覆盖清单仍保留 26 条，`reason` 字段继续作为「为什么还没测」的显式登记，
  禁止为了清空清单而写无判据测试。
- 2b/2c 的边界已在 ADR 内写明：D1/D2 目录写操作与宿主分支优先，纯 UI 驱动面最后。
