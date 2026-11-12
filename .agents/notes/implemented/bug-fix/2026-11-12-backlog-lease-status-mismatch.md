# Agent Note: 租约-状态互斥判据（补手改状态降级与 start 中断的漏检）

Status: implemented

## Problem

独立验收（2026-11-12）在临时账本上实测出两条门禁漏检，二者同源：

1. **手改状态降级逃检**：`start` 后把记录 `status` 手改成 `todo`，`lint` 与 `doctor` 均 exit 0，
   pre-commit 端到端放行。根因是 `leaseConsistency` 只遍历 `in_progress` 记录，而
   `validateRecord` 只做枚举与条件必填校验、没有迁移历史可判——「记录说 todo、锁说 in_progress」
   这个方向没有任何判据看守。
2. **`start` 失败泄漏租约**：`start <id> --owner X`（不带 `--paths`）时先用 `O_EXCL` 取锁、
   后 `update` 校验失败抛错，锁未回滚。结果是记录仍 `todo`、`.locks/<ID>.lock` 已存在：
   `doctor` 的孤儿判定只看「活跃池无此 id」，不覆盖「id 在活跃池但状态不是 in_progress」，
   于是 `doctor` 报 ok，而其它 owner 再 `start` 会被这道幽灵锁挡住（等 2h TTL 或手工 reap）。

## Decision

1. `leaseConsistency` 增加**反向**判定：本工作区存在 `.locks/` 且记录持锁、但 `status ≠ in_progress`
   → 判 `problem`（文案含「存在租约锁但记录状态」，并提示 `reap --id <ID> --minutes 0` 只清该锁或手动删锁文件）。
   干净克隆 `.locks/` 不存在时锁恒为 null，判定不触发——不引入「干净环境恒红」。
2. `reap` 增加 `--id <ID>` 过滤：修复建议从「全局 reap」收窄为「只清该任务」，避免并发 lane 下
   误清其它会话的锁（实测全局 `reap --minutes 0` 会把 B056/B057 一起回收）。`--id` 缺值 / 空值 /
   不合 `^B\d{3,}$` 一律 exit 2 fail-closed，**不回落为全局清理**；不带 `--id` 仍清达到阈值的全部租约并打印范围。
3. 该判定同时覆盖两类现场：手改状态降级、`start` 取锁后中断。它不替代 `start` 的失败回滚
   （回滚仍未做，记于 `backlog:B108`），但让泄漏在提交前可见、可恢复。
4. 负样本补齐：`backlog-gate.spec.sh` 增 N7（锁在、记录 todo → 必须拒），spec 由 N1–N6 扩为 N1–N7。
5. 文档口径纠偏（本次自造的两处过度承诺）：门禁文案不再声称 lint 校验「状态机」（实际只有状态枚举 +
   条件必填），改为「状态字段合法性 + 条件必填 + 租约-状态互斥」；契约补上「缺锁判定需新鲜
   `heartbeat_at`」这一前提（无 heartbeat 时静默跳过，属已知 fail-open，记于 `backlog:B107`）。

## Alternatives considered

- **在 `validateRecord` 里做状态迁移历史判定**：记录里没有前态，单靠一行 JSON 无法区分「CLI 合法降级」
  与「手改」；要判迁移必须引入额外状态字段（如 `prev_status`），等于改记录契约并重写迁移 →
  超出最小闭包。
- **只修 `start` 回滚、不加反向判定**：回滚只堵「起点泄漏」，手改降级依然全绿；漏检方向没变 → 落选。
- **只在 `doctor` 加判定、`lint` 不动**：pre-commit 与 `verify` 走 gate（lint + doctor），看似都覆盖；
  但 `node scripts/backlog.mjs lint` 仍是多处引用的单点命令，漏检会以「lint 绿」被误读 → 放同一实现更稳。
- **`.locks/` 不存在时也判**：干净克隆/CI 无锁，会把全部 `todo` 记录误报；B069 已裁决该信号在干净环境
  不可判别 → 落选。

## Consequences

- `scripts/backlog.mjs::leaseConsistency` 行为变更：持锁记录非 `in_progress` 时 `lint`/`doctor` 非零；
  修复动作是 `reap --id <ID> --minutes 0` 只清该锁，或手动删除 `.locks/<ID>.lock`（该状态下
  `release` 不可用：它会先删锁、再以「非法跃迁 todo → todo」exit 1，副作用与失败混在一起）。
  已验证 D1 夹具 `lint` exit 1、D2 泄漏夹具同样 exit 1。
- spec 自检 N1–N7 与 H（pre-commit 接线：门在场放行 / 门缺失 fail-closed）全过；
  `tests/backlog-cli.spec.ts` 增 3 例（锁-状态错配、reap `--id` 过滤、reap `--id` 参数 fail-closed），
  共 34 例全绿，既有用例语义不变。
- **提交后复核收据**（HEAD 5e4cb558 之后新增第二次提交）：`backlog-gate` 0 / `backlog-gate.spec` 0 /
  `adr:check` 0 / `ci-verify` 0 / `upgrade --check` 0 / vitest 34 passed。B106 归档证据里的
  「31 passed」是提交前时点回执，最终以本条为准。
- 仍未修的已知缺口：`start` 取锁失败的原子回滚（B108）、缺 `heartbeat_at` 时的静默跳过（B107）。
- 诚实边界：这道判据只能证明「锁与状态不矛盾」，不能证明「状态是经 CLI 改的」；迁移历史正确性靠 review。
