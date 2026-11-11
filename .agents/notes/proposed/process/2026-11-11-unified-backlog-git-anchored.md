# Agent Note: 待办与问题治理统一（backlog/tasks.jsonl 单一 JSONL 真源 + 记录级机械校验）

Status: proposed

## Problem

仓库同时存在三套任务载体，事实重复且互相漂移：

1. `backlog/backlog.md`（59KB、git-tracked、B001–B055 已被真实使用，闭环提交形如 `docs(backlog): close B050-B054`）；
2. `.agents/skills/pa-issue-triage/` + `scripts/pa-issue-triage.mjs`（会话异常捕获、指纹去重、`current-plan.json` 断点、`archive/resolved-*.jsonl`）；
3. `.agents/backlog/tasks.jsonl`（本轮由 triage 数据迁入，**未纳入版本控制**）。

Markdown 不适合作多任务状态流转与机械校验的载体：无法逐条 schema 校验、状态靠改标题、并发编辑必冲突、diff 噪声大。而 `pa-issue-triage` 的 `recordIssue` 用 `issues.length + 1` 生成 ID，归档后序号断层导致碰撞（实测又造出一个 `ISSUE-20261111-008`）。进度记录（`current-plan.json`）是人工维护的第二真值源，且单文件单例无法支持多会话。

方案 v1 曾提出「JSONL + Git 锚定校验」，经三路独立对抗审查全判 FAIL（36 findings / 22 BLOCKING），核心否决理由：新建未跟踪的第 4 个真源、ID 撞车既有 B008–B012、用 `git log` 解析 `close:` 导致自锁且 fail-open、`in_progress` 无归属使 `git diff` 无法归属任务。v2 保留 JSONL 方向，逐条修正上述四类缺陷。

## Proposal

1. **单一 JSONL 真源（跟踪纳管）**：唯一活账本为 `backlog/tasks.jsonl`（一行一任务，路径与既有 `backlog/` 同族、人可见、**必须 git-tracked**）。`backlog/backlog.md` 一次性导入后 `git mv` 至 `backlog/legacy/backlog-2026.md` 冻结，不再手写；不生成 Markdown 看板视图（生成物进 Git 只增 diff 噪声，人读视图由 `backlog list` 终端渲染）。
2. **行级 Schema**：必填 `id`(`^B\d{3,}$`)、`category`(`Defect|Task|Debt`)、`priority`(`P0|P1|P2|P3`)、`status`(`todo|in_progress|done`)、`title`、`created_at`、`metadata`（含 `fingerprint`、`occurred_count`、可选 `triage_context`）；可选 `description`（≤2000 字符）与 `doc_ref`（ADR 路径）。`triage_context` 限长 8KiB 并对 `*key*`/`*token*`/`*secret*`/`*password*` 字段值做脱敏。未知字段拒绝（严格模式）。
3. **ID 分配独占**：仅 CLI 分配，起始 **B056**（续现有高水位）；分配前扫描 `tasks.jsonl` + `archive/*.jsonl` 取最大编号 +1；临界区用文件锁（`flock` / 锁文件 + 原子 rename），人工改 ID 由 `backlog lint` 拒绝。
4. **状态机机械可判**：合法跃迁仅 `todo→in_progress→done`（回退 `in_progress→todo` 允许；`todo→done` 直跳判违规）。`in_progress` 必填 `owner`（lane/session 标识）与 `touched_paths`（≥1 个仓内相对路径）；`done` 必填 `evidence`、`closed_at`、`closing_commit`。
5. **滚动归档**：`done` 时先向 `archive/tasks-<YYYY>.jsonl` 追加该行、刷盘后再从活跃池删除（顺序固定，失败可重放）；`backlog doctor` 启动对账「归档 ∪ 活跃」与 ID 连续性，发现双写/丢失即非零退出。
6. **闭环与证据在记录内**：`backlog resolve <ID> --evidence "..."` 写入 `done` 三件套 → 归档 → 定向暂存（仅 `backlog/tasks.jsonl` 与 `backlog/archive/`，**禁止 `git add .`**）→ `git commit -m "close <ID>: <title> [evidence]"`。提交文案不参与任何校验（兼容历史 `docs(backlog): close …` 风格）。
7. **校验只看记录不变量（不解析 git log）**：`backlog lint` 做行级 schema、状态机跃迁、done 三件套、ID 唯一（含跨归档去重）、`doc_ref` 存在性、活跃池无 `done` 残留。挂载点：pre-commit 中**放在 `ponygo-gate` 哨兵块之外**（判据 2.4 要求该段逐字节一致，否则会被 init 静默删除），且 fail-closed（缺文件/读失败 → 非零退出，不静默放行）；同一命令在 CI 四段门内再跑一次，抵消 `--no-verify` 穿透。
8. **断点续做以记录 + 代码为真值**：任务为 `in_progress` 时，读 `owner` / `touched_paths` / `description` / `triage_context`，再与 `git diff --stat` 交叉确认；不引入任何 plan 文件（`current-plan.json` 语义废止）。
9. **退役 pa-issue-triage**：先改写 `tests/pa-issue-triage.spec.ts`（改为覆盖 backlog CLI 的 record/resolve/lint，含 schema 负例与 ID 分配用例），再删除 `scripts/pa-issue-triage.mjs` 与 `.agents/skills/pa-issue-triage/`；触发词并入 `dev-backlog` skill 的 description；删除 `.agents/issue-triage/` 前做数据核对（每条 fingerprint 必须在 `tasks.jsonl` 有对应记录）。
10. **保留 lane 扩展口**：不引入 `scripts/taskctl.py` / `.task-state/`（保持轻量），但 `owner` 字段语义与 dev-backlog 的 lane 模型对齐，未来接 lane/lock 不需迁移数据。

