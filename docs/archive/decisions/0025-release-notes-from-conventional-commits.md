# 0025 基于 Conventional Commits 的发布说明生成器

Status: implemented

## 背景

`.github/workflows/release.yml` 在每次推送 `vX.Y.Z` tag 时执行发版：`gh release create` 使用硬编码
`--notes "Automated signed release (Windows desktop)"`（原第 111 行），`latest.json` 的 `notes`
字段同样硬编码为空串（原第 145 行）。结果是 GitHub Release 页面与 Tauri 更新清单均不携带本次
发布的变更信息，用户无法判断新版本内容。仓库提交已遵循 conventional commits 约定
（`feat`/`fix`/`perf`/`docs`/`chore` 等，如 `chore(release): bump version to 0.1.114`），
具备从本地 git 历史结构化生成 release notes 的事实基础。workflow 运行于 `windows-latest` + pwsh，
Node 22 可用且无额外依赖面。

## 候选方案

1. **维持硬编码 notes（现状）**：零改动成本，但发布页永远无本次变更信息，且 `latest.json.notes`
   为空串会被下游更新流程以空文案展示，与"发布即告知变更"的预期长期背离，否决。
2. **改用 `gh release create --generate-notes` / `gh --generate-notes`（GitHub 自动生成）**：
   单命令即可产出官方摘要，落地成本低。但生成内容由 GitHub 按 commits/PR 摘要自动拼装，分组规则、
   文案语言、条目粒度均不可控且随 GitHub 端行为漂移；它依赖 GitHub API（网络调用、需令牌路径），
   与 `latest.json` 需要的是**同一份可控文本**无法保证一致（两处各自生成必然漂移），否决。
3. **自建跨平台 Node 生成器 `scripts/release-notes.mjs`（选定）**：以本地 git 历史
   （conventional commits）分组生成确定性 markdown；纯本地 git 调用、零网络、无新增依赖与 secret；
   输出可直接作为 `--notes-file` 传给 `gh release create`，同一文件全文写入 `latest.json.notes`
   保证两处同源一致；失败即 fail，符合现有发布硬门禁风格。

## 决策

`scripts/release-notes.mjs` 提供四个导出：`parseCommit(subject)`、`groupCommits(subjects)`、
`buildReleaseNotes({ subjects, from, to })`（纯函数、确定性、零网络）与 CLI 入口 `main(argv)`。
CLI 契约：`--repo-root <dir> --to <tag> [--from <tag>] [--out <file>]`；无 `--from` 时按 semver
自动探测前一 tag，无前一 tag 时回退为 `--to` 起全量历史并在输出中注明；git 子进程经
`execFileSync` 执行并带 60s 超时，失败以非 0 exit code 退出且 stderr 可诊断。分组固定五组
（feat→Features/新功能、fix→Bug Fixes/修复、perf→Performance/性能、docs→Docs/文档，
其余→Maintenance/维护），顺序固定、组内保留提交顺序；跳过 `chore(release):` 前缀的 bump 提交；
条目格式 `- <subject> ([short-sha])`，subject 做 markdown 转义（`#`、反引号、换行注入防护）；
空区间输出占位说明"本版本无用户可见变更"。`release.yml` 生成 notes 到文件后改用
`--notes-file` 传参（替换硬编码 `--notes`），`latest.json.notes` 字段从该文件全文读取
（免手工拼接，JSON 转义由 `ConvertTo-Json` 保证），与 Release 页 notes 保持同一真源。
本方案的 CLI 特定 NFR（git 超时 60s、零网络、确定性输出、UTF-8、exit code 契约、一次性 CLI
无需锁）记录于 `.dev-team/contract-matrix-release-notes.md`，不写入应用级
`.dev-team/nfr-baseline.json`。

## 影响

- CI 无新增 secret、无新增依赖、无需网络权限；发布流程仍为单 job（windows-latest）。
- 纯本地 git 调用，Windows 与 Linux 跨平台一致（Node 22 + `execFileSync` + fs）。
- 生成失败即 CI fail（失败即 fail），不再发布空 notes；`latest.json` 与 Release 页 notes
  严格同源一致。
- 仓库新增 `scripts/release-notes.mjs`（先以纯空桩冻结，由执行方以常规 ESM 实现替换）、
  契约矩阵 `.dev-team/contract-matrix-release-notes.md`；`release.yml` 的 notes 生成与
  latest.json 接线随之更新（原第 111 / 145 行硬编码移除）。