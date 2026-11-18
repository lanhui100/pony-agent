---
name: rust-best-practices
description: 何时用：写、改、重构、调试或解释本仓 Rust 代码时（新建 crate/模块/函数/结构体/trait、错误类型、async 与并发、unsafe 与平台 FFI、Cargo.toml、测试、rustdoc）；只要出现「Rust / cargo / crate / .rs / 借用检查 / 生命周期 / trait / clippy / rustfmt / 跨平台 unsafe」等词就用，用户没说"最佳实践/地道"也照样用。规则蒸馏自 Google Rust Style Guide、Rust API Guidelines、Effective Rust、Rust Design Patterns、Microsoft Pragmatic Rust Guidelines，已按本仓应用模式（bin + TUI）校准。
---

# rust-best-practices —— 本仓 Rust 写法契约（应用模式校准版）

本仓不是对外发布的库：产物是 `pony`（TUI）与 `ponycli` 两个 bin + 一个 Tauri 桌面端。规则本体来自
五个上游来源（全文见 `references/`），**落地口径按本仓应用模式裁剪**。上游之间冲突时本文写明跟哪个；
本文与 `.meta/lints/lints.toml` 冲突时以 lint 真源为准。

## 真相源

- `AGENTS.md` —— 常载命约、门禁速查（§2 架构不变量、§3 命令）
- `.meta/constitution/constitution.md` —— 常载命约五条（机械可查的承诺要有非零退出命令）
- `.meta/lints/README.md` —— lint 面实测数字与门语义（本文引用的 10,115 / 3,741 / 2,439 出自此文件，勿自行计数）
- `.meta/lints/lints.toml` —— **在册 lint 唯一清单**（引用 rule 名先读它；本文提到的 lint 名以它为准）
- `.meta/lints/clippy.toml` —— 阈值与测试豁免（当前只有 `max-fn-params-bools=0` 与 3 条测试豁免；**未**设 `msrv` / `disallowed-methods`，别假设它们存在）
- `.agents/notes/implemented/process/2026-11-18-rust-quality-gate-and-skills.md` —— 本次门禁与两技能的决策记录
- `docs/architecture/` —— 结构真源（扁平目录，按题查 `overview.md`、`runtime.md`、`system-design.md`、`structural-debt-proposals.md`）
- `references/{google-rust-style-guide,rust-api-guidelines,effective-rust,rust-design-patterns,microsoft-rust-guidelines}.md` —— 五来源 digest 原文
- `.agents/skills/rust-review/SKILL.md` —— 评审面（本技能写，它来查）

## 应用模式政策（本仓特有，**先读，判罚前必须过这一节**）

- **本体是应用，不是库**：`crates/pony-agent-core/src/lib.rs` 只作内部复用面（11 行，非对外 API），消费者是本仓两个 bin 与 `src-tauri`。API 设计规则按"应用"档执行：具体类型优先、不搞投机性泛化、错误可用 `anyhow`-式上下文而不必每 crate 一个错误枚举。
- **[lib] 类 lint 已关 6 条**（`missing_docs` / `missing_errors_doc` / `missing_panics_doc` / `exhaustive_enums` / `print_stdout` / `print_stderr`），理由：**日志与终端输出是产品行为**——TUI 画面就是 `stdout` 的字节序列，CLI stdout 就是契约；为"库的体面"消除它等于改产品。这些类别**不得作为 must fix 项**（评审侧同理，最多 consider 并写明"本仓应用模式关闭该 lint"）。
- 另有 6 条属**噪音/重复覆盖**，与 [lib] 类不是一回事，别混称：`doc_markdown`、`too_many_lines`（与 `npm run size:check` 的函数 200 行门重复）、`must_use_candidate`、`module_name_repetitions`、`module_inception`、`enum_glob_use`。
- **禁止建议加 `#![deny(warnings)]`**（也禁止把 `-D warnings` 写进源码）：工具链升级新增告警会直接把构建变红；严格性放 CI 的 clippy 调用参数上。仓内既有 `#![cfg_attr(test, allow(dead_code))]` 是 test-only 最小放行，不得扩大成 `#![allow(warnings)]`（契约 C3）。
- **禁止**"为了过 pedantic 就改公开 API / 改公共契约"。默认处置顺序：修 → 窄化抑制（带 reason）→ 若仍不合理，记进 `.meta/lints/lints.toml` 的取舍并在 ADR 写明。顺序不能倒。

## 步骤

1. **读 lint 真源**：`sed -n '1,80p' .meta/lints/lints.toml`——确认要引用的 lint 在册（不在册就引 clippy 上游名，别把不存在的规则说成本仓规则）。
2. **机械面（先跑再读代码）**：

```bash
npm run rust:lint         # clippy 棘轮门：新增/恶化即 exit 1（存量基线放行）
npm run rust:lint:update  # 重基线：检出新增/上涨会拒写；确认后加 -- --allow-growth，且必须人工 review diff
npm run rust:lint:selftest  # 门自检：证明"新增会红、存量不红"
```

