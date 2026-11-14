# 元复盘 · wave-acp-client-p0-p2（ACP 外部 Agent 接入，B149/B150/B151）

> 复盘对象：本次 Agent Team 的**协作元框架与组织协议**，不是业务代码细节。
> 波次：acp-client-p0-p2（tier C，基线 f4146970）。成员：Lead + protocol-agent + test-agent + executor；审查以 fresh-context subagent 挂载（Gate-1 / Gate-2）。

## 1. 通信拓扑与信噪比（Topology & Noise）

- **拓扑**：3 名常驻 teammate（协议特派员 / 独立测试者 / 战术实施者）+ 2 路按需 subagent 审查，常驻 4 人 ≤ 8 上限。角色边界基本成立：空桩/契约由 protocol-agent 落盘、验收测试由 test-agent 独占写权、业务实现由 executor 独占。
- **信噪比**：回报统一要求 ≤15 行且带 `path:line` + 机器 Exit Code，实际落在 6~14 行、无过程性吹捧；两次审查收据走严苛 JSON（`schema_version=2` + `attestation.self_signed=false`），`dev-review-lint.py` 均 Exit 0。
- **真实损耗**：executor 前两轮**空转失败**（一次 claim 后无落盘、一次读完 800 行测试后上下文压垮），根因是「一个 task = client.rs + session.rs + orchestration.rs 三文件 + 800 行测试」的超大切片；拆成「只改 client.rs」后当轮产出 484 行并附收据。test-agent 亦出现 2 次 turn 失败（丢失 prep 结果后重做）。
- **改进**：单轮切片上限应写成**机械阈值**（≤1 个文件族、预期改动 ≤400 行、必读材料 ≤300 行），而非依赖 Lead 临场判断。

## 2. 门禁穿透与误杀率（Gate Penetration）

- **有效拦截（本轮最大价值）**：红相在 3 个波次各锚定一次（P0 26 用例 / P1 14 用例 / P2 5 用例），跨轮冻结未被实现者改写；Gate-1 与 Gate-2 **各自独立**抓到同一条真缺陷——`session/prompt` 被 `ACP_REQUEST_TIMEOUT=5000` 硬顶，使 `timeoutMs`/`--timeout-secs`/执行策略全部不可兑现（真实 agent 一轮 >5s 必失败）；Gate-1 另抓到「取消令牌未联动，`stop_turn` 杀不掉外部 agent」。两条若漏放，交付即「测试全绿但生产不可用」。
- **穿透（漏网）**：`npm run verify` **不跑 Rust 集成测试**（仅 `cargo:check:shared`），因此工具面 38→39/45→46 引发 4 份冻结契约测试（6 用例）变红时 verify 仍 Exit 0；该回归由 Gate-2 的 Q6 反例发现。这是**结构性门禁缺口**，非个案。
- **误杀/噪音**：Gate-1 报 8 条 ADVISORY、Gate-2 报 5 条，复核后无一条属误杀（均有 path:line）；5 条转 backlog（B164~B168），2 条当场修复（exit code 映射、dead code），1 条改文档（observability 曾声明不存在的 `acp.session` span）。
- **时序违规（自证）**：`review-plan.json` 语义属**实施前**门禁，本轮实际在实施完成后执行（首轮收据 reviewed_commit=b61682e1）。收据合法但时序失真，须在下一波纠正，否则「方案冻结」形同虚设。

## 3. 分工契约与隔离有效性（Contract Isolation）

