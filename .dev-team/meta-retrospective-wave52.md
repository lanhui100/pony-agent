# Wave-52 元框架复盘（A 级：设置页「软件自动更新」卡片）

任务：桌面端设置中「软件更新」→「软件自动更新」、开关 Switch 移到同行最右端、上次检查日期移到当前版本之后。
分级：A 级（1 文件 +20/−18，2 个冻结测试文件；无并发/鉴权/网络新增）。
波次证据：`.dev-team/wave52-receipts.md`、`.dev-team/contract-matrix-wave52.md`、`.dev-team/wave52-red-receipts.md`、`.dev-team/review-wave52-a.json`、`review-wave52-b.json`。

## 1. 通信拓扑与信噪比

- 拓扑：Lead（契约 + 冻结测试 + git 唯一写主）→ 1 名常驻 Executor（`executor-ui`，唯一写域 1 文件）→ 2 路 L3 审查下沉 Subagent 计算池（不占名册）。Roster = 2 人，符合容量保守引理。
- 有效降噪：Executor 首轮只做只读侦察并回报「等 GO」，未出现抢跑写码；Lead 用「红相锚定提交」作为唯一放行信号，避免了「边写边改测试」的自证链路。
- 信噪比问题（真实存在）：Executor 把 `.dev-team/wave52-visual-gate-deviation.md` 误判为「他人可疑产物」上报。根因不是它多话，而是 **Lead 在该文件落盘后未在契约/消息中登记归属**——共享工作区里「未知未跟踪文件」对任何成员都是合理的风险信号。改进项见 §4-1。
- Token 效率：无重复全量构建（build 仅 2 次：红相前基线、绿相后验收）；`npx vitest run` 全量仅 2 次（基线 / 绿相）。重复读取率 0（契约文件一次成型，未反复读组件）。

## 2. 门禁穿透与误杀率

- **红相真实拦截**：冻结的 4 条 jsdom + 3 条浏览器几何断言在实现前全部按断言失败（不是编译错、不是找不到文件），证明测试先于实现且能证伪。
- **假测试风险点与处置**：几何断言若写成 class 断言（`justify-between`）就是代理指标、可被「类名对了但实际错位」骗过；本波改为真实 Chromium 的 `boundingBox()` 像素断言（`|switch.right−row.right| ≤ 2px`），并由审查员在 6 档视口独立复算为 0px，双向确认。
- **误杀率（本波关键收获）**：视角 B 报出 1 条 ADVISORY（窄视口 360/400 折行），并**自带不可达性反证**（`tauri.conf.json minWidth=780`）。这正是「已知局限免死金牌」机制该有的样子：不是审查员放水，而是拿出结构性证据把 BLOCKING 降级。Lead 未加 `whitespace-nowrap` 修复——那属于超出需求的新样式变更，且可能把折行换成裁剪，登记为后续候选。
- **审查纠错价值（证据驱动，非零和）**：视角 B 用实测推翻 Lead 在偏差文档里的表述「卡片高度改变导致下游位移」（实测前后均 119px），Lead 已按证据更正文档。**审查员纠正 Lead 的事实错误**是本波门禁质量的最强信号。
- 缺口（如实记录）：WebKit/WKWebView 未覆盖（本机无 webkit/firefox 二进制）；本轮 Console Error=0 仅在 Chromium 上取得。

## 3. 分工契约与隔离有效性

- 测试分权成立但形式为「Lead 兼测试（A 级豁免）」：冻结测试作者 = Lead，实现者 = Executor，二者不同体；Executor 全程未获测试写权，回报中明确「不造测试」。若本任务升为 B 级，应由独立 Test Agent 承担。
- 契约前置的价值被实证：`data-testid` 锚点（`config-update-section-title` / `-version-row` / `-autocheck-row`）在红相前就写进契约，Executor 一次实现即全绿，无「测试改标记」的返工轮。
- 隔离等级：k=1 共享工作区轻量路径（无 worktree），git 写操作全部由 Lead 执行，Executor 只读 status/diff；熔断未触发。
- 一处流程冗余：A 级任务仍走了 Subagent 双路审查，成本略高于「1 路审查」下限，但换回了 §2 的两处实质纠错，性价比为正。

## 4. 元协议迭代建议（对 dev-team skill 本身）

1. **共享工作区「未知未跟踪文件」登记义务**：建议在「物理状态账本」条款增加一句——Lead 写入任何 `.dev-team/` 新文件后，须在**下一条发给成员的正文**里登记路径与用途；否则成员必须按可疑上报，产生无谓往返。本波已实际发生一次。
2. **「像素 diff」门禁的适用边界应当显式化**：现行条款要求样式调整类任务做像素对比，但「刻意改变元素落位」的需求里像素 diff 必然假红。建议把本波采用的替代口径写进 skill：以**真实浏览器几何断言**（含容差与证伪组合）取代，并把「变更前后元素盒子尺寸」实测作为「无下游位移」的机器证据（本波 145/119px 双测）。
3. **A 级豁免须与「测试作者≠实现者」解耦表述**：现行措辞把「Lead 兼测试」与「测试分权」并列，容易被解读为分权失效。建议明确：A 级只是允许 Lead 兼任测试作者，分权约束（实现者不得改测试）不降级。
4. **审查收据应强制附「不可达性反证」字段**：本波 ADVISORY 之所以可信，是因为它自带 minWidth 反证。建议在 `dev-review-lint.py` 的 finding schema 中为 ADVISORY 增加可选 `unreachable_proof` 字段，把这条最佳实践固化为结构。

## 审查收据指引（机械核对用）
- 计划阶段：A 级快速路径**未启用**方案对抗审查（`.dev-team/review-plan.json` 等为历史波次存量文件，非本波产物；本波以「契约矩阵 + 红相锚定」替代）。
- 实施收口：本波两路审查 `.dev-team/review-wave52-a.json` / `.dev-team/review-wave52-b.json`，均 `dev-review-lint.py --scopes` exit 0，verdict 均为 PASS。
- 门禁：红相 exit 1（4+3 失败）、绿相 `npm run build` exit 0、全量 `vitest` 1060 passed、浏览器门禁 5 passed、`audit-diff.sh` exit 0、`check-declaration.sh` exit 0。