3. **类型面（编译与测试）**：

```bash
npm run cargo:check:shared                                            # 全 workspace 类型检查（target-check 槽）
npm run cargo:test:shared -- --package=pony-agent-core --no-fail-fast # 核心 lib 测试
```

4. **重型本地构建**（release / 全量回归 / 多 crate 一起编）一律走并发预算入口，不裸跑 `cargo`：`bash scripts/cargo-lane.sh test -p pony-agent-core --lib`（4 job + 跨 target 全局 flock + nice 5；`--dry-run` 可先看最终命令不执行）。load 持续 >1.5×核数就停一条 lane；release 与全量回归只由 Lead 走 `--heavy` 独跑。
5. **规模面**：`npm run size:check`（文件 3000 行 / 函数 200 行 / 嵌套 8 层，棘轮基线 `scripts/file-size-baseline.json`）。**规模门只管规模，不管性能**——别拿它当性能证据。
6. **lint 基线现状**（引用数字前必读）：棘轮基线当前 **3,870 条 / 1,336 键**（`platforms.linux` 分册；扫描面 `-p pony-agent-core --lib --bins`，
   clippy 0.1.91 / rustc 1.91.1；**真值以 `scripts/clippy-baseline.json` 为准**）；首扫探针另有
   10,115（上游全表）/ 3,741（应用模式）/ 2,439（机器可修）见 `.meta/lints/README.md` §3。改 `lints.toml`
   却未重基线 → 门 **exit 2**（口径漂移 fail-closed），所以改表必须同提交重基线并逐条解释 diff。
7. **语义面（靠 review）**：命名是否有"唯一的家"、错误是否可行动、状态能否用类型表达、unsafe 不变量是否成立、并发是否新增 UB 窗口、性能改动是否有测量。这些机器判不了，逐条写进交付说明或交 `rust-review` 出报告。

## 规则本体（五来源蒸馏；只压缩措辞，不删规则）

### 1. 心法

- **写 Rust 形状的代码**：错误用 `Result`，资源用所有权 + `Drop`，多态用 trait/enum，可空用 `Option`。C++/Java/Python 的一对一照搬（`Deref` 假继承、`Manager`/`Factory`、`throw_if_null`）是气味。
- **让非法状态不可表示**：不变量压进类型——enum 而非 bool 开关、newtype 区分单位与 ID、构造器一次校验（`try_new`/`TryFrom`/`FromStr`）。编译器能强制的就别写成注释里的请求。
- **应用代码 ≠ 库代码**（本仓按应用档，见上节政策）。**让工具干机器活**：clippy 管惯用法、编译器管内存安全（rustfmt 在本仓**尚未纳管**，见 §2）；注意力留给设计。

### 2. 交付前的机械面

- rustfmt 输出直接接受，不做风格争论——但本仓当前**未纳管 rustfmt**（无 `rustfmt.toml`、CI 无 fmt 门、存量差异 1.7 万处），所以**不要**顺手 `cargo fmt` 全量重排（那是夹带重构），只在改动行内保持与邻行一致的风格。`#[allow]` 前先想"能不能修"。抑制用**最窄作用域 + 理由**（`#[expect(clippy::x, reason = "…")]` 或 `#[allow(...)] // 理由 + 退出条件`）；`allow_attributes`/`allow_attributes_without_reason` 已在册。**门记账口径**：`lints.toml` 里 `= "allow"` 之外一律入账（含 `all`/`pedantic` 组成员与 clippy 默认组），**豁免唯一合法通道是把 allow 写进 `lints.toml`（一次可见 diff）**——源文件里的 `#[allow]` 只是就地压噪，不改变门的记账，用它"让新告警消失"等于藏债。全仓 `#[expect(` 目前 0 条，迁移按 `.meta/lints/README.md` §6 的**波 1** 排期推进（本仓未声明 `rust-version`，`#[expect]` 需 rustc ≥1.81），**靠 review 逐条做，不搞一次性大迁移**。
- 交付只跑「步骤」2–6 的机械命令（`rust:lint` / `cargo:check:shared` / 聚焦测试 / `size:check`），不全量重跑（证据面匹配，见 `.agents/skills/README.md` 五条纪律 3）。

### 3. 所有权与借用

