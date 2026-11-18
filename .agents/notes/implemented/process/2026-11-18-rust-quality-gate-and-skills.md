# Agent Note: Rust 质量机制适配落地——BeautifulRust lints 棘轮门 + 双技能（rust-best-practices / rust-review）

Status: implemented

工单：B243（已 `resolve`）。2026-11-18 两路独立审核后按 P0/P1 重构：平台分册、口径指纹扩到 `clippy.toml`、
元数据校验、反 fail-open 护栏、门自检 34 例接线 CI/verify；Windows/macOS 分册与 `src-tauri` 扩面记 B244

## Problem

本仓 Rust 面（`crates/pony-agent-core` + `src-tauri`）已有 541 个 `.rs` 文件、约 37.9 万行，但质量约束只有一条：
CI 注入 `RUSTFLAGS=-D warnings` + `scripts/check-rust-test-lints.mjs`。全仓 `clippy` 引用为零——也就是说：

- **没有 lint 表**：`clippy.toml` / `[lints]` 均不存在，`clippy::pedantic`、`clippy::unwrap_used`、
  `unsafe` 注释、`#[allow]` 理由、bool 参数、`as` 截断等**可机械判定的规则从未被评估过**，
  存量债务规模未知，也无「新增债必红」的机制（`-D warnings` 只覆盖 rustc 默认 warn 集）。
- **没有项目级 Rust 规约**：写作与评审口径散在个人经验里；`rust-review` 类流程靠现写，不可复用、
  不可追溯（finding 无规则出处）。
- **量化底数**（本次实测，clippy 0.1.91，`--lib` 全表）：BeautifulRust 表在本仓产生 **10,115 条告警**；
  按应用模式裁掉 `[lib]` 类 lint 后 **3,741 条**；其中 **2,439 条**可由 `clippy --fix` 机器修。
  这个数量本身就是结论：**任何「一次性清零」或「纯靠 review」的方案都不成立**。

upstream 资产 `morganlinton/BeautifulRust`（MIT，v1.6.0）提供了可复用的一半：三个 Agent Skill、
一份经过「每条 lint 都被证明会触发」验证的 clippy 表（本机复跑 `scripts/check-lints.sh`：30/30 fired，
`EXIT 0`），以及 `check-skill.py` / `check-examples.py` 的自证工具链。

## Decision

采用「**规则表 + 注入式棘轮门 + 双技能**」三件套，不采用 BeautifulRust README 的默认装法：

1. **规则表真值源落 `.meta/lints/`**（`lints.toml` + `clippy.toml` + `README.md`），
   **不写 `[workspace.lints]`**，而由 `scripts/check-clippy-baseline.mjs` 在门禁侧注入 `-W/-A` 旗标。
   理由见备选 A：`[workspace.lints]` 与 CI 既有 `RUSTFLAGS=-D warnings` 在 rustc lint 层级上互相压制
   （特定 lint 的 `--warn` 覆盖 `warnings` 组级 `deny`），等于给既有「deny warnings」门开后门；
   且 1 万条 warn 会淹没本仓每次 `cargo check` 的输出。
2. **棘轮语义按 `(lint, 文件)` 记账**：存量值进基线（grandfathered），**新增文件/新增 lint/计数恶化 → exit 1**；
   **入账口径 = 表内 `allow` 之外一律入账**（含 `all`/`pedantic` 组的组成员与 clippy 默认组）——
   只认表内显式条目会让组拉起的违例静默漏账（首版实测少算约 1,000 条：2,052 → 3,870，2026-11-18 修正）；
   基线由 `--update-baseline` 生成、**人工 review diff 后**提交；基线里的「可瘦身项」（live < base）
   只提示不判红（与 `scripts/check-file-size.mjs` 的棘轮口径一致，本仓已有先例）。
3. **表内容哈希不符 → exit 2（fail-closed）**：改表必须伴随重基线，杜绝「表改了、基线没跟上」的静默漂移。
4. **应用模式裁剪**：本体是 bin（`pony` / `ponycli`）+ TUI，不是对外库 ⇒ 关闭 `[lib]` 类 lint
   （`missing_docs` 4,651 / `print_stderr` 230 / `missing_errors_doc` 481 / `missing_panics_doc` 92 /
   `exhaustive_enums` 188）与纯噪音项（`doc_markdown` 601 / `too_many_lines` 90，后者已由
   `npm run size:check` 的函数行数门覆盖）。裁剪**逐条写理由**在 `lints.toml` 行内注释。
5. **技能 vendoring 为 `.agents/skills/rust-best-practices/` + `rust-review/`**：不做字节级复制，按
   `.agents/skills/README.md` 五要素契约改写（触发式 description / 真相源清单 / 程序步骤 / **本仓真实
   校准样例** / 验证与报告格式），并挂本仓治理钩子（`npm run rust:lint`、`cargo-lane` 预算、测试分权、
   「报告不改代码」）。来源与改动记各自 `PROVENANCE.md`。
6. **不写 `skills-lock.json`**：其 `computedHash` 契约在仓内无校验器、且与已装文件 sha256 不吻合
   （实测 `build-with-exa` 装后文件 sha256 ≠ 表中值），写入等于伪造一个无法复现的哈希；
   来源改由 `PROVENANCE.md`（repo + commit + 上游文件 sha256）承载。
7. **不接远程 MCP、不装 rust-port、不装 Bevy 参考**：门禁判据不依赖公网（桌面/离线场景）；
   本仓无 C/Py 待迁面、无 `bevy` 依赖。
