# PROVENANCE —— rust-best-practices（本仓自有技能，源自 BeautifulRust）

## 上游身份

| 项 | 值 |
|---|---|
| 仓库 | BeautifulRust（<https://github.com/morganlinton/BeautifulRust>） |
| 本地副本 | `/tmp/BeautifulRust`（grafted clone） |
| commit | `b9718436d77ee7bcf17cdcb6aa27628feb95d97a`（`b971843`） |
| 版本 | **v1.6.0**（`plugin.json` 的 `version` + `CHANGELOG.md` 的 `## [1.6.0] - 2026-11-08`） |
| 许可证 | **MIT**（`LICENSE` = `fe7214be…`，Copyright (c) 2026 Morgan Linton） |
| 取用范围 | 仅 `skills/rust-best-practices/` 的规则本体与 `references/` 五篇 digest；不放宽许可证条件（保留著作权与许可声明） |

## 上游文件 sha256（逐个）

| 上游路径 | sha256 | 本仓处置 |
|---|---|---|
| `skills/rust-best-practices/SKILL.md` | `f9446430549a64d9fb2d480c875c4920b1d98c2540b937bd41448200b2639f6a` | **改写**为本仓 `SKILL.md`（中文 + 五要素 + 应用模式裁剪） |
| `skills/rust-best-practices/references/google-rust-style-guide.md` | `cfe61cd554c9dc32b412bb22dec7bd58e9eb4b3931db79124a61b8c2f573542c` | **逐字节复制**到 `references/` |
| `skills/rust-best-practices/references/rust-api-guidelines.md` | `392a94ed5151a7eeaf6f0d0f0cbd29904fa41ae4c4ae5ae98d79e7da3ba2a8af` | 逐字节复制 |
| `skills/rust-best-practices/references/effective-rust.md` | `c0fe4ea29d2601224ac68b307d8c6137dd932b2565c141c1527c88ba1c5d5415` | 逐字节复制 |
| `skills/rust-best-practices/references/rust-design-patterns.md` | `3102f45153ac7c75179ee43b12d143b33e4aaba9b2209c454dd4f0542cae7ae7` | 逐字节复制 |
| `skills/rust-best-practices/references/microsoft-rust-guidelines.md` | `648cfee06cea304e3d9c525e6de9047c296ce353e156a8117111094418ccc4b8` | 逐字节复制 |
| `skills/rust-best-practices/references/bevy.md` | `f8a96bccecd2e3faa2d79d206c24061ac9c5eb2dab3269573717b910ba5e8899` | **未安装**（本仓无 Bevy 依赖，见下"未装项"） |
| `lints/lints.toml` | `ec06c539416d1f807e64d93048f18ae3646ee1c20a03dd7dd232183dd11be41e` | **不复制**：lints 表迁至本仓 `.meta/lints/lints.toml`（由 Lead 落地，含本仓裁剪与实测基线） |
| `lints/clippy.toml` | `012446990fbaa1b4705319c7cf95ed59ffbd14ac7ca9ec4b907f27a0ead82c42` | 同上，迁至 `.meta/lints/clippy.toml` |
| `LICENSE` | `fe7214bea08bfb3d6b8c34841b54f7bc8ee84b080c83888aed9b0df321705d7c` | 仅记录出处（MIT 条款随上游仓库保留） |

复制后的本仓文件 sha256 与上表上游值**逐字节相同**（`sha256sum .agents/skills/rust-best-practices/references/*.md` 可复核）：
复制件不改写、不加中文、不删行。

## 本仓改动清单（相对上游）