- **唯一所有权优先**：`Rc`/`Arc` 只当共享所有权是设计的一部分，`RefCell`/`Mutex` 只当需要内部可变性——它们把检查从编译期挪到运行期，要刻意用。
- **不许克隆来哄借用检查器**（只为结束借用的克隆是静默副本，后续修改会丢），按序试：① 临时值绑定成 `let` 或用块缩短借用；② 拆表达式成带类型中间量让出错那步可见；③ `std::mem::take`/`replace`/`Option::take` 从 `&mut` 移出（需 `Default`）；④ 拆大结构体让小字段分别借用（同一函数内编译器能分开借，跨方法不行）；⑤ 刻意克隆小且不可变的数据，不显然就写注释；⑥ 最后才用"索引进 owning `Vec`"或 `Rc<RefCell<_>>` + `Weak`，别造自引用结构。
- **入参取最泛的借用类型**：`&str` 而非 `&String`、`&[T]` 而非 `&Vec<T>`、`&T` 而非 `&Box<T>`、`Option<&T>` 而非 `&Option<T>`；只读路径/字符串/字节缓冲用 `impl AsRef<Path|str|[u8]>`（字段上禁止，字段存 `PathBuf`/`String`）；只迭代的库函数收 `impl IntoIterator<Item = T>`，应用代码直接 `&[T]` 也行。
- **要所有权才取值、取就按值取**；只读 `&T`，就地改 `&mut T`；多返回值用元组/结构体，不用 `&mut` 出参（复用调用方缓冲是例外）。**结构体优先存 owned 数据**（`&'a T` 给每个容器强加生命周期），有生命周期的类型起描述名（`'src`、`'conn`）。**设完冻结**：`let mut` 建完再 `let data = data;` 遮蔽。
- **`Rc`/`Arc`/`Box`/`RefCell` 不进公开签名**：传 `&T`/`&mut T`/`T`；要共享的服务内部 `Arc<Inner>` + 廉价 `Clone`。

### 4. 类型与 API 设计

**命名**（Google 风格）：

| 项 | 规范 |
|---|---|
| 类型/trait/enum 变体 | `UpperCamelCase`，缩写算一个词：`HttpClient` |
| 函数/方法/模块/局部变量 | `snake_case` |
| 常量/static | `SCREAMING_SNAKE_CASE` |
| 类型参数 / 生命周期 | 短：`T`、`R: Read`；类型上的生命周期给描述名 |

- 取数不加 `get_`（`fn len()`、`fn name_mut()`），配 `set_foo`/`take_foo`/`with_foo(|f| ..)`/`_unchecked` 兄弟；转换前缀表代价与所有权：`as_`（免费，借用→借用）、`to_`（昂贵，借用→owned）、`into_`（消费 self），且 `as_mut_slice` 而非 `as_slice_mut`。
- 构造器 `new`/`with_capacity`/`new_with_foo`/`try_new`/`from_foo`（重要的配 `From`）；I/O 资源用领域动词（`File::open`）；`iter`/`iter_mut`/`into_iter` 的返回类型叫 `Iter`/`IterMut`/`IntoIter`；错误类型读作"动词-宾语-Error"（`ParseBoolError`）；避免 `Service`/`Manager`/`Factory`/`Helper`/`Util` 模糊后缀；模块名不重复进类型名（`foo::Id` 而非 `foo::FooId`）；实现 `len()` 就实现 `is_empty()`；feature 名裸写（`std`、`serde`）。

**用类型建模领域**：

- newtype 区分语义（`Miles` vs `Kilometers`）；布局要紧时 `#[repr(transparent)]`；带不变量的 newtype 只在可失败构造器里校验，**不提供从原始类型的无失败 `From`**。
- **公开/共享签名不用 `bool` 参数**：`report(true)` 调用点不可读、也长不出第三种模式 → 两变体 enum（`Verbosity::{Summary, Full}`）或两个方法（`report()`/`report_verbose()`）。（在册 `fn_params_excessive_bools`，且在 `.meta/lints/clippy.toml` 里把阈值定到 **`max-fn-params-bools = 0`——出现一个 bool 参数就触发**；"私有 helper 可用"是本规则的靠-review 例外，不是 lint 的放行。**重构既有 bool 参数是目标，不是保留项**。）
- 具名字段优于元组（单字段 newtype 与极局部代码例外）；缺失用 `Option` 而非哨兵（`-1`、`""`）；"X 只在 Y 时有意义"用数据携带 enum；用 std 最强类型（`PathBuf`/`&Path`、`Duration`；`NonZeroU32` 只在内部，边界收普通整数）；独立布尔开关用 `bitflags`，enum 表达"恰好一种选择"。

**及早实现标准 trait**（下游受孤儿规则限制只能你实现）：能 derive 就 derive `Debug`（公开类型必须有，含密文手写脱敏）、`Clone`（唯一资源与密钥除外）、`Copy`（仅小且逐位可拷）、`PartialEq`/`Eq`、`PartialOrd`/`Ord`（有全序就别只给 `PartialOrd`）、`Hash`（与 `Eq` 一致）、`Default`；要被读的（错误、字符串包装）实现 `Display`；纯函数与"丢弃即 bug"的类型（错误、惰性值）加 `#[must_use]`。
- 实现 `From`/`TryFrom`，**不直接实现 `Into`/`TryInto`**（blanket impl 已给）；优先 `from`/`into` 而非 `as`（后者静默允许有损）。实现 `FromStr` 而非自造 `from_str`；集合实现 `FromIterator`/`Extend`/`IntoIterator`（owned 与 borrowed）；`Deref` 只给智能指针与 owning 集合，不做假继承；运算符只按数学含义重载。不在结构体定义上写可推导的 trait bound（`#[derive(Clone)] struct Good<T>`），bound 写在需要的 `impl` 上。
- **手写 `Ord` 就必须手写 `PartialEq`/`PartialOrd`/`Hash` 与之同意**，文档说明哪些字段不参与比较；派生 `PartialEq` + 自定义 `Ord` 会破坏 std 契约，让 `BTreeMap`/`sort`/`dedup` 静默错乱。

