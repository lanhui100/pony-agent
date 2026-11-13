# Agent Note: 决策门随 ADR 语料迁移切换判定真源

Status: implemented

## Problem

2026-11-12 的 ADR 语料迁移把决策记录从「`docs/decisions` 的 NNNN 编号 + README 索引」形态
搬到 `.agents/notes`（日期命名、按 lifecycle/class 分目录、归档以哈希 manifest 封印）。迁移改了
语料与 `AGENTS.md` 的判定真源声明（`npm run adr:check` → `write-adr/verify-note.sh` +
`archive-verify.sh`），但 **CI 与 release 调用的入口 `scripts/verify-decisions.ps1` 仍然只转发
`check-decisions.mjs`**——后者校验的是旧布局，找不到 `docs/archive/decisions` 就以 exit 1 收口。

后果（实测）：`main` 上 `CI / verify` 持续红，`release-signed-updater`（tag `v0.1.191`）在第一步
「决策记录校验」即失败——**发版被一个语料迁移的收尾遗漏卡住**，而代码本身是绿的。

这属于「门禁是承诺的可验形态」的典型失效：迁移只搬了语料与文档，没搬判定入口。

## Decision

`scripts/verify-decisions.ps1` 改为按**语料布局**分派，两分支都 fail-closed：

1. 存在 `docs/archive/decisions` 或 `docs/decisions` ⇒ 仍转发 `scripts/check-decisions.mjs`
   （保留对旧布局分支/仓库的判定能力，R1–R7 一字不改）。
2. 否则语料已迁移到 `.agents/notes` ⇒ 转发 `AGENTS.md` 已声明的判定真源：
   `.agents/skills/write-adr/verify-note.sh`（整树语料）+ `.meta/gates/archive-verify.sh check`（归档封印）。
3. 语料目录与判定脚本都找不到、或缺 `bash` ⇒ exit 1 并打印可操作错误，绝不「找不到即放行」。

`check-decisions.mjs` 与两份 workflow 保持原样（`.github/workflows/ci.yml`、`release.yml` 仍只调用
`./scripts/verify-decisions.ps1`），因此本改动是单文件收口，未扩散到 CI 定义面。

机械证据：`bash .agents/skills/write-adr/verify-note.sh` → exit 0；
`bash .meta/gates/archive-verify.sh check` → exit 0（117 条归档件哈希一致）；
`bash .meta/gates/backlog-gate.spec.sh` → exit 0（pre-commit 接线用例不变）。

## Alternatives considered

- **让 `check-decisions.mjs` 直接认 `.agents/notes` 布局**：否决。该脚本的 R2/R4/R6 建立在
  「编号 + README 索引」语料模型上，`.agents/notes` 没有编号索引，硬塞等于把判据改成空转。
- **两份 workflow 各加一条 `npm run adr:check`、删掉 `verify-decisions.ps1`**：否决。同一门禁出现
  两个入口，未来再迁移又会漏一处；入口唯一、内容可切换才是可维护形态。
- **新建空的 `docs/archive/decisions` 让旧门通过**：否决。那是让门禁空转的假绿，比红更危险。
- **在 release workflow 上跳过决策校验**：否决。门禁不能为某条流水线破例；要改的是入口指向，不是门的强度。

## Consequences

- CI `verify` 与 release 的第一步恢复可判定：旧布局仓库照旧走 R1–R7，新布局仓库走 note 门 + 归档封印。
- 未来若再次迁移语料，唯一改动点是 `scripts/verify-decisions.ps1` 的分派表，workflow 与 gate spec 不动。
- 本机无 pwsh 时该入口无法自测，靠 CI/Windows runner 实测（`verify-note.sh` / `archive-verify.sh`
  已在本地直接跑通，分派逻辑仅一层路径判断）。
