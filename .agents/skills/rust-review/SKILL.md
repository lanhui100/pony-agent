---
name: rust-review
description: 何时用：评审既有 Rust 代码——一个 .rs 文件 / 模块 / crate / 工作区 diff / 单个提交 / PR——并要求产出「改什么、破哪条规则、最小怎么改」的报告时；用户说评审、审查、code review、"这段 Rust 为什么不通顺/地不地道/有没有坑"、贴 clippy 输出求解读，都算。只出报告不改代码（用户明确要求"顺手改"才改）。
---

# rust-review —— 既有 Rust 代码的评审（报告，不是重写）

评审面与写作面共用同一套规则：规则只在 `.agents/skills/rust-best-practices/SKILL.md` 维护一份，
本技能**引用不复制**。产出是一份可执行的报告，改不改由读者定。

## 真相源

- `AGENTS.md` —— 常载命约、门禁速查（评审结论要能对上 §3 的命令）
- `.agents/skills/rust-best-practices/SKILL.md` —— 规则本体（唯一一份，含本仓应用模式政策）
- `.meta/lints/lints.toml` —— **在册 lint 唯一清单**：引用规则时先在这里找家
- `.meta/lints/clippy.toml` —— 阈值（当前只有 `max-fn-params-bools=0` 与 3 条测试豁免；**未**设 `msrv` / `disallowed-methods`）
- `.meta/lints/README.md` —— lint 面实测数字与门语义（引数字时标"来自本文件"，不自行计数）
- `.agents/notes/implemented/process/2026-11-18-rust-quality-gate-and-skills.md` —— 门禁与两技能的决策记录
- `docs/architecture/` —— 结构真源（评审并发/运行时问题前先读相关篇，如 `runtime-ownership-split.md`）
- `references/`（在 `rust-best-practices/` 下）—— 五来源 digest 原文，仅在需要复述规则理由时读

## 输入契约（缺 scope 就停下报告，不猜）

评审对象必须落在下面三选一，缺了就停：

| scope | 允许的输入 | 边界 |
|---|---|---|
| **diff** | 工作区 diff / 提交 sha / 分支 / PR | 只评改动行及其破坏面，不评整文件 |
| **文件清单** | 一个或多个 `.rs` 路径 | 只评这些文件的语义面，不外扩 |
| **crate** | 一个 crate 目录 | 全 crate 读，但要说明优先面（安全/错误/API） |

- 缺 scope（"看看这个项目有没有问题"）→ 回一句要 scope（diff / 文件 / crate），不开评。
- 多主题混装（一次提交同时改 CLI 契约 + TUI 渲染 + unsafe 平台码）→ 要求拆 scope，一次评一个主题；
  混装本身在报告里记一条 non-blocker。
- 读 `Cargo.toml` 确认 edition / `rust-version` / feature 面：**不要建议项目用不了的语法或 lint**
  （本仓 `crates/pony-agent-core/Cargo.toml` 只写 `edition = "2021"`，无 `rust-version`——`#[expect]`、`let-else` 之外的新语法先确认）。

## 治理钩子（评审前必须过，顺序不能倒）

1. **先看 scope 再评**：见上节。没 scope 就没有评审。
2. **测试分权**：评审者**不裁决自己的实现**；验收测试（`crates/*/tests/**`）是冻结面，**只读**。
   发现"测试挡着修法"时，报告里写"实现侧应改，测试侧由测试分权方按新契约重锚"，不自行把测试改绿；
   把"改测试让 lint 安静"列为 must fix（伪装问题被消除）。
3. **先机械面，后设计面**：能跑就先跑机械命令（下节），把机器能判的判完，注意力留给设计；
   **不贴 clippy 原始输出**，按 lint 聚合成一条 finding（同一 lint 的 N 处命中合成一条，列出行号）。
4. **规则要有家**：引用规则时优先引 `.meta/lints/lints.toml` 的在册条目（clippy lint 名）；上游五来源
   （API Guidelines 编号 `C-GETTER`、Effective Rust 条目号、Microsoft `M-PANIC-ON-BUG`、Google 风格指南）
   只作**背后依据**，可以附在括号里但不得顶替本仓 lint 名。