**构造器/建造者/默认值**：有 `Default` 也给 `new()` 且两者一致（互相委托）；可选参数 ≤2 用 `new`/`with_a`/`with_b`，再多或构造要校验/有副作用就上 builder（`Foo::builder()` → setters 按字段名、不消费、`build() -> Result<Foo, E>` 做全部校验与跨字段检查）；≥4 个散参数先归组成语义类型。

**可见性与演进**：默认私有，字段非 C 式被动数据不公开；先 `pub(crate)`/`pub(super)` 再 `pub`（变公开不破坏，反向破坏）；跨 crate 的公开 enum 与公开字段结构体加 `#[non_exhaustive]`（除非是稳定本体），match 时列全已知变体再加通配臂；不想被下游实现的 trait 用 seal（`pub trait TheTrait: private::Sealed`）。只在必要时返回 `Vec`（库内部 API 优先 `impl Iterator<Item = T>`，对外稳定面用 newtype）；每个公开项只从一个路径暴露；不做伞 crate 就别 `pub use` 别人条目；第三方类型不进公开 API，必须出现时重导出统一版本；`serde` 实现按 `serde` feature 门控；`const` 只在会一直是 `const` 时标；不暴露 `ErrorKind` 这类把私有失败模式锁死的东西。

**trait 与泛型**：trait 只为泛型编程（`T: Trait`/`impl Trait`/`dyn Trait`）或扩展 trait（`FooExt`）存在，不用来组织代码；核心能力放 inherent 方法、trait impl 转发过去（调用方不必 import）。按序选：具体类型 → 泛型 → `dyn Trait`（用自家结构体包一层 `struct DynStore(Arc<dyn Store>)`）；service 级类型不嵌套泛型（`Service<Backend<Store>>` 不行）；早定 trait 是否 object-safe（泛型方法/接收者外的 `Self` 会毁掉它，用 `where Self: Sized` 排除）。必备方法最小化、其余给默认实现（如 `Iterator` 由 `next` 推其余）；有明确接收者就做成方法，无接收者的计算是自由函数，关联函数只用于构造；闭包用最松可行 bound（`FnOnce`/`FnMut`/`Fn`）而非 `fn` 指针，闭包参数放最后且最多一个；参数顺序仓内一致（调用相关在前，logger/allocator 等全局上下文在后）；函数 + 泛型 + trait 都做不到才用宏（`macro_rules!` 优先，宏输入长得像输出，宏不改类型种类/函数签名）。

**重构别人的代码**：请求点了名的气味全修（只为过借用检查的 clone、`&String`/`&Vec`、可失败路径的 `unwrap`、签名里的 `Rc<RefCell<_>>`、`get_` 取数、bool 参数、`+` 拼字符串、`#![deny(warnings)]`、无 `Error` 的错误类型、输出泄漏 `HashMap` 迭代序）；除非改它就是目的，公共行为逐字节保持（panic 改 `Err` 是调用方想要的；删错误变体、去 derive、改构造器入参不是——必须改签名就前后对照写明理由）；看不懂的怪癖保留并标记，别静默"修好"；模块没测试就先钉住当前行为。

### 5. 错误与 panic

| 情形 | 用什么 |
|---|---|
| 缺失是常态（`first()`、`checked_add`） | `Option<T>` |
| 调用方需要知道错在哪 | `Result<T, E>` |
| 自家代码的 bug / 违反不变量 | `panic!` / `assert!` / `expect("为什么不可能")` |