1. **五要素改写**（`.agents/skills/README.md` 契约）：触发式 `description` 内联在 frontmatter（含 Rust/cargo/crate/.rs/借用检查/clippy/rustfmt 等触发词）；`## 真相源` 列本仓真实路径；`## 步骤` 的机械部分写成可复制命令、语义判断显式标"靠 review"；`## 校准样例` 正反例成对且锚 `file:line`；`## 验证与报告` 给自证命令与报告格式。frontmatter 只保留 `name` + `description`（未加 `disable-model-invocation`：本技能是写作面常驻流程，不是用户点名才跑的审查器）。
2. **中文语境改写**：术语与既有技能（`write-adr`、`change-review`）一致（"靠 review"、"真相源"、"校准样例"、"交付前检查清单"）。
3. **应用模式裁剪**（本仓特有）：明确本体是 bin（`pony`/`ponycli`）+ TUI，不是对外库；[lib] 类 6 条 lint 已关且理由写死为"日志与终端输出是产品行为"；禁止建议 `#![deny(warnings)]`；禁止"为过 pedantic 改公开 API"。同时区分"噪音/重复覆盖"的另 6 条，不混称。
4. **规则本体压缩但不删**：五来源全部核心规则保留（心法、机械面、所有权与借用、类型与 API 设计、错误与 panic、日常惯用法、并发与 async、unsafe 与 FFI、crate/module/依赖、文档、测试、性能、检查清单共 13 节），仅合并同主题条目、补齐本仓命令与路径；上游 511 行 → 本仓 230 行（含五要素与校准样例）。
5. **校准样例整体替换为本仓真实代码**：上游的 bevy/inventory 样例全部丢弃，改用 6 组可 `grep -n` 复核的 `file:line` 锚点（`agent/recovery.rs:128-130`、`acp/transport.rs:737`、`bin/tui/terminal.rs:409-411`、`bin/tui/queue.rs:276`、`bin/tui/task_center.rs:316`、`bin/tui.rs:4` 及其正例）。每组写明"差异维度 = 判定依据"，不用形容词当判定线。
6. **命令面接本仓门禁**：`npm run rust:lint` / `rust:lint:update` / `rust:lint:selftest`、`npm run cargo:check:shared`、`npm run cargo:test:shared -- --package=pony-agent-core --no-fail-fast`、重型构建 `bash scripts/cargo-lane.sh <cargo 参数>`、规模 `npm run size:check`。
7. **lints 表迁出**：不复制上游 `lints/lints.toml`、`lints/clippy.toml`，改为引用本仓 `.meta/lints/`（一处家，含本仓实测数字 10,115 / 3,741 / 2,439 的出处说明）。
8. **性能节诚实化**：本仓无 criterion/divan、无 `benches/`；性能证据链只写 `crates/pony-agent-core/examples/perf_probe.rs` + `target-perf` 槽位；明确 `npm run size:check` 只管规模不管性能，禁止编造不存在的基准工具。

## 未装项与理由

| 未装 | 理由 |
|---|---|
| `references/bevy.md` | 本仓 Cargo 清单 `bevy` 零命中（无 ECS 依赖）；装进来只会让评审产出虚假 finding（B0001 查询冲突、system 参数按值传等在本仓无对应面）。文件 sha256 仍记录在上表，便于日后若引入 ECS 时按图索骥。 |
| 上游 `rust-port` 技能 | 本仓当前无 C/C++/Python 来源代码树，无迁移 diff 可审；适配它等于凭空造场景。 |
| 上游 `evals/`（各技能的 case/graders） | 上游用 prompt+grader 评测技能行为；本仓技能契约是五要素 + 校准样例（人工/grep 可核），评测机制不同，搬 case 会产生"两条真相源"。 |
| 上游 `lints/*.toml` | 本仓 lint 唯一真源是 `.meta/lints/`（由 Lead 落地并含实测基线）；复制一份即漂移。 |

## 维护约定

- 校准样例里的 `file:line` 会随代码漂移：改到样例锚点所在代码时，同提交更新本文件与 `SKILL.md` 的样例（否则样例从"证据"退化为"传说"）。
- 上游升级（新 vX.Y.Z）时，按本文件 sha256 表逐文件比对：digest 变则重新复制，规则变则重审规则本体；**不得只改版本号**。
- 本技能与 `rust-review` 共用同一套规则；规则只在 `rust-best-practices` 维护一份，评审技能引用不复制（复制即漂移）。