5. **已关类别不得 must fix**：本仓应用模式关闭的 [lib] 类 6 条（`missing_docs` / `missing_errors_doc` /
   `missing_panics_doc` / `exhaustive_enums` / `print_stdout` / `print_stderr`）不得作为 must fix 项，
   最多 `consider` 并写明"本仓应用模式关闭该 lint（理由：日志与终端输出是产品行为）"。
   另有噪音/重复覆盖 6 条（`doc_markdown`、`too_many_lines`、`must_use_candidate`、`module_name_repetitions`、
   `module_inception`、`enum_glob_use`）同样不得进 must fix。

## 步骤

1. **定 scope**（上表）+ 判定应用/库档：本仓是 bin（`pony`/`ponycli`）+ TUI 的产品，`lib.rs` 只是内部复用面
   ⇒ 按"应用"档评（具体类型、`anyhow`-式错误、不搞投机泛化）；只有对外可复用模块面才用库档的严格 API 规则。
   报告开头一句写明"按应用档评 / 按库档评"。
2. **跑机械面**（可用时；不可用就在报告里写明"未运行，按阅读评审"）：

```bash
npm run rust:lint          # clippy 棘轮门（在册 lint + 阈值都在这条命令背后，exit 1 = 有新增/恶化）
```

   **格式面不是门禁面**：本仓未纳管 rustfmt（无 `rustfmt.toml`、CI 无 fmt 门），`cargo fmt --check`
   当前有 1.7 万处既有差异 ⇒ **不得**作为"本次变更不合格"的判据，也不得把存量格式差异写成 finding
   （那是把噪音当 bug，且等于夹带全量重排）。仅两种情形可以提格式：① 用户点名格式面——只报规模，不逐条列；
   ② diff 评审——只看**改动行**是否落在既有 fmt 差异里（新增行与邻行风格不一致才提，最多 consider）。
   手动跑 clippy（不经门）**必须**带 `CLIPPY_CONF_DIR=.meta/lints`（否则 `clippy.toml` 的阈值不生效，
   如 `max-fn-params-bools = 0`）；更省事的是直接用上面那条门命令。门的记账口径是"`lints.toml` 里
   `= "allow"` 之外一律入账"，因此**源文件 `#[allow]` 不能让告警退出记账**；表改了却没重基线 → exit 2。
   构建与测试**不重跑**：行为证据从门禁/CI 输出取，或要求提供；重型构建属 Lead 的 `--heavy` 独占活
   （`bash scripts/cargo-lane.sh`，评审者不抢 lane）。
3. **读设计面**（按优先级；前两类是真 bug 的产地）：

   | 优先级 | 看什么 | 本仓已见的锚点（示例，非清单） |
   |---|---|---|
   | 1 正确性与 soundness | `unsafe` 无 `// SAFETY:`、`transmute`、`unsafe impl Send`、可达 panic（`unwrap`/索引/`as` 回绕/溢出）、trait 契约（手写 `Ord` 配派生 `PartialEq`、`Hash` 与 `Eq` 不一致）、锁（`MutexGuard` 跨 `.await`、持锁调用户闭包、`into_inner` 复中毒锁）、析构 panic/阻塞 | `bin/tui/terminal.rs:409`、`agent/process.rs:145` |
   | 2 错误处理 | 公开 `Result` 的 `E` 不可用（`()`/`String`/`Box<dyn Error>`）、错误丢信息（`map_err(|_| ..)`、无 `source()`）、用 panic 报预期失败、`catch_unwind` 当控制流 | `agent/recovery.rs:130` |
   | 3 API 设计（库档严格） | `&String`/`&Vec`/`&Box` 参数、公开签名 bool 参数、取所有权后又 clone、返回只用于返回的 `Vec`、公开字段、`Rc`/`RefCell`/`Arc` 进签名、缺 `Debug`/`Display`/`Default`/`FromStr`、`get_` 取数、`as_`/`to_`/`into_` 与代价不符、缺 `#[non_exhaustive]`、该 seal 的 trait | `bin/tui/queue.rs:276` |
   | 4 惯用与清晰 | 只为过借用检查的 clone、check-then-unwrap、`+` 拼字符串、长到需要 loop 的管道、`HashMap` 迭代序泄漏到输出/测试、`#![deny(warnings)]`、无理由 `#[allow]`、外部 crate glob | `acp/transport.rs:737` |
   | 5 文档与测试 | 公开项无文档（**本仓 consider 封顶**）、`# Errors`/`# Panics`/`# Safety` 缺位（consider 封顶）、doc example 用 `unwrap`、无测试或只复述常量 | — |

