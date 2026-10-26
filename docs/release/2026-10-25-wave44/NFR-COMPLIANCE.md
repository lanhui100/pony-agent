# Wave-44 NFR 达标报告（前端健壮性收敛：PA-115 / PA-135 / PA-117）

> 基准：`.dev-team/nfr-baseline-wave44.json`（`dev-nfr-lint.py` Exit 0）
> 收据：`.dev-team/evidence/wave44/RECEIPTS.md`｜契约：`.dev-team/contract-matrix-wave44.md`｜评审：`review-plan-{a,b}.json`、`review-wave44-{a,b}.json`、`review-l3p-wave44.json`

| # | 基准字段 | 实现/测试证据 | 结论 |
| :--- | :--- | :--- | :--- |
| 1 | `hydration_read_timeout_ms=15000` + 调用点夹逼 | `src/stores/runtime.ts:2696` `withTimeout(hostRead, HYDRATION_TIMEOUT_MS, "加载对话")`；PA115-T0 常量断言、T2 下界 14000ms 未收敛 / 16000ms 收敛（1000ms mutant 被击杀） | **PASS** |
| 2 | `draft_preserved_after_timeout=true` | `runtime.ts:2723-2728` 回滚前保存/回滚后恢复 draft；PA115-T2a 真实断言 | **PASS** |
| 3 | `visible_failure_required` + 禁止 stuck label | 超时走既有 catch → `sessionError="初始化对话失败：…超时"`、`agentRuntimeStatus.mode=error`；PA115-T2 | **PASS** |
| 4 | `submit_turns_during_composition=0` | `HomeWorkspace.vue:1413-1428` 守卫收敛进 Enter 分支（isComposing / keyCode=229 同权）；PA135-T1/T2 | **PASS** |
| 5 | `undo_invocations_during_composition=1` | 撤销分支前置、目标为最新 turn 父节点；PA135-T1（`node-root` 参数断言） | **PASS** |
| 6 | `debug_log_default_off`（生产） | `utils.ts:45-67` 优先级 storage false > true > URL > DEV；`HomeWorkspace.vue:196-199` + `useTimelineAutoScroll.ts:187-190` 双短路 | **PASS** |
| 7 | `layout_reads_when_disabled=0` | 关闭态 `getBoundingClientRect`=0（PA117-T1），启用态>0（参照有效，logs=2/rect=12） | **PASS** |
| 8 | `runtime_reevaluation_no_cache` | 每次调用实时求值（PA117-T3 关→开→关即时生效） | **PASS** |
| 9 | `external_call_timeout_ms=3000` | 本波无新增外部调用；N-A 理由见契约 §2.1/§2.3 | **N-A** |
| 10 | `logging.format=json + trace_id` | 本波无新增结构化日志面（`console.info` 为诊断输出，已登记 `docs/observability.yaml` note） | **N-A** |
| 11 | `resource_release` | `withTimeout` settle 后 `clearTimeout`；`initializeSessions` finally `clearTimeout(watchdog)` | **PASS** |

## 机器收据汇总

| 命令 | Exit | 结果 |
| :--- | :--- | :--- |
| `npx vitest run tests/acceptance/wave44` | 0 | 8/8 passed（红相锚定前 8/8 failed） |
| `npm run test:unit` | 0 | 102 files / 1034 passed / 10 skipped |
| `npm run build` | 0 | vue-tsc 0 diagnostics；vite built in 17.39s |
| `npx vue-tsc --noEmit` | 0 | 无 diagnostics |
| `npm run version:check` | 0 | 版本四处同步 |
| `node scripts/check-secrets.mjs --all` | 0 | 0 leaks |
| dev `vite` :4176 + 生产 preview :4177（agent-browser） | — | 两态 Console Error=0；生产态 `[workspace-scroll]`=0；截图见证据目录 |
| `check-declaration.sh` | 0 | 四维声明一致 |
| `audit-diff.sh --base ab54b06 --decl-approved` | 1（仅 WARN） | 无违规；WARN=声明无收口点（结构性，已登记） |

## 遗留

- `layout_reads_when_disabled` 的“0 次 buffer 分配”目前以「缓冲未增长 + latest undefined」观测，未断言“数组对象未被创建”（审查 B advisory #5），登记为 Non-Goal（可观测等价）。
- 启动遮罩期间草稿路径属 defense-in-depth（审查 B advisory #4）：超时收敛保留草稿在 store 层可测，UI 层在遮罩消失后仍有效；不改变本波验收口径。

## 审查遗留（已登记，非本波阻断）

- `runtime.ts:2678` local-cache 策略的宿主读仍为裸 await（契约 Non-Goal 排除，建议按 PA-115 同语义另立 backlog）。
- 超时 catch 使用 t=0 快照整包回滚；watchdog 解锁窗口内若已 `switchSession`，15s 收敛会把 sessionId 回退（探索面：启动遮罩下 UI 不可达，登记为后续加固）。
- `HomeWorkspace.vue:1525` markdown render-complete 仍采集 `getBoundingClientRect`（盲区 B-117-1，已声明不外推“整页零布局读数”）。
- 关闭态 “0 次 buffer 分配” 以「缓冲未增长 + latest undefined」观测，未断言数组对象未创建（可观测等价）。
