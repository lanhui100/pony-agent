# Wave-17 绿相物理收据

- 基线 commit：`5d7d691`（工作区干净）
- 红相锚定：`f060c8d`（tag `wave-17-red-anchor`）
- 分级：B-lite
- 收据生成时间：2026-10-16

---

## 1. 机器门禁（全部真实 EXIT 码）

| 门禁 | 命令 | 结果 | 收据文件 |
| :--- | :--- | :--- | :--- |
| 类型检查 | `npx vue-tsc --noEmit` | **EXIT=0** | `evidence/wave-17/green-barrier-final.txt` |
| 冻结验收（C1+C2+C4） | `npx vitest run tests/acceptance/wave17/ime-composition-guard.spec.ts tests/acceptance/wave17/tabular-nums-contract.spec.ts tests/acceptance/wave17-adversarial/ime-guard-adversarial.spec.ts` | **EXIT=0，3 files / 31 passed** | 同上 |
| 全量单测回归 | `npx vitest run` | **EXIT=0，85 files / 897 passed / 10 skipped**（基线 82 files / 866 passed） | 同上 |
| 浏览器真实链路门禁 | `npm run build && npx playwright test tests/e2e/wave17-browser-gate.spec.ts` | **EXIT=0，4 passed**（C3-1/C3-2 Console Error=0/C3-3 截图/C2-2a-d） | `evidence/wave-17/green-browser-gate.txt` + Lead 独立复跑 |
| 版本四处同步 | `npm run version:check` | **EXIT=0** | `evidence/wave-17/green-barrier-final.txt` |
| 本波 NFR 基准校验 | `node .dev-team/tools/validate-nfr-baseline.mjs` | **EXIT=0** | 同上 |
| 既有 16 键冻结基准回归 | `python3 .dev-team/nfr-lint.py` | **EXIT=0**（`.dev-team/nfr-baseline.json` 零字节改动） | 同上 |
| Flake 复跑 | 冻结验收 × 3 次 | **3/3 EXIT=0，31 passed** | `evidence/wave-17/green-flake-check.txt` |

### 1.1 红相物理复核（隔离 worktree，防「红相被测试修订污染」）

在 `git worktree add --detach /tmp/w17-redverify wave-17-red-anchor` 拉出的**红相工作树**中（`src/lib/ime.ts` 仍为 `throw new Error("NotImplemented")`），放入 **REV-2 修订后**的 adversarial spec 复跑：

```
Tests  12 failed | 2 passed (14)   RED_VERIFY_EXIT=1
```

→ 证明 REV-2 的测试修订**未削弱红相证伪能力**，红相锚定依然有效。工作树已 `git worktree remove --force` 回收。

---

## 2. 变更清单（union diff = 7 文件，101 行）

| 文件 | +/- | 归属任务 |
| :--- | :--- | :--- |
| `src/lib/ime.ts` | +8/-5 | task-3 (executor-a) |
| `src/components/chat/WorkspaceComposer.vue` | +22/-5 | task-3 (executor-a) |
| `src/components/HomeSessionSidebarRow.vue` | +8/-1 | task-3 (executor-a) |
| `src/components/HomeSessionSidebar.vue` | +8/-1 | task-3 (executor-a) |
| `src/components/HomeStatusPanel.vue` | +19/-14 | task-4 (executor-b) |
| `src/components/HomeTracePanel.vue` | +4/-4 | task-4 (executor-b) |
| `src/components/AskPanel.vue` | +1/-1 | task-4 (executor-b) |

完整 diff：`evidence/wave-17/union-diff-src.patch`（Lead 已核对与 `git diff -- src/` 逐字节一致）。

### 2.1 机械写域审计

- task-3 write_scopes = {`src/lib/ime.ts`, `WorkspaceComposer.vue`, `HomeSessionSidebarRow.vue`, `HomeSessionSidebar.vue`} → 实际变更文件 ⊆ 声明 ✅
- task-4 write_scopes = {`HomeStatusPanel.vue`, `HomeTracePanel.vue`, `AskPanel.vue`} → 实际变更文件 ⊆ 声明 ✅
- **越界文件数 = 0**。两任务文件集交集 = ∅。
- `tests/**` 与 `src/**` 写域正交（Test Agent 未写 src，Executor 未写 tests）✅

### 2.2 Blast-radius 检测

