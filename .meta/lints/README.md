# lints/ —— Rust lint 表（真值源）与棘轮门的口径说明

本目录是"什么算本仓合格的 Rust"的**唯一家**：`lints.toml`（允许/禁止哪些 lint）+ `clippy.toml`
（阈值与测试豁免）。门在 `scripts/check-clippy-baseline.mjs`，存量账在 `scripts/clippy-baseline.json`
（**按平台分册**）。门自检在 `scripts/check-clippy-baseline.spec.mjs`（34 个负样本，进 CI 与 `verify`）。

来源：适配自 [`morganlinton/BeautifulRust`](https://github.com/morganlinton/BeautifulRust) v1.6.0
（MIT，commit `b9718436`）的 `lints/lints.toml` + `lints/clippy.toml`。决策、备选与代价见 ADR
[`2026-11-18-rust-quality-gate-and-skills.md`](../../.agents/notes/implemented/process/2026-11-18-rust-quality-gate-and-skills.md)。

## 1. 为什么不是 `[workspace.lints]`（与上游默认装法的分歧）

上游 README 的装法是"把 `[lints]` 粘进 `Cargo.toml`，然后 `cargo clippy -- -D warnings`"。本仓不这么做，
因为本仓 CI 已有 `RUSTFLAGS=-D warnings` 这条更早的严格门：

- cargo 的 `[lints]` 会向 rustc 传 `--warn=<lint>`；而 rustc 的层级规则是**特定 lint 级别覆盖 `warnings`
  组级级别**，所以这些 lint 反而会逃出既有 `-D warnings`（静默削弱既有门，而非增强）。
- 本仓首扫即万条量级告警，写进 `Cargo.toml` 会让每次 `cargo check` / `cargo build` 的输出不可读，
  开发回路直接失效；也无法区分"存量"与"新增"。

因此本目录的表由门**自己注入**（`-W`/`-A`），只回答一个问题：**这次变更有没有新增债**。

## 2. 应用模式裁剪（形状不同，不是降标准）

本仓本体是 bin（`pony` TUI / `ponycli`）+ Tauri 壳，不是对外发布的库。`[lib]` 类 lint 在此形态下
产出的是噪声而非信号：

| 关闭的 lint | 首扫条数（lib 单面探针） | 为什么关 |
|---|---:|---|
| `missing_docs` | 4,651 | 公开项四散在内部模块，本仓不对外发布 API 文档；文档义务由 `docs/` 分层承担 |
| `doc_markdown` | 601 | 中英混排文档误报为主，属噪音（**非** [lib] 类） |
| `missing_errors_doc` | 481 | 不对外发布库，`# Errors` 契约无消费者 |
| `print_stderr` | 230 | TUI/CLI 的错误面向用户输出是产品行为 |
| `exhaustive_enums` | 188 | 导出枚举的 `#[non_exhaustive]` 演进保护，只在有外部使用者时成立 |
| `missing_panics_doc` | 92 | 同 `missing_errors_doc` |
| `too_many_lines` | 90 | 与 `npm run size:check`（文件 3000 行 / 函数 200 行 / 嵌套 8 层）重复覆盖（**非** [lib] 类） |
| `must_use_candidate` 等 4 条 | — | 纯风格建议，不接受它驱动公开 API 改动（**非** [lib] 类） |
| `print_stdout` | 0 | 同 `print_stderr`（当前 0 条，保留关闭状态以防 TUI 输出被误判） |

关掉 ≠ 不管：规模由 `size:check` 管、文档由 `docs/AGENTS.md` 的分层契约管、用户可见错误输出由
`docs/observability.yaml` 的归属管。**每一条关闭的理由都在 `lints.toml` 行内注释里，改关闭状态要先改理由。**

## 3. 实测底数（clippy 0.1.91 / rustc 1.91.1；只写实测值，不做跨口径推导）

| 口径 | 数值 |
|---|---|
| 首扫探针（`--lib` 单面，上游全表） | 10,115 条告警 |
| 同上，关 §2 的 6 条 `[lib]` 类后 | 3,741 条 |
| 其中 `clippy --fix` 可机器修 | 2,439 条 |
| **棘轮基线（实际扫描面 `--lib --bins`，`platforms.linux`）** | **3,870 条 / 1,336 键 / 119 种 lint** |
| 其中 lib 面 / bin 面 | 3,072 条（1,117 键）/ 798 条（219 键） |

探针与基线**不可逐项对账**（探针未启用本仓 `clippy.toml` 的 `max-fn-params-bools=0`、未含 bins、
未按 allow 表裁剪），故基线是唯一可复算真值：`scripts/clippy-baseline.json` 的
`platforms.linux.entries` 逐条可查、可复算上表。

Top 违例（决定波次，实测 lib+bins）：`as_conversions` 643、`cast_possible_truncation` 241、
`uninlined_format_args` 239、`map_unwrap_or` 236、`redundant_closure_for_method_calls` 194、
`missing_debug_implementations` 164、`needless_pass_by_value` 163、`allow_attributes` 131、
`assigning_clones` 123、`unreachable_pub` 121、`fn_params_excessive_bools` 114、`redundant_clone` 104、
**`unwrap_used` 100**、`cast_sign_loss` 94、`wildcard_imports` 83、`cast_possible_wrap` 71。

## 4. 命令

```bash
npm run rust:lint            # 棘轮门：存量不判红，新增/恶化 exit 1；平台分册缺失 exit 2
npm run rust:lint:selftest   # 门自检（34 例，纯 Node，秒级；CI 与 verify 都跑）
npm run rust:lint:update     # 重基线本平台分册（检出上涨默认拒写；确认后 -- --allow-growth）
npm run rust:lint:json       # 机器可读

node scripts/check-clippy-baseline.mjs --platform win32      # 指定平台分册（默认 process.platform）
node scripts/check-clippy-baseline.mjs --all-targets         # 含测试目标（默认 --lib --bins）
node scripts/check-clippy-baseline.mjs --print-command       # 只看门将执行的 cargo 命令
```

手动跑 clippy（不经门）时**必须**带上配置目录，否则 `clippy.toml` 的阈值不生效：

```bash
CLIPPY_CONF_DIR=.meta/lints cargo clippy -p pony-agent-core --lib --bins
```

重型本地构建遵守并发预算：`bash scripts/cargo-lane.sh clippy -- -p pony-agent-core`（见 AGENTS.md）。

## 5. 棘轮语义、平台分册与重基线流程

**记账**：键 = `(lint, 相对文件)`，不绑行号（行漂移不产生假失败）；`live ≤ base` 放行；
`live > base` 或键不存在 → exit 1；`base` 有而 `live` 变少 → 「可瘦身项」仅提示。

**入账口径**：表里 `= "allow"` 之外的一切 warning/error 一律入账——包括 `clippy::all` / `clippy::pedantic`
组的**组成员**与 clippy 默认组。理由：只认表内显式条目会让组拉起的违例静默漏账。
因此"豁免某条噪声"的唯一手法是把它写进 `lints.toml` 的 `= "allow"`——豁免必然出现在 diff 里。

**平台分册（v2）**：`cfg(windows)` / `cfg(unix)` 让各平台违例集合不同（本仓 28 个文件、约 24% 存量落在
平台门控文件里）。基线因此按 `platforms.<process.platform>` 分册：

- 当前平台分册缺失 → **exit 2**，提示在该平台跑一次 `--update-baseline --allow-growth`（首次建册；
  只写本平台分册，不动其他分册）；
- 禁止套用别平台口径（否则 Windows 一跑必假红，或 Linux 基线被 Windows 存量污染）；
- 当前只建了 `linux` 分册；Windows/macOS 由 B244 跟进（各自平台跑一次后，在该平台 CI 补同一步）。

**路径归一化**：`(lint, 文件)` 的路径先统一分隔符再按**大小写不敏感**前缀剥离仓根（Windows 盘符
`c:` vs `C:` 不会让存量整体变成"新增/消失"），剥不掉的前缀才保留为绝对路径。

**口径指纹（改了就必须重基线）**：`lints.toml` 内容 sha256（每分册内）+ `clippy.toml` 内容 sha256（全局）
+ 基线的 `package` / `targets`。任一不符 → **exit 2**。这条挡住了两类静默变松：把
`max-fn-params-bools` 从 0 改回 3（114 条基线凭空消失）、换包/换扫描面仍判绿。

**反 fail-open 护栏**（都是实测过的洞）：

| 现象 | 门的行为 |
|---|---|
| 0 条可计数诊断（`--message-format` 被吞、clippy 空转） | exit 2「扫描面失效」 |
| 实测 < 基线 50%（cfg 变体没编到、表被改小） | exit 2「疑似扫描面收缩」 |
| 表里有 clippy 不认识的 lint（E0602，无 span） | exit 2 并点名该 lint（拼错不会报错、只会永久空转） |
| 符号链接调用门脚本 | 入口守卫用 realpath（+win32 小写），照常执行 |
| 有 lint code 但**无源码位置**的诊断（宏/包级） | exit 2 并点名 lint（不得当"外部文件"静默丢弃） |
| `--from-json` 夹具流 + 重基线 | 默认拒（需显式 `--baseline`；指向受控基线一律拒） |
| cargo 编译失败 / 工具链缺失 | exit 2（原生码写进消息；不再与「新增债 1」同码） |

关于「cargo 增量缓存会不会漏报」：实测**不会**——cargo 对 fresh 单元会重放缓存诊断，连跑两次门的
`--json` 输出逐项相同（`liveTotal` 3,870 / `messageCount` 3,870 / `perLint` 完全一致）。若真出现缓存
失效，护栏 ①② 会以 exit 2 拒绝判绿，而不是静默放行。

**重基线流程**：

1. `npm run rust:lint:update`。写入只在**纯收缩**时自动放行；只要有**新增键/计数上涨**，或为某平台
   **首次建册**（等于一次性接收全部存量），门默认**拒写**并 exit 1——重基线不是"消红"工具，删掉分册
   也不该能洗白新增债。确认属有意变更（工具链升级、新 lint 纳入、新平台建册）后加 `-- --allow-growth`；
2. 读输出里的 diff（新增 / 上涨 / 下降 / 消失），**逐条解释上涨与新增**；解释不出来的就是门在变松；
3. 把 `scripts/clippy-baseline.json` 与代码改动一起提交。基线只许删条目或下调计数。

## 6. 波次（人力收口顺序，不改变门判定）

1. **波 1 零争议卫生**（单行锚定，多数 `--fix` 可覆盖）：`allow_attributes*` 131、`redundant_clone` 104、
   `wildcard_imports` 83、`undocumented_unsafe_blocks` + `multiple_unsafe_ops_per_block` 39、`dbg_macro`/`todo`。
2. **波 2 类型与转换**（需读调用点）：`as_conversions` 643 + `cast_*`（241 / 94 / 71）、
   `fn_params_excessive_bools` 114、`rc_buffer`/`clone_on_ref_ptr`。
3. **波 3 需要设计判断**：`unwrap_used` 100（lib 热点路径换 `?` / 带理由的 `expect`）、
   `needless_pass_by_value` 163（API 形状决定是否接受）。

## 7. 提升路径（渐进，不一次性迁移）

某条 lint 的基线**清零**后，把它从本目录的 `lints.toml` 迁入 `[workspace.lints]`，由既有
`RUSTFLAGS=-D warnings` 原生强制；一次迁一条，迁移本身是一次决策（改表 = 重基线 + ADR 增补）。
这样 `-D warnings` 的语义逐条变强，而不是一次性把门换成红海。

## 8. 诚实条款（本门不覆盖什么）

- **平台面**：当前只有 `linux` 分册（CI 挂在 ubuntu-latest 的 core-platform-check）。Windows/macOS 的
  `#[cfg]` 分支本门扫不到，由 `-D warnings` + 平台矩阵门承担；各平台补分册后可在该平台 CI 加同一步。
- **扫描包**：只扫 `-p pony-agent-core`（541 个 .rs 的主体）。`src-tauri/src`（10 个文件、5 处 `unsafe`，
  含 `platform.rs` 的 `ShellExecuteW` 安全边界）**不在本门扫描面**——扩面需另立第二份基线（B244 跟踪）。
- **测试目标**默认不在扫描面（`--all-targets` 可开）；测试里 `unwrap`/`expect`/打印由 `clippy.toml` 豁免，
  测试在 `-D warnings` 下的可编译性由 `scripts/check-rust-test-lints.mjs` 负责。
- **格式面**：本仓**未纳管 rustfmt**（无 `rustfmt.toml`、CI 无 fmt 门，存量差异 1.7 万处）。`cargo fmt --check`
  不是门禁判据，不得把存量格式差异当 finding，也不得顺手全量重排（那是夹带重构）。
- **语义面**：本门只判"机械可查"的部分；命名是否贴切、抽象是否过度、错误是否该吞——靠
  `.agents/skills/rust-review/` 的评审流程，不靠本门。
- **性能**：本目录不含任何性能判据。性能证据走 `target-perf` 槽位与
  `crates/pony-agent-core/examples/perf_probe.rs`，`size:check` 只管规模不管快慢。

## 9. 未采纳项（上游有、本仓不用）

| 上游资产 | 未采纳原因 |
|---|---|
| `[workspace.lints]` 直装 | §1：与 `-D warnings` 层级冲突 + 输出淹没 |
| `rust-port` 技能 | 本仓无 C/C++/Python/JS 待迁模块 |
| `references/bevy.md` + Bevy 规则例外 | 本仓无 `bevy` / `bevy_ecs` 依赖 |
| 远程 MCP（`mcp.beautifulrust.dev`） | 门禁判据不依赖公网；需要时可由上游 `mcp/` 自托管（独立决策） |
| `skills-lock.json` 登记 | 其 `computedHash` 契约仓内无校验器且不可复现；来源改由技能目录内 `PROVENANCE.md` 承载 |