4. **聚合**：同一 lint 的重复命中合成一条（列出全部 `file:line`）；跨 lint 的同一根因也合成一条
   （例：4 个 bool 参数既是 `fn_params_excessive_bools`，又是"模式该用 enum"的设计问题）。
5. **写报告**（格式见下）。

## 分档判定线（must fix / should fix / consider）

| 档 | 判定线（命中任一） | 本仓例子 |
|---|---|---|
| **must fix** | 真 bug / soundness 洞 / 可达 panic / 一旦发布即成破坏性变更的 API 错 / 与已记录 ADR 或契约直接冲突 / 把问题藏起来（改测试、加无理由 `allow` 掩盖新增告警） | `unsafe` 无 `SAFETY`（`bin/tui/terminal.rs:409-411`） |
| **should fix** | 能跑、但有真实成本：错误信息变差、测试更难写、调用点不可读、行为在边界处不对、lint 在册且命中 | 4 bool 参数（`bin/tui/queue.rs:276`）；无理由抑制（`acp/transport.rs:737`） |
| **consider** | 风格、命名、打磨；或**本仓已关掉的 lint 类别**（哪怕上游规则认为重要） | `#[allow(dead_code)]` 无理由但属冻结面（`acp/transport.rs:235`，其上有散文说明） |

- **可达/不可达**决定 must fix 与否：写清"什么输入/什么路径能触发"，不给触发条件的一律降到 should fix。
- **尊重意图**：代码里有注释说明是刻意为之（冻结面、平台限制、契约要求）→ 放 consider 并写权衡，不当 must fix。
- **比例**：短文件 5–10 条足够；不凑数，也不拿风格淹没真 bug。
- **"已做得好的"必须有**：2–4 条具体事实（不是"代码整洁"这种形容词）。

## 报告格式

~~~markdown
# Rust review: <评的是什么：文件名 / diff 范围 / crate>

<一段：代码做什么、按应用档还是库档评、总体结论、最该修的一件事；说明 `npm run rust:lint` 是否跑了及其退出码。>

## Must fix
### 1. <短标题> (`src/file.rs:42`)
**规则：** <本仓 lint 名，如 `clippy::undocumented_unsafe_blocks`>（背后依据：<API Guidelines 编号 / Effective Rust 条目 / Microsoft 编号 / Google 风格指南，可省>）
**问题：** <错在哪 + 具体后果，1–2 句>
**最小修法：**
```rust
<只写要改的那几行>
```

## Should fix
### 2. ...
（同一 lint 的多处命中合并在一条里，列出全部 `file:line`）

## Consider
### 3. ...

## 已做得好的
- <2–4 条具体事实：哪个文件哪一行做对了什么，为什么值得保留>
~~~

- 每条 finding 三件套缺一不可：`file:line`（无单点位置写 `whole file` 并说明）、**规则名**、**最小修法**（代码，不重写整函数）。
- diff 评审按"哪几行"定位：`src/file.rs:120-131`。
- 报告不改代码；用户明确要求改时才动，且改完在报告里逐条对应"哪条 finding → 改了哪个文件"。

## 校准样例

判定依据就是每组标注的"差异维度"，不要用形容词当分界线。

- **正例（按 lint 聚合）**：`crates/pony-agent-core/src/acp/transport.rs:737` 的 `#[allow(clippy::too_many_arguments)]`
  + 全仓同类无理由抑制（`grep -rn '#\[allow(' crates/pony-agent-core/src --include=*.rs | grep -v '//'` = 124 行行内无理由，
  其中 `clippy::too_many_arguments` 49 处、`dead_code` 66 处；与 `.meta/lints/README.md` 的 lint 侧计数 131 口径不同，不混用）
  → **一条** should fix：「抑制无理由/无退出口」，列出 `transport.rs:737` 等代表行号 + 给 `#[expect(..., reason = "…")]` 或归组参数的修法。
- **反例（把原始输出当报告）**：把 `npm run rust:lint` 的 100+ 行 clippy 输出原样贴进报告，或按"每个告警一条 finding"
  写成 124 条 → 读者无法排序，真 bug 被淹没。**差异维度 = 是否按 lint/根因聚合、是否给排序**。