- **panic 不是异常**：不用它报坏输入，不用 `catch_unwind` 做控制流（`panic = "abort"` 下无效且可能留脏状态），不在库内捕获 panic 后继续。必须提供会 panic 的便捷 API 时配可失败版本（`v[i]`/`v.get(i)`、`new`/`try_new`）并在 `# Panics` 写明。
- **每个故意 panic 的文案带相关数值**：`assert!(buf.len() >= HEADER, "buffer too small: {} bytes", buf.len())`；优先 `expect("原因")` 而非 `unwrap()`；消息别复述条件本身。
- **匹配代替 check-then-unwrap**：`if let Some(v) = opt`、`let Some(v) = opt else { return .. };`，绝不 `if opt.is_some() { opt.unwrap() }`（在册 `unwrap_used`；`expect_used` 是"带理由的形式"，**不是违规项**）。
- **锁中毒是致命的**：`.lock().expect("lock poisoned")`，不用 `into_inner` 恢复。
- **库错误类型**（`lib.rs` 属内部面，按此宽松执行；对外可复用模块照办）：实现 `std::error::Error`（要 `Debug` + `Display`）、`Send + Sync + 'static`、`source()` 串因果；公开 `Result` 的 `E` 不用 `()`/`String`/`Box<dyn Error>`；`Display` 小写简洁无尾标点；实现 `From<Upstream>` 让 `?` 可转，`map_err` 只用于外部错误或加语境。两种正当形状：每因一变体的 enum（`thiserror` 生成）或"私有 `kind` + `is_xxx()`"结构体；不做一个 crate 级大 enum 列尽失败模式，不暴露调用方无法处置的细节；消费入参的可失败函数把参回传给错误（`FromUtf8Error::into_bytes`）；公开 API 不用 `anyhow::Error`。
- **应用错误处理**：`anyhow`/`eyre` + `.context("opening config")`，一个 app 级错误类型而非多个；`main` 可返回 `Result<(), anyhow::Error>`。
- **析构不失败、不 panic、不阻塞**：需要失败收尾另给 `close(self) -> Result<()>`，`Drop` 只做尽力清理。
- 失败模式写文档：有 `Result` 写 `# Errors`，可达 panic 写 `# Panics`，每个 `unsafe fn` 写 `# Safety`（[lib] 类 lint 已关，不等于这段文字失去价值——**靠 review 要求**）。

### 6. 日常惯用法

- **短管道用迭代器**：`.iter().filter(..).map(..).collect()`、`.any()`、`.sum()`、`.fold()`；可失败步骤收 `Result<Vec<_>, E>` 再用 `?`。函数体大、干几件事、或需要绕回早退时保留 `for`；长链绑定成命名变量而不是塞进 `for` 头；`for x in it` 优于 `.for_each(|x| ..)`。
- `Option`/`Result` 组合子（`map`/`and_then`/`ok_or_else`/`unwrap_or_else`/`as_ref`/`as_deref`）比 `match` 清楚时用组合子，反之用 `match`。
- `format!`/`write!` 用内联参数（`format!("{name}: {count}")`），不链式 `+`、不 `str::replace` 拼接；热路径写进已有缓冲。推断遮住意图处写显式类型：`.into()`、`.parse()`、`.collect()` 与复杂迭代器结果——`let x: Vec<u32> = ..` 优于 turbofish。
- 清理守卫（早退/展开都要跑）绑定命名变量 `let _guard = ..`（`let _ = ..` 立即 drop）。混合捕获的闭包先块内重绑再 `move`：`let job = { let cfg = cfg.clone(); move || run(cfg, data) };`。
- **import 风格**：一模块路径一行（`use foo::bar::{Bar, Baz};`）；类型/trait 无限定名引入，自由函数带模块限定（`mem::take`、`fs::read`）；trait 只在需要的函数内引入；**禁 glob**（例外：测试 `use super::*`、函数内 `use Enum::*`；框架 prelude 本仓不适用）；在册 `wildcard_imports`。魔数变命名 `const`，注释写清来源与改动影响。
- **日志**：结构化事件走 `tracing`（本仓已用），不用 `println!`/`dbg!`——**但 TUI 画面与 CLI stdout 本身就是产品输出，属应用模式的合法 stdout，不是待清的 println**（`print_stdout`/`print_stderr` 已关）。`dbg_macro`/`todo` 在册，交付前清干净。

### 7. 并发与 async

- 公开类型与其返回的 future 应 `Send`（通常也 `Sync`），用编译期断言钉住；跨 `.await` 持有 `Rc` 或 `RefCell` 借用会让 future 变 `!Send`：

```rust
const _: () = { const fn assert_send<T: Send>() {} assert_send::<MyClient>(); };
```

- 优先消息传递（channel）而非共享状态；必须共享时：有联动不变量的数据放同一把锁、锁作用域压到几行、**持锁不调用户闭包**、公开方法不返回 `MutexGuard`。
- 写 `async fn` 而不是 `fn -> impl Future`（trait 里或热路径需要把 setup 放 async 块外时例外）；async 里的长 CPU 活必须让出（每几十微秒 `tokio::task::yield_now().await`）或挪 `spawn_blocking`；库代码不自起裸线程、不假定运行时（由调用方给 executor/spawn，或收 `impl AsyncRead`/`impl Read`）。
- **本仓附注**：TUI 主循环与 turn 线程的归属见 `docs/architecture/runtime-ownership-split.md` 与 `per-session-async-turn-task-model.md`；跨线程共享优先走既有 event / 状态发射器，不新造全局。

### 8. unsafe 与平台 FFI

