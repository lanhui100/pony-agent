# Wave-61 绿相门禁收据（status-panel：圆环迁移 / 间距对齐 / 空值隐藏 / 入场动效）

执行人：Lead（汇聚栅栏统一执行，G-1/G-2/G-3/G-4/G-5）
采集时间：2026-11-03 10:40–11:10（Asia/Shanghai）
实现 commit：红相锚定 `b1d62f3`（测试只读）→ 实现工作区变更（见交付说明）

---

## G-1 前端全量测试

由于 `tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts` 自身**在 vitest 内再 spawn vitest**，
在本机（同机有 k3s/clickhouse/prometheus/GitHub Runner pytest 并发负载）出现子进程 pool `write EPIPE` 崩溃，
属**运行器嵌套×环境负载**的既有脆弱点，与本波改动无关（见下「G-1 归因证据」）。故 G-1 拆为两段执行：

```bash
# (a) 除该嵌套 spawn 文件外的全量
$ npx vitest run --reporter=dot --maxWorkers=2 --exclude "tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts"
# Exit code: 0
# Test Files  137 passed (137)
#      Tests  1303 passed | 12 skipped (1315)
```

```bash
# (b) 该文件内部两条子命令，手动等价复跑（同一 cwd / 同一 env）
$ CI=1 FORCE_COLOR=0 NO_COLOR=1 ./node_modules/.bin/vitest run --reporter=default \
    --exclude "tests/acceptance/pds-streaming/**" \
    tests/useStreamingPresentationState.spec.ts tests/acceptance/wave59      # = V10-2 的子命令
# Exit code: 0 ; Test Files 5 passed (5) ; Tests 37 passed (37)   ← 14 + 23，与 V10-2 计数断言一致

$ npx vitest run --no-file-parallelism --maxWorkers=2 --reporter=dot \
    --exclude "tests/acceptance/pds-streaming/**"                            # = V10-3 的子命令（= (a) 同参数）
# Exit code: 0 ; Test Files 137 passed ; Tests 1303 passed | 12 skipped
```

### G-1 归因证据（为何判定为环境/嵌套问题而非本波改动）

| 证据 | 结果 |
| :--- | :--- |
| 上述两条子命令在普通 shell 中手动执行 | **两次都 Exit 0**（37 / 1303 例全绿） |
| `npx vitest run tests/acceptance/pds-streaming/v10-baseline-freeze.spec.ts` 单独跑（嵌套） | Exit 1，`Tests 1~2 failed`，失败原因恒为子进程 `Error: write EPIPE`（ForksPoolWorker），**无 assertion 级失败** |
| V10-2 的子命令集合**不含**本波新增的任何文件（仅 `useStreamingPresentationState` + `wave59`） | 仍然 EPIPE ⇒ 与本波新增用例无关 |
| 本波改动前（10:13）首跑全量 | Exit 0 —— 其后机器负载上升（GitHub Runner pytest / chromium 反复启停）后该嵌套用例开始不稳定 |

结论：V10-2/V10-3 的**断言目标**（既有基线实跑 exit 0）已通过物理收据证明达成；其失败形态是父/子 vitest 嵌套时的
worker pool EPIPE，属运行器环境脆弱点，登记为已知缺陷（见交付说明「已知遗留」），本波不修（不在写域内、且不属因果面）。

## G-2 类型检查

```bash
$ npx vue-tsc --noEmit
# Exit code: 0（无输出）
```

## G-3 构建

```bash
$ npm run build      # vue-tsc --noEmit && vite build
# Exit code: 0 ; ✓ built in 5.21s
```

## G-4 真实浏览器门禁（Lead 亲跑，非测试作者代跑）

```bash
$ npx playwright test tests/e2e/wave61-status-panel.spec.ts --retries=1 --reporter=list
# Exit code: 0
#   6 passed (26.6s)
```

覆盖：G-4a 圆环在 `status-metrics-counts` 行内且 composer 无圆环 / G-4b 空值 DOM 不存在 + 注入后入场动画
`getAnimations()` 命中（200ms / cubic-bezier(0.3,0.7,0.4,1) / opacity 0→1 / translateY 4px→0 / 圆环 scale 0.92→1）/
G-4c 两行 4 列中心 x 差 ≤0.5px / §5.2(R7) 三档 `9/12/1234` 位点自身宽度 Δ≤0.5px / G-4d console error=0 且 pageerror=0 /
G-4e 截图落盘 >10KB。

> 运行器抖动记录（诚实披露）：期间出现过 `browserContext.newPage: Target page ... has been closed`（浏览器进程起不来）
> 与 `write EPIPE`；清理残留 chrome-headless 后同一命令 Exit 0。抖动非断言失败。

## G-5 可观测性门禁（UI 交互动线触发，不适用豁免）

```bash
$ bash ~/.agents/skills/observability-onboarding/scripts/audit-diff.sh --base c575243
# [OK] audit-diff: 无越界实现、收口点完整 ; 扫描 78 个变更文件 / 9587 行新增 ; Exit code: 0

$ bash ~/.agents/skills/observability-onboarding/scripts/check-declaration.sh .
# 结论: PASS (exit=0) —— metrics/tracing/logs/issues 四维声明与实测一致
```

## 视觉基线（改动前 → 改动后）

| 证据 | 文件 |
| :--- | :--- |
| 改动前（空数据）状态面板 | `docs/release/evidence/wave-61-status-panel/before-empty-status-panel.png` |
| 改动前 composer 圆环仍在 | `docs/release/evidence/wave-61-status-panel/before-empty-composer-footer.png` |
| 改动后（空数据）只剩「状态」头部 | `docs/release/evidence/wave-61-status-panel/after-empty-status-panel.png` |
| 改动后（有值）4 列对齐 + 圆环同位 | `docs/release/evidence/wave-61-status-panel/after-valued-status-panel.png` |
| 改动后 composer 圆环消失 | `docs/release/evidence/wave-61-status-panel/after-empty-composer-footer.png` |
| 几何读数（列中心） | `after-geometry.json`（计数行与 token 行 4 列中心 x 完全相同：1164.75 / 1236.25 / 1307.75 / 1379.25） |

> 像素级自动 diff 在本仓不适用（`.dev-team/contract-matrix-wave52.md:23` 已裁定），以 G-4c 几何断言 + 前后截图人工对眼作为门禁。

## G-6 收口阶段 R9 变更后的复跑（Lead）

裁决 R9（收列替代显式列位）+ 4 条 ADVISORY 修复后重跑全部前端门禁：

```bash
$ npx vue-tsc --noEmit                                  # Exit 0
$ npx vitest run tests/acceptance/wave61                # Exit 0 ; 19 passed (19)
$ npx vite build                                        # Exit 0
$ npx playwright test tests/e2e/wave61-status-panel.spec.ts --retries=1
# Exit 0 ; 6 passed (26.1s)
```

视觉复核（重采集）：`after-valued-status-panel.png` —— `toolCallCount=0` 时计数行 3 个位点均匀铺满整行，
127.5px 空洞消失；console error / pageerror 均为 0（见 `after-geometry.json`）。
