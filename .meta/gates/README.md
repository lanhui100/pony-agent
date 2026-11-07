# gates/ —— 工程门禁集合

本项目将关键验证逻辑收敛为非零退出（non-zero exit）命令，与 `.githooks` 及 GitHub Actions CI 协同运作。

## 门禁资产编号约定

`A<n>` 是 ponygo 治理骨架的**资产编号**（生成物清单里的槽位号），让脚本、钩子、CI 适配器与本文
互相引用时不必复制路径。编号一经分配不复用，新增资产只在末尾追加。

判据：**门禁本体**（判定逻辑真实可执行，A4 恒生成、进宇宙表）与**平台薄适配器**（A5，换 forge 只换壳）
必须分离——适配器里不许出现判定逻辑。

## 门禁索引

### 决策记录（ADR）链

> **判定真源**：`.agents/skills/write-adr/verify-note.sh`（1.1–1.10 机械影子：目录封闭集、深度、
> 文件名日期、Status 与生命周期一致、骨架标题时态、`## Alternatives considered` 必含、文档有家）。
> 无参 = 全树，带路径 = 单文件。它是 A1 与 A4 第 1 段共用的判定真源，**未分配资产编号**
> （不属 ponygo 生成骨架的门禁槽位，由技能包维护）。

1. **A1 —— 暂存区 ADR 格式门**：`.meta/gates/adr-verify.sh`
   - 无参 = 校验暂存区里改动的 ADR；带参 = 校验指定相对路径。
   - 判定委托 `verify-note.sh`；A1 只补它做不到的 R6 TOCTOU、归档探测与路径归一。
   - 接入：`.githooks/pre-commit`。
2. **A2 —— A1 的负样本执行判据**：`.meta/gates/adr-verify.spec.sh`
   - 证明「门确实会拒绝非法样本，且会放行合法样本」；样本建在 `mktemp -d`，只读仓库。
   - 接入：`A4` 第 2 段。
3. **A4 —— forge 无关的全树门**：`.meta/gates/ci-verify.sh`
   - 三段：全树 ADR 格式（`verify-note.sh` 无参）→ 暂存区门自检（`A2`）→
     归档封印（`A6` check + `A7` 自检）。
   - `A6`/`A7` 缺件即 **fail-closed**（exit 1），不静默跳过：缺件 = 封印无人看守，
     按「未验证不得计为 PASS」处理。
   - 本地复现远端结论：`bash .meta/gates/ci-verify.sh`。
4. **A5 —— CI 平台薄适配器**：`.github/workflows/ponygo-governance.yml`
   - 仅一行 `bash .meta/gates/ci-verify.sh`；换 GitLab / Jenkins / Drone 只换这层壳。
   - **不进宇宙表**：本身不含判定逻辑。

### 归档封印链

5. **A6 —— 归档 SHA-256 封印门**：`.meta/gates/archive-verify.sh`
   - 位置子命令（零旗标）：无参 / `check` = 校验封印（只读）；`seal` = 追加新封印（唯一写入路径）。
   - manifest：`.agents/notes/archived/manifest.sha256`，纯文本**追加式**，
     每行 `<64hex><空格><空格><仓库相对路径>`（`sha256sum` 原生格式，小写十六进制）。
   - 接入：`A4` 第 3 段 + `A1` 的归档探测。
6. **A7 —— A6 的负样本执行判据**：`.meta/gates/archive-verify.spec.sh`
   - 五类负样本（篡改 / 缺条目 / …）各配正向对照，防「一律拒绝」的空门混过；只允许 0/1 退出。
   - 接入：`A4` 第 3 段。

### 仓库卫生与版本（PowerShell 遗留链，尚未纳入 A4）

> 这三条是 Windows 开发机上的既有门禁，判定逻辑本身有效，但**未接入 `A4`**，
> 因此 Linux CI 上不会执行。登记在此以保证「索引与实现不漂移」；纳入 `A4` 是待办项。

7. **版本一致性门禁**：`scripts/check-version-sync.ps1`
   - 验证 `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`、`.version.json` 严格一致。
   - 聚合入口：`.meta/gates/check-governance.ps1`。
8. **提交卫生门禁 (Git Guard)**：`scripts/git-guard.ps1`
   - 拦截构建物（target, dist, coverage, tmp 等）、二进制文件（.rlib, .pdb 等）及超大文件提交。
9. **决策格式门禁（旧）**：`scripts/verify-decisions.ps1`
   - 验证 `docs/decisions/` 下存量 ADR 的 Status 行合法性与目录归属（该目录自决策替代形态规范起为
     只读档案，新决策一律进 `.agents/notes/`，见
     `implemented/process/2026-11-01-alt-governance-adaptation.md`）。

### 代码规模（跨平台，已接入 `npm run verify`）

10. **规模门禁（基线棘轮）**：`scripts/check-file-size.mjs` + 基线 `scripts/file-size-baseline.json`
    - 阈值：单文件 ≤3000 行 / 单函数 ≤200 行 / 单函数花括号嵌套 ≤8 层。
    - 判定：违规值 ≤ 基线值 → 放过（grandfathered）；超基线或基线无此条目 → exit 1。
      `exempt-*`（围栏过期/缺理由/日期非法）永不进基线，一律失败。
    - 接入：`npm run size:check`（`verify` 内）；接受新债用 `npm run size:check:update`，
      必须在同提交说明理由。决定见 ADR 0090。

## 负样本 spec 的两种形态
- **自包含**：从自身路径解析仓库根，不依赖调用者 cwd；样本一律建在 `mktemp -d` 临时目录，退出时清理；
  不改仓库任何文件；自包裹 `timeout 30`；不调用自身（防递归）。
- **诚实条款**：spec 只证明「门会响」，证明不了「抽样无偏」；抽样设计靠 review。`PONYGO_IN_SPEC=1`
  时打印 SKIPPED 并 exit 0（不计入 PASS）。

## 约定

- 本目录脚本一律 `chmod +x`（非可执行钩子会被 git 静默忽略，判据会假达标）。
- 门禁脚本**只读**仓库：不写仓库任何文件（`seal` 是唯一例外，且只追加 manifest）。
- 新增/删除门禁资产必须同提交更新本索引——索引与实现漂移等于门禁无人看守。