- **隔离成立面**：写入域按波次切分（protocol: `acp/` + `.dev-team/`；test: `tests/` + fixtures；executor: `acp/` 实现 + `agent/tools/orchestration.rs` + `bin/cli.rs`），无两人同时改同一文件；git 写操作全程只有 Lead（10 次提交，每次先取 `git status --porcelain` + `git diff --cached --stat` 暂存区收据）。
- **越界（两处，均已留痕）**：① protocol-agent 为工具面变更同步了 `agent/provider/tests.rs`（编译兼容衍生写入，事后补声明 `allow_derivative_scopes`，方向可接受但应事前声明）；② protocol-agent 在「空桩」amend 里**顺手实现**了 `set_prompt_budget` 函数体（空桩纯度被破坏）——好处是 BLOCKING 修得快，坏处是「契约层不含逻辑」不再成立，且 Lead 只能事后核出。
- **Lead 边界自查**：Lead 未写业务逻辑，只做 (a) 文档/ADR/可观测性声明、(b) 过文件规模门的**约 10 行**纯抽取（`acp_delegate_tool_definition` + 注释合并，零行为变更）、(c) 若干 1 行注释合并。均 < 50 行阈值，但严格说 (b) 应下发给 executor。
- **契约账本**：滚动契约矩阵（`contract-matrix-acp-p0.md` / `-p1.md`）经历两轮 amend 仍保持原地改写 + 单行变更流水，未膨胀；但 P1-9/P1-10 断言面是在**阻断被发现之后**才补，说明矩阵「先于实现」的属性只在 P0 守住。

## 4. 元协议迭代建议（Self-Evolving Protocol）

1. **verify 门禁必须覆盖冻结契约清单**：把「工具面/契约类」测试二进制纳入 `npm run verify`（或新增 `verify:contracts` 并由 verify 调用）。本轮 6 个红用例穿透 verify 即为反例。
2. **空桩纯度需机械门**：`dev-stub-lint.py` 只覆盖 Python AST；Rust 侧应加轻量检查（`unimplemented!` 计数 + 禁止 `Command::new`/`fs::` 出现在 stub 文件），否则「契约层不含逻辑」只能靠 review 抓。
3. **executor 切片机械化**：把「单轮 ≤1 文件族 / ≤400 行改动 / 必读 ≤300 行」写入 skill，并在 `team_task_create` 阶段按此拆分（本轮是失败两次后才拆）。
4. **计划门禁时序前移**：`review-plan.json` 必须在红相锚定前产出，并让 `dev-meta-lint.py` 校验其 `reviewed_commit` 早于首个实现提交（当前只校验文件存在）。
5. **derivative 写域事前声明**：凡工具面/枚举基数变化必然连带既有契约测试，应在建任务时登记 `allow_derivative_scopes`，避免事后补票。

## 4.1 审查轮次流水（机器收据：`.dev-team/review-plan.json` / `review-join.json`，schema_version=2）

| 轮次 | Gate-1（方案对抗） | Gate-2（L3-P 六问） | 本轮被抓住的真缺陷 |
|---|---|---|---|
| r2 @3ebb0742 | FAIL（2 BLOCKING） | FAIL（2 BLOCKING） | ①prompt 被 5000ms 硬顶 ②取消令牌未联动 ③工具面 38→39/45→46 漏同步 4 份冻结契约 |
| r3 @4ba50955 | FAIL（1 BLOCKING） | FAIL（1 BLOCKING） | ④cli/orchestration 两入口未接线（库层 setter 零调用点）⑤工具入口缺省仍 5s 硬顶 |
| r4 @be302170 | **PASS**（0 BLOCKING / 4 ADVISORY） | **PASS**（0 BLOCKING / 5 ADVISORY） | 无 |
| 处置 | 逐条修复并以提交闭环 | 同上 | ADVISORY 转 B164~B170；当场修 3 条（exit code、dead code、capabilities 不虚报） |

**保真声明**：r2/r3 的 FAIL 收据不做美化，保留在 git 历史与 backlog 归档里；r4 PASS 的审查对象是 `be302170`（源码面 `de3f7a5e`）。共 5 条 BLOCKING，全部由独立审查在交付前钉住，零漏放到用户手上。

## 5. 结论

本轮团队在**红相独立分权**与**双路对抗审查**上取得真实收益（4 条 BLOCKING 全部在交付前被机器反例钉住并修复），代价是 2 次 executor 空转 + 1 次门禁时序失真。下一波优先落地建议 1（verify 覆盖契约测试）与建议 3（切片机械阈值），这两条直接决定「门禁是否被穿透」与「token 是否被空转吃掉」。
