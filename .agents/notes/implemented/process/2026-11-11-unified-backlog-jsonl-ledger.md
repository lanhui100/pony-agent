# Agent Note: 待办与问题治理统一到 backlog/tasks.jsonl 单一账本

Status: implemented

## Problem

仓库曾同时存在三套任务载体，事实重复且互相漂移：

1. `backlog/backlog.md`（59KB、B001–B055 已被真实使用，闭环提交形如 `docs(backlog): close B050-B054`）；
2. `.agents/skills/pa-issue-triage/` + `scripts/pa-issue-triage.mjs`（会话异常捕获、指纹去重、`current-plan.json` 断点、`archive/resolved-*.jsonl`）；
3. `.agents/backlog/tasks.jsonl`（迁移期临时产物，未纳入版本控制）。

Markdown 不适合作多任务状态流转与机械校验的载体：无法逐条 schema 校验、状态靠改标题、并发编辑必冲突、diff 噪声大。`pa-issue-triage` 的 `recordIssue` 用 `issues.length + 1` 生成 ID，归档后序号断层导致碰撞（实测又造出一个 `ISSUE-20261111-008`），且单文件 `current-plan.json` 断点既无法多会话并行，也无法与真实进度对账。

该方向先后两版方案被判不通过（v1、v2 各三路独立对抗审查全 FAIL：36 findings/22 BLOCKING、23 BLOCKING），失败点集中在「新建未跟踪的第 4 个真源」「ID 命名空间撞车」「用 `git log` 解析 close 提交导致自锁与 fail-open」「`closing_commit` 自指不可满足」「迁移无可执行顺序」。

## Decision

1. **单一 JSONL 真源**：`backlog/tasks.jsonl`（一行一任务、git-tracked）是唯一活账本；归档为 `backlog/archive/tasks-<YYYY>.jsonl`；ID 高水位为 `backlog/.next-id`。
2. **账本作用域 = 仓库**：`scripts/backlog.mjs::resolveLedgerRoot` 用 `git rev-parse --git-common-dir` 的父目录定位账本，使同一仓库的 6 个 worktree（含 detached/临时树）读写同一份，杜绝任务跨工作树混淆；`PONY_BACKLOG_ROOT` 仅用于测试注入。
3. **行级强校验**（`validateRecord`）：必填 `id`(`^B\d{3,}$`)、`category`(`Defect|Task|Debt`)、`priority`(`P0..P3`)、`status`(`todo|in_progress|done`)、`title`、`created_at`、`metadata.fingerprint`；未知顶层字段与未知 metadata 字段一律拒绝；`triage_context` 上限 8KiB；`description` 上限 2000 字符。
4. **状态机机械可判**：只允许 `todo→in_progress→done` 与 `in_progress→todo`；`in_progress` 必填 `owner` + `touched_paths`（≥1）；`done` 必填 `evidence` + `closed_at`，且必须经 `resolve` 写入并归档——活跃池残留 `done` 由 `lint` 判违规。
5. **ID 单调分配**：`.next-id` 存「下一候选编号」，`allocateId` 取 `max(高水位, maxSeen+1, 56)` 并回写 `candidate+1`；删除归档或活跃行都不会让编号复用（有负例测试）。
6. **校验不解析 git log**：闭环证据写在记录内（`evidence` / `closed_at`），彻底删掉 v2 的 `closing_commit` 自指字段；`lint` 只校验记录不变量与跨归档 ID 唯一性。
7. **归档幂等**：`resolve` 先追加归档（同 ID 已存在则跳过）、再删活跃行；`doctor` 对账「活跃∪归档」的 ID 重复、高水位越界、活跃池 done 残留。
8. **迁移与退役**：旧 triage 7 条记录重编号为 **B056–B063**（重映射见 `normalizeLegacy`，逐条带 `source_ref=legacy-triage`，done 行回填 `evidence/closed_at`）；`backlog/backlog.md` → `backlog/legacy/backlog-2026.md`、`backlog/current.md` → `backlog/legacy/current-2026.md`、旧 triage 账本 → `backlog/legacy/issue-triage/`，全部冻结只读；`scripts/pa-issue-triage.mjs`、`.agents/skills/pa-issue-triage/`、`tests/pa-issue-triage.spec.ts` 删除。
9. **门禁双挂载**：`.githooks/pre-commit` 在 ponygo-gate 哨兵块**之外**追加 `node scripts/backlog.mjs lint`（账本存在但非法即 fail-closed；账本缺失时明确提示并放行，用于迁移期）；CI 在 `ponygo-governance.yml` 追加独立 step（不动不可变资产 `ci-verify.sh`）；`package.json` 增 `backlog:lint` / `backlog:list`。
10. **断点续做无计划文件**：`in_progress` 任务以 `owner` / `touched_paths` / `description` 与 `git diff --stat` 交叉确认进度，`current-plan.json` 语义废止。

## Alternatives considered

