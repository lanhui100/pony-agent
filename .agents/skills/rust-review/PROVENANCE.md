# PROVENANCE —— rust-review（本仓自有技能，源自 BeautifulRust）

## 上游身份

| 项 | 值 |
|---|---|
| 仓库 | BeautifulRust（<https://github.com/morganlinton/BeautifulRust>） |
| 本地副本 | `/tmp/BeautifulRust`（grafted clone） |
| commit | `b9718436d77ee7bcf17cdcb6aa27628feb95d97a`（`b971843`） |
| 版本 | **v1.6.0**（`plugin.json` 的 `version` + `CHANGELOG.md` 的 `## [1.6.0] - 2026-11-08`） |
| 许可证 | **MIT**（`LICENSE` = `fe7214be…`，Copyright (c) 2026 Morgan Linton） |
| 取用范围 | 仅 `skills/rust-review/SKILL.md` 的评审程序与报告骨架；规则本体不取（引用 `rust-best-practices`） |

## 上游文件 sha256（逐个）

| 上游路径 | sha256 | 本仓处置 |
|---|---|---|
| `skills/rust-review/SKILL.md` | `3a2ee9874896d2f8f980b617e2b32ce3daff8275ec6ee12c2fb2710e2ae91a9f` | **改写**为本仓 `SKILL.md`（中文 + 五要素 + 治理钩子 + 校准样例换真实代码） |
| `LICENSE` | `fe7214bea08bfb3d6b8c34841b54f7bc8ee84b080c83888aed9b0df321705d7c` | 仅记录出处（MIT 条款随上游仓库保留） |

上游 `skills/rust-review/evals/`（`review-bevy-arena/`、`review-inventory-module/` 的 case/graders/夹具）
**未取用**，理由见下"未装项"。

## 本仓改动清单（相对上游）

1. **五要素改写**：触发式 `description` 内联（含"评审/审查/code review/贴 clippy 输出求解读"等触发语）；
   `## 真相源` 换成本仓路径；`## 步骤` 里机械步骤给可复制命令、语义判断标"靠 review"；
   `## 校准样例` 6 组正反例；`## 验证与报告` 给自检三问。frontmatter 只留 `name` + `description`。
2. **中文语境**：术语与 `write-adr` / `change-review` 一致（"真相源""校准样例""靠 review""non-blocker"式的分档表达）。
3. **输出契约强化**：明确"报告不改代码"（除非用户明确要求），三级分档 `must fix / should fix / consider`
   各给判定线，每条 finding 强制三件套 = `file:line` + 规则名（优先本仓在册 clippy lint 名）+ 最小修法，
   报告结尾强制「已做得好的」（2–4 条具体事实）。
4. **挂本仓治理钩子**（上游没有，本仓新增）：
   - **输入契约**：scope 必须落在 diff / 文件清单 / crate 三选一，缺 scope 停下报告、不猜（`skills/README.md` 五条纪律 2）；
   - **测试分权**：评审者不裁决自己的实现，`crates/*/tests/**` 只读，修法不得以"改测试变绿"结案；
   - **先机械面后设计面**：`npm run rust:lint` 先跑（**格式面明确为非门禁面**：本仓未纳管 rustfmt、无 CI fmt 门、存量差异 1.7 万处，不得把存量格式差异写成 finding），**不贴原始 clippy 输出**，按 lint 聚合成一条；
   - **规则有家**：优先引 `.meta/lints/lints.toml` 在册条目，上游五来源只作背后依据；
   - **应用模式豁免**：[lib] 类 6 条（`missing_docs` 等）与噪音 6 条不得进 must fix，最多 consider 且注明理由。
5. **删掉上游的 Bevy/bev 分支与"建议安装 BeautifulRust lints 表"一节**：本仓 `bevy` 零依赖；
   lint 表已由 `.meta/lints/`（本仓真源）落地，评审不再"建议装别人的配置"。
6. **证据面匹配**：评审不重跑重型构建（`--heavy` 属 Lead），行为证据取自门禁/CI 输出；
   重型 lane 由 `bash scripts/cargo-lane.sh` 统一调度（评审者不抢）。
7. **校准样例整体替换为本仓真实代码**（6 组，全部可 `grep -n` 复核）：`acp/transport.rs:737` +
   124 行无理由 `#[allow]` 的聚合、`bin/tui/queue.rs:276` 的 bool 参数分档、`bin/tui/terminal.rs:409-411`
   的 must fix、已关 lint 的 consider 封顶、证据面匹配（不越权跑重型门）、规则三件套（`bin/tui/task_center.rs:316`）。

## 未装项与理由

| 未装 | 理由 |
|---|---|
| 上游 `evals/review-bevy-arena/` | 夹具是 Bevy ECS 代码（查询冲突、system 参数），本仓无 Bevy 依赖，评测会逼出与仓库无关的 finding。 |
| 上游 `evals/review-inventory-module/` | 夹具是上游虚构的库模块（`Rc<RefCell>` API、`#![deny(warnings)]`）；本仓是 bin + TUI，用本仓真实 `file:line` 做的校准样例更贴近判定现场。 |
| 上游 `references/` | 本技能不持有规则副本（规则只在 `rust-best-practices/references/` 一份）；复制即漂移。 |

## 维护约定

- 规则只在 `rust-best-practices` 维护；本文件出现"规则名"时只引名字，不抄条文。
- 校准样例的 `file:line` 随代码漂移：改到锚点所在代码时同提交更新本文件与 `SKILL.md`。
- 上游升级时按 sha256 表比对 `skills/rust-review/SKILL.md`：变了就重读其评审程序（分档定义、报告骨架），
  **不得只改版本号**。