- **正例（分档正确）**：`crates/pony-agent-core/src/bin/tui/queue.rs:276` 的
  `should_drain_queued(is_streaming: bool, ui_idle: bool, queued: usize, turn_in_flight: bool, dispatch_pending: bool)`
  → should fix：命中在册 `fn_params_excessive_bools`，后果是调用点 `should_drain_queued(false, true, 3, false, false)`
  （`crates/pony-agent-core/tests/tui_streaming_input_acceptance.rs:477`）不可读；修法给"两变体 enum 或拆成两步判据"，
  并注明改签名会动冻结验收测试 → 测试侧由测试分权方重锚。
- **反例（错档：把风格升成 must fix）**：把上面这条写成 must fix（它不产生运行时错误）而漏掉
  `bin/tui/terminal.rs:409-411` 的 `unsafe { libc::raise(libc::SIGSTOP) }` 无 `// SAFETY:`（真正的安全性问题：
  在册 `undocumented_unsafe_blocks` 的存量命中——在棘轮基线内，**不会**让 CI 变红，但新增同形态必红；
  评审按 must fix 处理，理由是 soundness 论证缺失，不是"CI 红"）。
  **差异维度 = 是否能指出"什么输入/路径会坏"**：说不出的降 should fix，说得出的才是 must fix。

- **正例（已关 lint 只进 consider）**：评审 `crates/pony-agent-core/src/bin/tui.rs` 时发现大量 pub 项无 rustdoc
  → consider，写明"本仓应用模式关闭 `missing_docs`，理由：日志与终端输出是产品行为，不是对外 API 承诺"。
- **反例（错档：把已关类别当 must fix）**：把 `missing_docs` / `missing_errors_doc` / `print_stdout`（TUI 的 stdout 就是画面）
  列为 must fix，等于要求改产品行为去满足一条本仓已明确关闭的库规则。**差异维度 = 该 lint 是否在 `.meta/lints/lints.toml` 的在册名单里**。

- **正例（证据面匹配，不越权跑重型门）**：diff 只改渲染函数 → 报告写"已跑 `npm run rust:lint`（exit 0）与
  `npm run rust:lint:selftest`（exit 0）"，行为证据引用门禁输出，不本地重跑 release/全量测试；
  格式面按"非门禁面"处理，只在用户点名或改动行风格不一致时提。
- **反例（越权/造证据）**：为评审自己起 `--heavy` release 构建抢 lane，或写"应该没问题"当测试结论；
  以及把"reviewer 顺手改了 `crates/pony-agent-core/tests/**` 让断言变绿"当修法。
  **差异维度 = 结论背后的命令输出/契约行号是否真实存在，且是否落在评审者权限内**。

- **正例（规则有家）**：finding 写「规则：`clippy::as_conversions`（在册）+ `cast_possible_truncation`」，
  锚 `crates/pony-agent-core/src/bin/tui/task_center.rs:316` 的 `content_lines(state).len() as u16`，修法给
  `u16::try_from(..).unwrap_or(u16::MAX)`（同仓正确写法见 `crates/pony-agent-core/src/bin/tui.rs:2094`）。
- **反例（规则无家/无定位）**：写"这里类型转换不太安全，建议改一下"——没有 `file:line`、没有 lint 名、没有最小修法；
  或者只引上游"Rust API Guidelines C-…"却不给本仓在册 lint 名。**差异维度 = 三件套（位置/规则名/最小修法）是否齐备**。

## 验证与报告（评审者的自检）

- 收尾自检三问：**每条 finding 有三件套吗**（`file:line` + 规则名 + 最小修法）？**每档都有判定线证据吗**
  （must fix 说得出触发路径）？**「已做得好的」写了吗**（≥2 条具体事实）？
- 机械面引用必须贴**命令 + 退出码**原文（`npm run rust:lint` → exit 0/1），不贴告警正文。
- 交付物 = 报告本身；评审不写盘、不改码、不更新基线（`npm run rust:lint:update` 属实现方/Lead 的动作）。
- 报告落盘位置按用户指定；未指定就贴在对话里，不擅自往仓库里写文件（写入面由治理规程约束）。