- **Markdown（`backlog/backlog.md`）作唯一真源**：落选——无法逐条 schema 校验与状态机校验，多任务并发编辑必生冲突，状态靠改标题不可机械判定；正是本次要消除的形态。
- **新建 `.agents/backlog/tasks.jsonl`（隐藏目录）**：落选——不易发现且历史上已漏 `git add` 成为未跟踪真源；真源必须显眼、可被 CI 直接寻址。
- **用 `git log` 解析 `close:` 提交作闭环校验输入**：落选——v1 实测双向失效：`resolve` 自身提交在 pre-commit 时不存在（自锁）、归档后异常态不可见（false negative）、历史 0 个 `^close:` 命中、`--no-verify` 可绕过（fail-open）。
- **一步内写入 `closing_commit`**：落选——一次提交内的记录无法包含自身哈希（只能写父提交），字段会退化为占位符；改为 `evidence` + `closed_at`，git 仅作审计线索。
- **直接落地 dev-backlog 契约的 `taskctl.py` + `.task-state/{registry,lanes,locks}`**：落选——并发/lane 设计完备但工程量最大，当前为轻量模式；先以 `owner` 字段对齐 lane 语义，保留升级路径。
- **由 CLI 生成 `backlog.md` 只读看板进仓**：落选——生成物每次渲染都改 Git 文本，制造 diff 噪声与误提交面；人读视图由 `backlog list` 承担。
- **保留 `pa-issue-triage` 作为 Defect 专用入口**：落选——两套状态机语义重叠（标识/状态/优先级/断点/归档五维），重叠本身就是漂移源；其能力已被 `category=Defect` + `metadata.fingerprint` 去重 + `triage_context` 承载。

## Consequences

- **落地物**：`scripts/backlog.mjs`（CLI + 库）、`backlog/tasks.jsonl`（活跃 3 条）、`backlog/archive/tasks-2026.jsonl`（归档 5 条）、`backlog/.next-id`（下一候选 B064）、`tests/backlog-cli.spec.ts`（17 例）、`backlog/legacy/`（冻结历史）。
- **机械收据**：`node scripts/backlog.mjs lint` → `PASS（活跃 3 / 归档 5）`；`doctor` → `ok:true`、`highWater:64`；`npx vitest run tests/backlog-cli.spec.ts` → 17/17 passed；`bash .agents/skills/write-adr/verify-note.sh` exit 0。
- **迁移保真**：旧 triage 账本 6 个 distinct fingerprint 全部在新账本中有唯一对应（含迁移首轮遗漏、后补齐的 `tui::rewind::default_scope_destructive` → B063）。
- **行为变化**：`backlog/backlog.md` 与 `backlog/current.md` 不再是人写入口；新增待办一律走 CLI（`record`），闭环一律走 `resolve`（强制 evidence）。`pa-issue-triage` 技能与其触发词不再存在，相关语义合并进 `dev-backlog`（全局 skill 的「数据位置」节已同步改为 JSONL 契约）。
- **验证补充**：`npm test` 全量 154 个 spec 中 1450 passed / 12 skipped / **1 failed**——失败项为 `tests/check-secrets.spec.ts` 的「端到端零误报」用例在 154 文件并行负载下超 5s 预算；单独跑同文件 52/52 通过、该用例耗时 1033ms，属负载敏感 flake（非本次改动引入：被移动的 `backlog.md` 59KB 原已 tracked，扫描体量基本不变）。
- **任务级租约（并发控制）**：`backlog/.locks/<ID>.lock`（`O_EXCL` 原子创建）保证**同一任务同时只被一个会话持有**；`release` 显式释放，`heartbeat` 刷新心跳，`reap --minutes N` 回收会话异常退出留下的过期租约；**不同任务可被不同会话并行推进**（有 6 条用例覆盖：抢占失败/同 owner 重入/并行不同任务/释放鉴权/过期回收/owner 冲突判违规）。
- **编号并存的裁决（2026-11-11 决策）**：不切换前缀。理由：① 提交信息里的历史 `B0xx` 已不可改写，换前缀只影响未来，不能消除历史歧义；② 并发会话正在用 `B064/B065`，换号会破坏在飞协作；③ 约定成本更低——提交信息/文档引用账本任务写 `backlog:B123`，裸 `B123` 视为 legacy 历史编号（已写入 `dev-backlog` 技能的「编号约定」行）。残留：裸 `B` 号仍可能被误读，靠 review 与 `backlog list` 索引消解。
- **限制（如实登记）**：① 并发写 `tasks.jsonl` 本体未加文件锁——两个会话同时 `record` 仍可能丢行（租约只保护**同一任务**的编辑，不保护账本追加），靠「record 后立即提交 + 冲突人工合并」缓解；② 账本是工作区文件，未 `git add` 前 `lint` 走「账本缺失」分支（fail-open），已由 CI 覆盖抵消；③ 与 `dev-backlog` 的 lane/lock 及 `openspec` 校验尚未实现；④ 钩子实测基线约 81–132ms，`lint` 为 O(n) 单遍，记录量上千后需复测；⑤ 全局 skill 文件不在本仓版本控制内，契约同步靠本次改动，未来漂移无机械门禁；⑥ 编号命名空间与旧 `backlog.md` 存在过渡期交叉——并发会话仍在提交信息里使用 legacy 编号（实测 `1a1d400d ... (B057)` 指 legacy 语义，而账本 B057 是「审批消费侧 TTL」条目），账本内 ID 唯一但**提交信息不唯一**，过渡期需以账本为索引解读提交。