- union diff 修改行数 = **101 > 100** → 机械触发「≥2 路互补 L3 审查」（已执行，见 §3）。
- `.dev-team/blast-keywords.txt` 命中：0（变更面为 Vue 模板 class/事件绑定 + 一个纯谓词函数，无 lock/thread/http/io/fs/exec/unsafe）。
- 跨写域变更：0。

---

## 3. L3 对抗审查（Subagent 池，互补视角矩阵）

| 审查 | 视角 | 文件 | verdict | findings |
| :--- | :--- | :--- | :--- | :--- |
| L3-A | 并发/调度/状态流转 | `.dev-team/review-wave17-a.json` | **PASS** | 0 |
| L3-B | 安全防御/协议一致性 | `.dev-team/review-wave17-b.json` | **PASS** | 3（全 `[NON-BLOCKING]`） |
| L3-P | 生产就绪五问基准映射否决制 | `.dev-team/review-wave17-l3p.json` | **PASS** | 0 |

Schema 机器解析校验（`verdict` 枚举 + `findings[].file/line/rule/counterexample` 类型）→ **SCHEMA_ALL_OK=true**，致命 findings 数 = 0。

### 3.1 L3-B 三条 NON-BLOCKING 的 Lead 裁决

1. **`src/lib/ime.ts:27` 与 `HomeWorkspace.vue:1369` 判定集合非全等（跨 realm / 非布尔 `isComposing`）**
   → **接受为已知差异，不改**。差异方向是**更严格**（多一道 `instanceof` 闸门 + `=== true`），只会更保守不会误放行；审查者已实测本仓 `src/` 内 iframe/webview/`createEvent` 零命中，5 个调用点全为 Vue 原生 `@keydown` 同 realm 事件，**无实际可达路径**。为对齐而修改 `HomeWorkspace.vue` 属超范围改动（违反最小闭包）。

2. **`HomeWorkspace.vue:1369` 守卫粒度过宽，组合态吞掉 Ctrl/Cmd+Z 撤销**
   → **确认为存量缺陷，非本波引入**（union diff 未触及该文件）。登记 backlog **PA-135**，不在本波夹带修复。

3. **`ime-composition-guard.spec.ts:288` C1-4 缺 keyCode 229 的反向断言**
   → **接受为已覆盖，不扩冻结测试集**。反吞方向已由 C1-5（`isComposing:false` + Enter 必须提交）与 C4-3（200 次非组合态回车 → 精确 200 次提交）双向覆盖；「永久无法回车提交」若发生，C1-5/C4-3 必红。

---

## 4. 契约修订记录（全部为锚定后澄清/测试缺陷修订）

| 修订 | 阶段 | 内容 | 熔断计数 |
| :--- | :--- | :--- | :--- |
| REV-1 | 红相锚定前 | C2-2 拆为 a/b/c/d：`tabular-nums` 只统一**等位数**字形宽度，位数重排必须由容器 `min-w` 吸收（Test Agent 实测 Δ=17px→21px 证明原口径不可满足） | 契约缺陷，不计数 |
| REV-2 | 绿相 | L2-AT 两处 `toBe(rawObject)` 恒假（Pinia reactive Proxy）→ 改 `toRaw` 身份等价 + 诱饵负向自检 | **用例自身缺陷，豁免计数** |
| REV-3 | Join Barrier | C3-3 截图拍到启动闪屏（`index.html:17` + `main.ts:91-96`）→ 补闪屏移除等待 + `elementFromPoint` 遮挡命中证伪器 | **用例自身缺陷，豁免计数** |

三条修订**均未削弱任何断言强度**，REV-2 另有隔离 worktree 复跑证明红相仍真红。

---

## 5. 基线缺陷登记（非本波引入，不得计入回归）

| id | 描述 | 处置 |
| :--- | :--- | :--- |
| BASE-E2E-1 | `npm run test:e2e` 在基线即 8 failed / 12 passed（全新构建后复现）。`tests/e2e/browser-preview.spec.ts` 断言 `session-sidebar-session-list`，侧栏结构已漂移。 | 登记 backlog **PA-134**；本波 C3 走独立 wave-17 门禁 spec，不依赖其转绿 |

---

## 6. 结论

C1（IME 组合态回车守卫，6 处入口）与 C2（动态数字抖动消除，8 位点）全部绿相通过；7 文件 / 101 行最小闭包；零越界；零致命审查发现；真实 Chromium Console Error = 0 且关键旅程截图有效。
