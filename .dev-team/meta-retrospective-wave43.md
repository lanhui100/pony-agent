# Wave-43 元复盘：Agent Trace 迁移 OTel → Langfuse（协作元框架诊断）

> 复盘对象是**本次 Agent Team 的协作元框架**，不是业务代码细节。
> 账本：`.dev-team/state.json`｜契约：`.dev-team/contract-matrix-wave43.md`、`.dev-team/wave43-adjudication.md`｜
> 收据：`.dev-team/evidence/wave43/`、`.dev-team/regression-baseline-wave43.json`、`.dev-team/wave43-dep-probe.md`｜
> 评审：`review-wave43-{a,b,l3p}.json`、`review-wave43-recheck.json`｜ADR 0047。

## 1. 通信拓扑与信噪比

**有效实践**：把「侦察」全部外派给 Subagent（Explorer / L2-AT / 三路 L3），Lead 上下文只留裁决与收据，
没有出现一次「Lead 亲自 grep 大文件」的浪费；Explorer 产出的 `path:line` 事实清单让后续所有协议冻结与
测试编写都不必重新摸索（依赖预演 `.dev-team/wave43-dep-probe.md` 一次通过、零版本回退，就是这条链路的红利）。
JSON 审查协议（`dev-review-lint.py` 闭合校验）把三份评审压到可机械核对的粒度，BLOCKING 全部带可复现命令，
没有一条是「我感觉」。

**主要问题（信噪比最大的黑洞）**：**同写域并发写入**。`executor-core` 在被中断后又写了一次
`otel/mod.rs`（把 `OtelState.runtime` 改名、`export()` 改 `pin!`+`poll_fn`），与接棒的
`executor-core-2` 落盘冲突并导致对方 `E0560` 编译失败，直接损失一轮编译与一次沟通。
根因不是纪律问题，而是**编排信息流问题**：中断（interrupt）不等于「收回写权」，而新 owner 在
认领时看不到旧 owner 仍在途的写入意图。次生问题：`test-agent` 因上下文耗尽在红相阶段掉线，
接棒者 `test-agent-b` 接手时发现工作区里已有「不属于自己写入」的 OT-7/8d/8e 代码，无法判断归属
——这属于**署名与因果链断裂**，会让复盘无法归因。

**量化**：全流程 Subagent 派生 8 次（侦察 3、L2-AT 1、L3 3、复审 1），Teammate 6 名；
Lead 侧重复读取同一文件的情况 0 次；但「同一文件被两个 agent 写」发生 1 次（代价 ≈ 1 轮返工）。

## 2. 门禁穿透与误杀率

**门禁真实拦截了东西**，这是本波最值得肯定的部分：

- **L3-B 抓到 3 条 BLOCKING**：令牌经 `Debug` 明文（实测打出 `SUPER_SECRET_TOKEN_123`）、
  trace 新路径脱敏命中 0（**相对被删的旧路径是安全退化**）、导出器层无 gate（实测「关闭后仍在飞上报」）。
- **L3-A 抓到 1 条最刁的 BLOCKING**：`BypassGuard` 是**进程级**的，于是「关闭态下手动上传」会把
  同期其它 turn 的 `llm.chat` 一起送出网（实测 27~30 条）。这条是「隐私开关默认关闭」这一产品承诺的
  直接破坏，靠读代码几乎不可能发现，只有 wire 级实测才暴露。
- **L3-P 抓到 1 条 NFR BLOCKING**：新路径用明文 `eprintln!` 绕过 JSON 日志面，等于悄悄扩大 PA-119 豁免面。

**误杀/过判**：L3-P 提的「`flush_state` 每调用新建线程不 join」首轮措辞偏保守，实为可接受；但同一批
里「`ACTIVE_TURNS` 全局栈违反 per-turn closure」是真问题——**误杀率低、漏判率更低**，两路审查视角互补
（并发视角 vs 安全视角）确实抓到了彼此看不到的东西，验证了「互补视角矩阵」不是形式主义。

