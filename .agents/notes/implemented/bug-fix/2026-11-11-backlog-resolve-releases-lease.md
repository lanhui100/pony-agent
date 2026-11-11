# Agent Note: 闭环释放任务租约（B081：`resolve` 后 `.locks/<ID>.lock` 残留）

Status: implemented

## Problem

`resolve()`（库与 CLI `node scripts/backlog.mjs resolve <ID> --evidence ...` 同一实现路径）只做三件事：
归档写入、活跃池剔除、推进高水位；**不碰租约**。于是任务闭环后 `backlog/.locks/<ID>.lock` 原样留下，
现场实证：B069 闭环后锁文件仍在，需手工调用 `releaseLease` 才清掉。

三个后果：

1. **假占用**：残留锁在 TTL（`DEFAULT_LEASE_TTL_MS` 默认 2h）内对任何会话都是
   「该 ID 已被持有」——`acquireLease` 直接抛「已被 X 持有，不得并发编辑」。而该任务已归档，
   根本不该再被租用。
2. **门禁语义腐化**：B069 确立的判据是「`.locks/` 目录存在 ⇒ 走严格支，缺锁即判手改」。
   残留锁让目录常驻，严格支因此永久生效；真正的手改信号被这层噪音稀释（无 `in_progress` 时不报错，
   所以也永远不会有人注意到分支已切死）。
3. **不可见**：空目录/锁文件不入仓（`backlog/.locks/` 已 gitignore），只能靠人肉发现，
   没有任何门禁会报——`doctor` 原本只列活跃任务的租约，残留锁根本不在视野里。

## Decision

1. **`resolve()` 在归档落盘后释放该任务租约**，把回收结果放进返回值
   （`{ row, archiveFile, lease }`，`lease = { released, previous? }`）。CLI 据此打印
   `租约已释放（原持有者 @owner）`。
2. **释放顺序排在归档/活跃池/高水位写完之后**。理由是失败方向：先清锁再归档，中途失败会留下
   「任务仍在活跃池、却没人持锁」的伪手改态（正好污染 B069 的严格支判据）；当前顺序最坏只是
   「多留一个可 reap 的锁」，与修复前同阶，方向安全。
3. **闭环是终态，释放不过滤 owner**。`resolve` 本身已是带 evidence 的显式裁决动作，
   任务即将离开活跃池——此时坚持「只有持有者能释放」只会把残留锁固化下来。回收的
   `lease.previous.owner` 会打印出来，跨会话误闭环可见而非静默。
4. **`doctor` 新增残留锁检查**（同域补一条机器可查的看门狗）：`.locks/<ID>.lock` 存在但活跃池
   已无该 id ⇒ 记 `issue` 并把自愈命令写进文案（`reap` 回收 / `release <ID>`）。
   这是本类缺陷唯一的**事后**发现点——否则只能等 TTL 自然过期。

## Alternatives considered

- **只在 CLI `resolve` 分支调 `releaseLease`**：否决——库与 CLI 是同一路径，但 `resolve()` 也被
  测试与脚本直接 import；把状态收尾放在 CLI 层会让「库调用」成为漏网面，正是本缺陷的成因之一
  （状态迁移写全了，收尾漏了）。
- **`release` 前显式 `--owner` 校验**：否决——见决策 3，闭环语义上是撤销任务而非撤销某会话的占用；
  且 CLI `resolve` 本就不带 owner，强制校验等于要求调用方先查锁文件所有者，把复杂度推给用户。
- **`resolve` 顺手 `rmdir` 空的 `.locks/` 目录**：否决——目录存在是 B069「本工作区是否在用租约」的
  唯一可观测信号。删掉空目录会让刚闭环完的工作区退回 note 支，反而**削弱**同域的手改检测；
  空目录本身不入仓、不影响 `git status`，留着无成本。
- **残留锁在 `resolve` 里做严格清理失败即抛错**：否决——清理发生在归档已经落盘之后，此时抛错等于
  让一个已完成的闭环"看起来失败"，而重放又会遇到「ID 不在活跃池」。清理失败只能是软失败。
- **只加 `doctor` 检查、不修 `resolve`**：否决——把可自愈的缺陷降级成人工巡检项；
  B081 的成因就是"终态收尾没人负责"，看门狗是第二道，不是第一道。

## Consequences

- **行为变更（面向契约）**：`resolve()` 返回值新增 `lease` 字段（向后兼容，多一个键）；
  副作用新增「删除本任务锁文件」。CI/其它脚本若 import `resolve` 无需改动。
- **`doctor` 可能转红**：存在残留锁时 `doctor` exit 1（此前恒绿）。这是有意的——文案自带自愈命令，
  且 `doctor` 不在 CI 门禁链（`ponygo-governance.yml` 只跑 `lint`），不会把历史遗留变成红构建。
- **本工作区实测**：`start B081` 建立锁 → 修复后 `resolve B081` 归档，`backlog/.locks/` 回到空目录
  （锁文件消失），`doctor` 无「残留租约」issue。
- **测试**：`tests/backlog-cli.spec.ts` 由 26 例增至 **31 例全绿**，新增 5 例覆盖
  库级释放 / 无租约幂等 / 他人持有仍释放 / `doctor` 残留锁正反例 / CLI 全链路
  （`start` 建锁 → `resolve` 清锁并打印回收信息）。
- **全量单测的两个红与本单无关（诚实披露）**：`npm run test:unit` 实测
  152 passed / 2 failed，失败面是 `tests/check-secrets.spec.ts`（`check-secrets --all`
  在 `.dev-team/verify-b064b-1794391769.json` 里命中两条源文件 sha256，该文件由
  `a5ee7681` 入仓）与其下游 meta 用例 `v10-baseline-freeze` V10-3（实跑全量单测 exit 0）。
  两者在本次改动前即红（改动面只有 `scripts/backlog.mjs` + 本 spec），已登记为 **B082**；
  `tests/update-signed-store.spec.ts` 在整轮并发下偶发失败，单跑 12 例全绿，属既有 flaky。
- **与 `docs/observability.yaml` 无冲突**：不改对外接口、网络 IO、后台任务、LLM 调用或 UI（纯本地 CLI）。
- **未覆盖（留给 review）**：会话在 `start` 之后崩溃（未 `resolve`）仍会留锁，属 `reap` 的职责面，
  本次未改动 `reapStaleLeases` 的 TTL 判据。