- `unsafe` 只为三种理由：安全代码表达不了的新抽象、有测量支撑的性能需求、FFI 与平台调用；先用 std 与经过审计的 crate。
- **每个 `unsafe` 块 / impl 紧邻一条 `// SAFETY:` 注释说明不变量为何成立**；每个 `unsafe fn` 的 `# Safety` 列出调用方责任；开 `unsafe_op_in_unsafe_fn`（在册；另在册 `undocumented_unsafe_blocks`、`multiple_unsafe_ops_per_block`）。
- 把 `unsafe` 关在**能维持该不变量的最小模块**里并用安全 API 包住：模块边界即 soundness 边界。一个安全函数经任意安全调用序列能触发 UB 就叫 unsound，unsound 永不可接受。
- 只有当误用会导致 UB 才标 `unsafe`（`delete_database()` 是安全函数，只是危险）；不用 `transmute` 转 enum 或绕生命周期，不用 `unsafe impl Send` 安抚编译器；`unsafe` 代码在 Miri 与对抗输入下测。
- 平台分支标注 `[VERIFY DEGRADED: <平台>]`（本仓惯例：Linux 本机无 Windows target，windows-only 分支不参与类型检查，见 `agent/process.rs` 既有写法）。
- FFI：共享结构体 `#[repr(C)]`、定宽整数、`CString`/`CStr`、谁分配谁释放、C 拥有的指针包进 RAII 类型由 `Drop` 释放、判空返回 `Result`、边界处 `catch_unwind` 截断 panic；绑定用 `bindgen` 放 `foo-sys`，业务逻辑留安全核心 crate。

### 9. crate / module / 依赖

- 扁平 `crates/` workspace 内小而单一用途的 crate；绝不在一个 crate 的 `src/` 里嵌 crate；crate 长出第二用途或出现子目录就拆；版本与 lint 经 `[workspace.dependencies]`/`[workspace.lints]` 共享。
- 模块：叶模块以其主条目命名（`Foo` 在 `foo.rs`）；非叶模块只放 `mod` 声明与重导出；核心条目在 crate 根，其余按用例分组（`account`、`network`）而不是按种类（`traits`、`errors`）；不自造 prelude；逐个重导出（`pub use foo::{A, B};`），不 glob。
- 依赖：每一个都付编译时间、二进制体积、供应链暴露（含 `build.rs` 与 proc macro）；用范围（`"1"`、`"1.4"`）不用 `"*"` 或无谓 `"="`；bin 提交 `Cargo.lock`；用 `cargo tree --duplicates`、`cargo deny`、`cargo udeps`、`cargo audit` 审计。
- feature 可加性、裸名（`std`/`serde`/`alloc`/`test-util`），不写会删/改公开条目的 feature；不用 feature 门控字段或 trait 方法；mock/fake 放 `test-util`。
- semver：删/改公开项、给非 `#[non_exhaustive]` enum 加变体、给公开可构造结构体加字段、加 trait bound、让 trait 失去 object-safe 都是破坏性变更；先在 minor deprecate 再在 major 删；新 crate 用最新稳定 edition，**MSRV 要刻意定**（本仓现状：`crates/pony-agent-core/Cargo.toml` 只写 `edition = "2021"`，无 `rust-version`——引用新语法/新 lint 前先确认这条缺口，别假设 MSRV）。不在 `main()` 前跑代码（不用 `ctor` 类 crate）；不用 stable 工具链上的 unstable 特性。

### 10. 文档与注释

- 公开项 `///`、公开模块与 crate 根 `//!`；首句一行约十五词（列表里就是摘要）；节按序 `# Examples`、`# Errors`、`# Panics`、`# Safety`；示例用 `?`（`fn main` 藏进 `# ` 行）**不用 `unwrap()`**，因为用户会照抄；参数用散文而非表格。
- 文档写签名说不出来的东西：前置条件、副作用、所有权转移、复杂度、为什么；不复述类型或名字（`remove_element` 上写"移除一个元素"是噪音）；用 `` [`Type`] `` 链接条目；用户永不触发的 impl（`From<PrivateError> for PublicError`）加 `#[doc(hidden)]`。
- `//` 注释解释 why 不是 what：`// SAFETY:`、`// TODO(issue-123):`、不显然的魔数、带链接的绕法；不写设计日记、不写"为什么选 X"的散文、不写规则对照表；文档写终态。

### 11. 测试

- 私有内部用 `#[cfg(test)] mod tests { use super::*; }`；能经公开 API 触达的写 `tests/`，两者都行时优先 `tests/`；doc example 也是测试。
- 测可观察行为与性质（单调、往返、幂等），不复述常量、不镜像分支；修 bug 先写失败测试；覆盖每个 feature 组合与每个平台分支。
- 做 I/O、时间、随机、网络的必须可 mock（`new_mocked() -> (Self, MockCtrl)` 挂 `test-util`），不用"会做 I/O 的 `Default`"；解析不可信输入用 `cargo fuzz`；`unsafe` 跑 Miri；性能基准用 `criterion`/`divan` + `std::hint::black_box`（**本仓两项都没有，见 §12**）。
- 断言带上下文，失败可读：`assert_eq!(got, want, "input {input:?}")`。