**门禁自身的缺口**：`dev-stub-lint.py` 对 Rust **物理不可用**（只解析 `.py/.ts/.js`，遇到 `.rs` 输出
WARN 并 exit 0 = 空门禁）。protocol-agent 自己补了一份 Rust 等价校验并落 `.dev-team/tools/wave43-rust-stub-purity.py`，
已由 Lead 收编为常备门禁。这是本次对元框架最实质的一次加固。

## 3. 分工契约与隔离有效性

**测试分权真的挡住了作弊**：Executor 全程零改动 `tests/**`（git diff 可证），而 Test Agent 独立编写并
红相锚定两个测试文件（commit `1dea72e`），中途还主动发现「扩扫守卫按整文件排除」的自指漏洞并改成
**按常量块掩码 + 三条完整性断言**（等长、13 符号全在掩码内、守卫文件必须在扫描面）——这是防御性测试
写法的正例。

**契约冻结的收益**：Protocol Agent 先把 `otel/mod.rs` 空桩（341 行、签名+属性常量+默认语义）与 NFR 基准
落盘，之后 Test Agent 才能写「不依赖实现细节」的属性映射断言（`AttrValue` 中性类型避免了 wire 字节扫描）。
Lead 裁决 A1~A8（尤其 A1 改路径、A2 补纯映射 seam、A6 驳回「需要 ADR」的错误判断）在**编码开始前**
消掉了三处必然返工。

**写域声明是 advisory 而非锁，这次真的被穿透了**：task-3 与 task-8 的 `write_scopes` 有交叠，
`executor-core` 越界写了 `src-tauri/src/lib.rs`（属 task-8）与 `ponysentry/**`（属 task-4），
虽然结果可用，但**机械审计只能事后发现、不能阻止**。这是「平台无文件锁」的客观限制，不是流程疏漏。

**并行收益**：前端（Task 5）与 Rust core（Task 3/8）两条线真正并行，前端在 core 还在编译时就完成了
开关 UI + 浏览器 E2E（Console errors 0 + 三张截图），墙钟上省掉一整轮串行。

## 4. 元协议迭代建议

1. **中断即吊销写权（硬）**：`interrupt_agent` 之后必须显式 `team_task_update action=release` 或
   `reassign`，并在给接棒者的 prompt 里写明「前 owner 已停手，若观察到文件被外部改动，先在回报里标注
   而不是自行覆盖」。本次的 E0560 就是这条缺失造成的。
2. **接棒必须有署名台账（硬）**：Test Agent 接棒时要能回答「这些代码谁写的」。建议在 `state.json`
   增加 `file_authors: {path: [agent, ts]}`，由每次落盘的 agent 自行追加一行。
   （本次 `test-agent-b` 只能如实报告「非本 agent 写入」，可接受但不可追溯。）
3. **`dev-stub-lint.py` 需要多语言支持（工具缺陷）**：至少补 `.rs/.go` 的最小实现（本次已用
   `.dev-team/tools/wave43-rust-stub-purity.py` 打补丁，建议上游收编）。
4. **审查视角矩阵再补一维：隐私/数据出境**。本次三条 BLOCKING 里两条（令牌泄漏、旁路带出他人 span）
   都属这一类，而现有矩阵只有「并发/调度」与「安全/协议」。建议显式列为第三视角，并在
   NFR 基准里加字段 `data_egress_scope`（本次这类问题正是缺这条才让实现者默认「关闭即零出口」）。
5. **wire 级证据应成为隐私相关变更的必备收据**：L3-A 的 `/tmp/w43a-repro`（本地 OTLP sink + protobuf
   解析）比任何静态阅读都有说服力。建议把「可跑通的本地 sink 探针」沉淀为 skill 资产，而不是每次现写。
6. **红相锚定应早于任何源码写入**：本次 protocol-agent 落空桩时，Lead 的基线测试仍在跑，导致基线
   `cargo test` 与新增空桩混编（`[BASELINE CONTAMINATED]`）。应把「基线采集 → 冻结 → 再动工」写成
   带依赖的 DAG 边，而不是靠 Lead 自觉排序。
