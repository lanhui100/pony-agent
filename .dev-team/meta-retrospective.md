# Wave-38 元复盘（TUI 启动目录即默认 workspace）

> 复盘对象：本次 Agent Team 的**协作元框架与组织协议**。账本：`.dev-team/state.json`（wave 38）、契约 `.dev-team/contract-matrix-tui-workspace.md`、收据 `.dev-team/evidence/wave38/*` 与 `tests/acceptance/receipts/l2at-tui-workspace/`。

## 1. 通信拓扑与信噪比

| 要素 | 实际 | 评价 |
| :-- | :-- | :-- |
| 名册 | Lead + Protocol + Test + Executor（4/8），审查走 Subagent 池 | 合适：B 级单模块跨核心/TUI 改造 |
| 看板 | T1 空桩 / T2 红相 / T3 实现 / T4 delta 空桩 / T5 delta 测试 / T6 delta 实现 | 偏序干净，全部 completed，无 in_progress 残留 |
| 审查 | L3-A/B/P 首轮 + A2/B2/P2 定向复审 | 首轮 1 PASS + 2 FAIL，delta 后复审 PASS |
| 关键增益 | E2E 红相在实现前真实复现「工具读 ~/pony_agent」 | 根因钉死，杜绝「猜路径」式修复 |
| 信噪比 | Protocol 主动上报 NFR 基线冲突与 `#[allow]` 偏差；Test 上报 stale-rlib 误红 | 好：执行方有自检与自我纠正 |

- 有效信号：`blocked_by` 把「空桩→红相→实现」固化为机器可读偏序；契约 §1→§5 每次增量先落盘再派工。
- 噪声：① 空桩指令误写「覆盖既有 16 键 NFR 冻结基线」，由 Protocol 自查拦下并改为增量文件；② 红相 E6 隐式依赖工作区既有 `/thinking` 未提交改动，直到历史重建才暴露；③ 首轮 3 路复审中的 A/B 结果按期返回，P 轮超时后重派，Lead 以产物落盘为唯一判据。

## 2. 门禁穿透与误杀率

- **红相真实性**：L2-T 9/10 失败、失败点全部位于 SUT 空桩（`workspace.rs:276` / `tui.rs:1256`）；L2-AT E2E 真实复现 DEFAULT_WS_MARKER 被读取、项目标记缺失；`dev-red-purity.py` 判定 PURE_RED（EXIT=0）。实现后 15/15 契約测试、E2E 10/10 通过。
- **假测试拦截**：AT1 首版只测纯函数（不触达 SessionStore/TUI 持久化），被 L3-B 判为假绿；delta 补 AT1-RO（真实 `chmod 500` HOME + tmux 启动，`home_writable_after_chmod=0`、无 panic、exit 0）后闭合。
- **穿透事件**：resume 路径的 header/工具分叉未被首版 E2E 覆盖（脚本不发 `/resume`），由 L3-B 发现并按 §5.D1 冻结修复 + E8–E12 纯函数矩阵；说明「happy path E2E 全绿」不等于跨路径一致。
- **误杀/降级**：L3-P Q3（`save_store` 吞错）经核查属既有 SessionBackend 契约（`backend.rs:216` 返回 `()`），按最小闭包裁定 D4（best-effort + fail-safe 归一 default），拒绝夹带后端错误传播重构；D3（workspace 记录无 TTL）裁定为显式设计。两轮复审最终 PASS。
- **物理收据**：`cargo check -D warnings` EXIT=0、契约测试 15 passed、E2E 10/10、中间提交（WIP/红相/空桩）在隔离 worktree 全部可独立构建、主 E2E 含 mock JSONL 与屏幕截图。

## 3. 分工契约与隔离有效性

- 写域正交：Protocol 只碰 4 个 src 的空桩、Test 只碰 tests/acceptance、Executor 只碰 src 实现；`git diff --name-only` 审计显示 delta 变更全部落在契约声明范围。Executor 未改任何测试断言，Test 未改 src。
- **隔离失效真实现场**：共享工作区存在他人未提交的 `/thinking` 改动（`tui.rs`/`config.rs`/`Cargo.lock`），wave38 测试与实现引用了其中 `reasoning_effort` 字段，导致提交树无法独立构建。经用户授权后执行「WIP 独立快照提交 + 历史重建（plumbing）」，提交链恢复为 `7145631 → WIP → 协议 → 红相 → 绿相 → delta`，三方工作均未丢失。
- 滚动契约账本有效：§5 增量把「resume 语义 / AT1 假绿 / 记录增长 / 持久化裁定」四项全部先冻结后派工，实现与测试无口头漂移。
- 并发 ROI：改动集中在同一 `tui.rs`，全程 WIP=1 串行是正确保守选择；未发生写域竞争。

## 4. 元协议迭代建议

1. **共享工作区 WIP 预检门禁**：Phase 1 增加「`git status` 存在非本任务改动」的强制裁定步骤（快照/隔离 worktree/等待），并要求红相测试只依赖已提交符号，杜绝隐式依赖。
2. **红相历史独立性校验**：Test Agent 交付前必须在临时 worktree（HEAD-only）编译测试；无法编译即判 TEST-DEFECT 打回。
3. **审查产物超时守门**：L3 Subagent 必须声明写盘路径与截止轮次；Lead 在约定轮次无产物即重派并登记「审查未落地」事件，禁止把缺位当 PASS。
4. **Delta 闭环模板化**：FAIL finding 必须走「finding → 契约 patch → delta 测试 → delta 实现 → 定向复审」五段式，并在 `state.json` 记录 finding→commit 映射，确保 Lead 裁定可追溯。
5. **E2E 场景矩阵强制化**：TUI/CLI 变更默认覆盖「新建 / 恢复 / 只读 HOME / 异常退出」四场景；仅 happy path 的 E2E 不得作为放行证据。
6. **NFR 基线增量约定**：冻结基线只增不改；每波增量落 `nfr-baseline-waveXX.json`，避免覆盖导致 schema 失配。