### 12. 性能（诚实版：本仓没有 bench harness）

- 先可用后优化，优化前先测量。**本仓当前没有 criterion/divan 基准套件，也没有 `benches/`**——任何"性能提升 X%"必须有下面这条证据链，否则按猜测处理。
- 唯一既有性能证据：`crates/pony-agent-core/examples/perf_probe.rs`（分相位计时 + VmHWM；只读相位，需把 `PONY_AGENT_DATA_DIR` 指向库副本），槽位用 `target-perf` 目录（`scripts/clean-targets.sh`、`scripts/artifacts-status.sh` 把它列为可再生产物）：`bash scripts/cargo-lane.sh run -p pony-agent-core --example perf_probe --target-dir target-perf -- <args>`（参数含义见 `perf_probe.rs` 头注释，勿指向真实库）。
- `npm run size:check` 管的是**规模**（行数/嵌套），与性能无关；别把"拆小了"当"更快了"。
- 优化顺序：`Vec::with_capacity`/`collect()` 替代 push 循环；热 API 复用调用方缓冲（`fn read_into(&mut self, buf: &mut Vec<u8>)`）；长命不可变序列用 `Box<[T]>`/`Box<str>`/`Arc<str>`；无预留构好的长命集合 `shrink_to_fit`；可信键才换更快的 hasher；批量处理替代逐项加锁；应用可设 `#[global_allocator]`（本仓已用 mimalloc，见 `src/lib.rs`）；小 plain 类型 derive `Copy`，大的永不。

### 13. 交付前检查清单

- `npm run rust:lint` 绿（棘轮：新增/恶化 exit 1）；`npm run size:check` 不新增超限。
- 测试内与 `main` 外没有 `unwrap()`；每个 `expect`/`assert`/`panic` 的文案带相关数值。
- 公开函数收 `&str`/`&[T]`/`impl AsRef<_>`/`impl IntoIterator`，不收 `&String`/`&Vec<T>`；公开签名无 `bool` 参数；手写 `Ord` 必配一致的 `PartialEq`/`Hash`。
- 公开类型 derive `Debug` 与适用 trait；错误类型 `Error + Display + Send + Sync + 'static`。
- 没有只为过借用检查的 `.clone()`；没有 glob（除测试 `use super::*`）、没有自造 prelude、没泄漏第三方类型。
- 每个 `unsafe` 块有紧邻 `// SAFETY:`；每个 `unsafe fn` 有 `# Safety`。
- 公开项文档带一行摘要，相关处补 `# Errors`/`# Panics`（应用模式不强制，但**新写的公开面按此办**）。
- 应用代码保持简单，库面（`lib.rs` 及其模块）守 API 规则。

## 校准样例

每组"正例/反例"的差异维度就是判定依据；行号可用 `grep -n` 复核，别用"看起来乱"当判据。

- **正例（无 panic 点）**：`crates/pony-agent-core/src/agent/model_window.rs:289-292` 把"可选数值"一路收成 `Option`（`.and_then(Value::as_u64).and_then(|tokens| u32::try_from(tokens).ok())`），失败即 `None`，无 `unwrap`。
- **反例（check-then-unwrap）**：`crates/pony-agent-core/src/agent/recovery.rs:128-130` 在 `ctx.fallback_model.is_some()` 合取判定后才用 `.unwrap()`（第 130 行 `fallback_model: ctx.fallback_model.clone().unwrap()`）——panic 虽被同条件挡住，但它是可避免的断言重复，命中在册 `unwrap_used`；修法 `if let Some(fallback) = ctx.fallback_model.as_ref()`。**差异维度 = 取用可选值是否出现 `unwrap()/expect()` 调用点**。

- **正例（抑制带理由与退出口）**：`crates/pony-agent-core/src/agent/stop_escalation.rs:107` `#[allow(dead_code)] // wave60-T0：字段面冻结，读取方在 wave60-T1/T3 实现时接入。`——说明冻结范围 + 谁接 + 接到哪一波。
- **反例（抑制无理由）**：`crates/pony-agent-core/src/acp/transport.rs:737` 的 `#[allow(clippy::too_many_arguments)]` 挂在 7 参的 `read_loop` 上，属性与紧邻文档都没写"为什么这 7 个参数不可归组"、"何时拆"；命中在册 `allow_attributes_without_reason`。**差异维度 = 抑制行是否携带"理由 + 退出条件"**（同文件 235 行 `#[allow(dead_code)]` 有散文理由，属可接受形态）。