## Alternatives considered

- **A. 以 `backlog/backlog.md` 为唯一真源（只补规范）**：落选——Markdown 无法逐条 schema 校验与状态迁移校验，多任务并发编辑必生冲突，状态靠改标题不可机械判定；正是本次要解决的问题。
- **B. 直接落地 dev-backlog 契约的 `taskctl.py` + `.task-state/{registry,lanes,locks}`**：落选——并发/lane 设计完备但工程量最大，且当前为轻量模式（`backlog/current.md` 明示未启用）；先用 JSONL + `owner` 字段，保留平滑升级路径。
- **C. 把 `tasks.jsonl` 放在隐藏目录 `.agents/backlog/`**：落选——不易发现、历史上已漏 `git add` 导致未跟踪，真源必须显眼且可被 CI 直接寻址。
- **D. 用 `git log` 解析 `close:` 提交作为闭环校验输入**：落选——v1 实测双向失效：`resolve` 自身提交在 pre-commit 时不存在（自锁），异常态在归档后不可见（false negative），历史 0 个 `^close:` 命中且可被 `--no-verify` 绕过（fail-open）。改为校验记录内 `evidence/closing_commit` 三件套，git 仅作审计线索。
- **E. 由 CLI 生成 `backlog.md` 只读看板进仓**：落选——生成物每次渲染都改 Git 文本，制造 diff 噪声与误提交面；人读视图由 `backlog list` 终端输出承担。

## Acceptance criteria

1. `backlog lint` 对负例（非法 `category`、缺 `title`、ID 非 `B\d{3,}`、`todo→done` 直跳、`done` 缺 `evidence`）均非零退出，且错误信息定位到行号。
2. ID 分配：导入后高水位为 B055，新任务首个分配到 **B056**；构造「归档后删除再分配」场景，不得复用退役编号（有负例测试）。
3. `git ls-files backlog/tasks.jsonl` 非空；`backlog/legacy/backlog-2026.md` 存在且 `backlog/backlog.md` 已不存在。
4. pre-commit 与 CI 均执行 `backlog lint`；构造「删掉 `tasks.jsonl` 后提交」场景，两处均非零退出（fail-closed 可观测）。
5. `resolve` 后：活跃池无该 ID、`archive/tasks-<YYYY>.jsonl` 有该行、`git log` 出现 `close <ID>` 提交、暂存集仅含 backlog 路径（`git diff --cached --name-only` 断言）。
6. 删除 `pa-issue-triage` 后 `npm test` exit 0（spec 已改写），全仓 `grep -rn "pa-issue-triage"` 仅剩历史记录与 notes/ADR 引用。
7. 迁移核对：`.agents/issue-triage/` 中每条 fingerprint 在 `backlog/tasks.jsonl` 或 `archive/` 有唯一对应记录（0 丢失、0 重复）。
8. `bash .agents/skills/write-adr/verify-note.sh` exit 0；`backlog lint` 在 1000 条记录下 < 300ms（秒级预算内）。

## Risks

- **迁移保真**：`backlog.md` 的 59KB 散文无法结构化进 JSONL，`legacy` 冻结文件只保历史可读性，不进机械校验面；导入映射需人工复核一次（`category` 与 `priority` 的推断规则可能误分）。
- **并发写**：文件锁只在 CLI 内生效，人工直接编辑或 IDE 扩展写 `tasks.jsonl` 仍会与 CLI 竞争；缓解为「禁人工编辑 + `backlog doctor` 对账」，而非强锁。
- **单文件增长**：活跃池靠归档切片维持小规模，但 `archive/` 逐年增长；`backlog lint` 需保持 O(n) 单遍实现，否则秒级预算会破。
- **pre-commit 叠加**：既有钩子实测 0.07s，新增 lint 必须放哨兵块之外并保持亚秒级；顺序上 `adr-verify` → `check-decisions` → `backlog lint`。
- **`--no-verify` 与多 worktree**：本地钩子天然可绕过（已由 CI 覆盖抵消），但其他 worktree 若无对应文件副本，`backlog lint` 会因缺文件 fail-closed 报错，需在文档中写明「新 worktree 先 `git checkout -- backlog/`」。
- **脱敏边界**：`triage_context` 的键名启发式脱敏只能挡常见形态，不能保证零泄密；写入端仍应避免整段错误体落地。