8. **基线按平台分册（v2，2026-11-18 两路独立审核 P0 后修正）**：`cfg(windows)`/`cfg(unix)` 让各平台
   违例集合不同（本仓 28 个文件、约 24% 存量落在平台门控文件里），单平台基线在另一平台必假红。
   故 `platforms.<process.platform>` 分册，缺册即 exit 2 并要求在该平台建册；CI 挂到
   `ubuntu-latest` 的 core-platform-check（与 linux 分册同平台），windows 面继续由 `-D warnings`
   与平台矩阵门承担。Windows/macOS 分册与 `src-tauri` 扩面记 B244。
9. **反 fail-open 护栏（审核 P1/P2 采纳；自检 34 例）**：口径指纹扩到 `clippy.toml`（改阈值不能零红放行 114 条）
   与基线元数据 `package`/`targets`（换包/换面不能判绿）；0 诊断、实测 < 基线 50%、未知 lint（E0602）、
   符号链接入口、`--from-json` 写正式基线、无源码位置的可计数诊断、cargo 非零退出 —— 全部归 exit 2；
   重基线只有在纯收缩时自动放行（上涨或首次建册须显式 `--allow-growth`）；门自检 34 例接进 CI 与 `verify`。
10. **「cargo 增量缓存漏报」被实测证伪**：cargo 对 fresh 单元重放缓存诊断——连跑两次门 `--json`
   得到逐项相同的 `liveTotal`/`messageCount`/`perLint`。故不采用"每次强制全量重编/清 target"的备选
   （代价是 CI 从秒级回落到分钟级），仅保留 0 诊断与 <50% 两道护栏兜底。

## Alternatives considered

- **A. 照 upstream 推荐：paste `[workspace.lints]` + `cargo clippy -- -D warnings`。** 落选：
  (i) 与既有 `RUSTFLAGS=-D warnings` 的 rustc 层级语义冲突（特定 lint 的 `--warn` 会覆盖组级 `deny`，
  使既有严格门在这些 lint 上静默失效）；(ii) 10,115 条 warn 会让每次 `cargo check`/`cargo build`
  输出不可读，开发回路直接失效；(iii) 无法区分「存量」与「新增」，等于用红海逼人 `#[allow]`。
- **B. 一次性清零（大爆炸修复 3,741 条）。** 落选：违反最小闭包原则，触及 379k 行里绝大多数文件，
  单次变更无法证明行为等价、评审面不可控；且 `clippy --fix` 的 2,439 条里有语义敏感项
  （`as` 转换、`redundant_clone`），不能盲跑。
- **C. 只装技能、不装 lint 门。** 落选：技能是语义约束（靠 review），无法承担「新增债必红」的机械判据；
  9 千条存量恰恰证明纯靠 review 的约束力为零。
- **D. 装远程 MCP（`mcp.beautifulrust.dev`）做规则查询。** 落选：把门禁/评审判据押在公网服务上，
  离线与桌面分发场景不可用；需要时可由 `mcp/` 自托管，属后续独立决策。
- **F. 单平台基线 + 把 CI 步骤 downgrade 成 `continue-on-error`/仅提示。** 落选：那是假豁免——
  门在别的平台上恒假红或恒绿，等于把"新增债必红"的承诺改成"看心情"；平台差异只能靠分册表达。
- **E. 新开 `.meta/gates/clippy.sh` 走 ponygo 门禁宇宙表。** 落选：`.meta/gates/` 由 `ponygo init`
  托管（宇宙表不可变，A1–A9），实例门虽有先例（`scratch-hygiene.sh`），但本门需要 Node 解析
  cargo JSON 流，与 `scripts/check-file-size.mjs` 形态一致，放 `scripts/` 更省一层壳。

## Consequences

- 新增：`.meta/lints/{lints.toml,clippy.toml,README.md}`、`scripts/check-clippy-baseline.mjs`（门）、
  `scripts/check-clippy-baseline.spec.mjs`（负样本自检）、`scripts/clippy-baseline.json`（基线）、
  `.agents/skills/rust-{best-practices,review}/`（含 `PROVENANCE.md` 与 upstream 5 篇 digest（bevy 未装））。
- CI `core-platform-check` 的 Linux 分支新增 `rust:lint:selftest` + `rust:lint`（与 linux 分册同平台）：
  首次约 3 分钟（本机实测），命中 `target-check` 缓存后增量秒级；本地不挂 pre-push（10 秒预算不变），
  重型跑法仍走 `scripts/cargo-lane.sh`。`npm run verify` 同步加入自检与门。
- 门自检 34 例（桩 cargo 验证 RUSTFLAGS 被清空、符号链接入口、平台缺失、阈值变更、空流、收缩、
  未知 lint、无位置诊断、路径归一化、重基线拒写与建册闸门），`npm run rust:lint:selftest` 即时可跑。
- 存量 **3,870 条 / 1,336 键 / 119 种 lint**（`platforms.linux`，实际扫描面 `--lib --bins` 实测；
  lib 面 3,072 / bin 面 798；按 `(lint, 文件)` 聚合）
  成为**待办地图**：`README.md` 按波次给出收口顺序
  （波 1 零争议卫生 / 波 2 类型与转换 / 波 3 需要设计判断）。基线只许删条目或下调计数。
- 提升路径（渐进，不一次性迁移）：某 lint 的基线清零后，把该条从 `.meta/lints/lints.toml` 迁入
  `[workspace.lints]`，由既有 `-D warnings` 原生强制；迁移是一次独立决策（改表即需重基线 + ADR）。
- 代价与残余风险：工具链升级可能新增告警 → 门按棘轮判红，需人工 review 后重基线（预期行为，非缺陷）；
  `RUSTFLAGS` 被门主动清空（门自持 lint 级别），因此门不反映用户本地 `-D warnings` 的严格度。