- **正例（unsafe 有 SAFETY 且写不变量）**：`crates/pony-agent-core/src/agent/process.rs:145-146` `// SAFETY: \`process\` is the live process handle of the suspended child owned by the caller.` + `unsafe { NtResumeProcess(..) }`。
- **反例（unsafe 无 SAFETY）**：`crates/pony-agent-core/src/bin/tui/terminal.rs:409-411` 的 `unsafe { libc::raise(libc::SIGSTOP); }` 只有函数级 `///` 说明（讲为什么挂起），块上无 `// SAFETY:`；命在册 `undocumented_unsafe_blocks`。**差异维度 = unsafe 块是否紧邻一条陈述不变量的 `// SAFETY:`**。

- **正例（模式用枚举承载）**：`crates/pony-agent-core/src/bin/tui.rs:5068` `fn runtime_mode_lifecycle(mode: &RuntimeMode) -> TurnLifecycle`——0 个 bool 参数，8 种模式由 `RuntimeMode` 变体（`tui.rs:510-519`）表达，match 穷尽到编译期。
- **反例（bool 参数成灾）**：`crates/pony-agent-core/src/bin/tui/queue.rs:276` `pub fn should_drain_queued(is_streaming: bool, ui_idle: bool, queued: usize, turn_in_flight: bool, dispatch_pending: bool) -> bool`——5 个入参里 4 个 bool，调用点写作 `should_drain_queued(false, true, 3, false, false)`（`crates/pony-agent-core/tests/tui_streaming_input_acceptance.rs:477`）不可读；命中在册 `fn_params_excessive_bools`。**差异维度 = 是否出现 bool 参数**（本仓阈值 0，见 `.meta/lints/clippy.toml` 的 `max-fn-params-bools`），以及"模式"是否被类型化。

- **正例（可截断转换饱和处理）**：`crates/pony-agent-core/src/bin/tui.rs:2094` `let width_u16 = u16::try_from(width).unwrap_or(u16::MAX);`——不静默回绕、不 panic。
- **反例（`as` 静默回绕）**：`crates/pony-agent-core/src/bin/tui/task_center.rs:316` `let content = content_lines(state).len() as u16;`（usize→u16，超 65535 直接回绕）；同向第二例 `crates/pony-agent-core/src/agent/process.rs:657` `bytes.len() as i32`——Win32 `cchWideChar` 的负数有"`-1` = 以 NUL 结尾"的特殊语义，回绕会把长度语义反转。命中在册 `as_conversions`/`cast_possible_truncation`/`cast_sign_loss`。**差异维度 = 可截断/可变号转换是否走 `try_from`（或显式饱和）而不是裸 `as`**。

- **正例（规模无债）**：拆出的 TUI 叶模块 `crates/pony-agent-core/src/bin/tui/*.rs`（28 个文件，最大 `status_dwell.rs` 597 行）在 `scripts/file-size-baseline.json` 里**没有任何条目**——不靠豁免、不靠基线，每文件 <3000 行、每函数 <200 行的门自然成立。
- **反例（把规模债藏进豁免围栏）**：`crates/pony-agent-core/src/bin/tui.rs:4` `// ponylint:file-size-exempt [until 2026-12-31] tui.rs 结构庞大正在实施模块化重构拆分`——文件 13201 行（`wc -l`），因围栏整体跳过规模门，基线里也无其条目（脚本头注释的 15327 是历史值）。**诚实条款**：围栏本身是既有语义（带 `until` 且过期必须判违规），"反例"指**把规模债长期寄存在豁免里**这种用法，不是该文件本身违规；到期（2026-12-31）不拆或续期无理由，即从"加工中"变成"债务"。**差异维度 = 规模债是"已拆解且门自然绿"，还是"靠豁免围栏跳过门"**。

## 不适用面（别复制不存在的场景）

- **无 Bevy 依赖**：本仓 Cargo 清单里 `bevy` 零命中，不用 ECS。故上游 `references/bevy.md` **未安装**，ECS 相关规则（B0001 查询冲突、system 参数按值传、prelude、passive component）在本仓一律不适用——不要拿它们当 finding。
- **当前无 C/C++/Python 迁移面**：上游第三个技能 `rust-port` 未适配（本仓没有来源语言代码树，无迁移 diff 可审）。
- **不是对外库**：不需要 semver 发布流程、不需要为"下游"保留 API——但**内部复用面**（`lib.rs` 及其模块）仍守 §4，因为两个 bin 与 `src-tauri` 都在消费它。

## 验证与报告

- 跑 `npm run rust:lint`（必须绿；exit 1 时先看是"新增"还是"恶化"，再看是否该 `rust:lint:update`），改了基线就跑 `npm run rust:lint:selftest` 证明门仍有牙。
- 跑 `npm run size:check`；规模变化必须在交付说明里区分"拆解（无新增超限）"与"新增债（更新基线要理由）"。
- 交付报告写四行：**改了哪些文件** / **机械面命令与退出码原文** / **语义判断（靠 review）的逐条理由** / **未做的事**（未跑的命令、未验证的平台分支）。只认命令输出，不认"应该没问题"。
- 拿不准是否越界应用模式政策时，停下来先问，不要把"为过 lint 改产品行为"当交付。
