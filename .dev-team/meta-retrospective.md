# 元框架复盘（Wave-55：架构评估缺陷修复波）

> 本轮对象：wave-55——按架构评估结论逐项机器复核后，只修可闭包的 P0/P1 缺陷（编译门禁、并发授权上下文、
> 沙箱诚实性、持久化原子性、渲染层外泄、事件 sessionId、文档事实同步）。历史复盘见 `meta-retrospective-wave54.md`。
> 交付区间：`5fa0ca6` → HEAD（9bf7862 build / 4bc7a7c 红相 / 115ce83 docs / 59a5df7 core / e7430ac web）。
> 任务分级：B（安全敏感 + 跨模块，命中 blast-keywords：lock/thread/net/ipc）。

## 1. 通信拓扑与信噪比

- **拓扑**：Lead + 3 常驻 teammate（rust-executor / frontend-executor / test-agent），审查全部走 Subagent 池
  （3 路 L3 + 1 路 L3-P + 1 路复审），Roster 峰值 4 ≤ 8，未挤占名册。
- **写域正交有效**：`crates/pony-agent-core/src`（rust）、`src/ + src-tauri/ + index.html`（web）、
  `tests/ + crates/pony-agent-core/tests`（test）三域零重叠，全程无脏写冲突。
- **信噪比**：teammate 回执一律「结论 + 文件:行 + Exit Code」三段式，无过程性吹捧。
  **真实浪费点**：L3/L3-P 的审查 JSON 首轮不满足 v2 schema（缺 `schema_version/reviewer/attestation`），
  Lead 补写三轮才通过 `dev-review-lint` —— 派发 prompt 未前置收据骨架，是最大的 Token 漏损。
- **重复劳动**：`cargo check --all-targets` 被 4 个成员各跑 ≥2 次；全量 `npx vitest run` 跑了 5 次。
  缺「门禁命令白名单 + 单次收据共享」约定。

## 2. 门禁穿透与误杀率

- **抓到的真问题（本波最高价值）**：L3 两路各出 1 条 BLOCKING，都是**测试无法覆盖的语义缺口**：
  1) `turn_flow::stream_delta_chunks` 无 session 形参 → 非真流式 provider 的 delta 被前端严格过滤整条丢弃
     （修 fail-open 时**新引入**的功能破损）；
  2) goal 续轮合成 `TurnInput` 丢 `tool_authorization_mode` → read_only 失效；test-agent 深挖出更底层根因：
     旧闸门只比小写 `"write"/"edit"/"bash"`，而生产描述符是 `Write`/`workspace_run_command`，**闸门从未触发过**。
  → 「机器全绿 ≠ 语义正确」：8/8 红相全绿时仍潜伏两处真实 fail-open。
- **正确误杀**：L3-P 判 1 条 BLOCKING（NFR 基准 `guarantee` 声称"不存在无限挂起"夸大），拦下的是
  「我们自己的基准在说谎」，按方案 (b) 修措辞 + 登记缺口，而非改代码迁就文字。
- **假绿风险**：`cargo test --workspace` 首跑 101，红项是既有 flaky（`wave37 t3_0` 的 OnceLock check-then-use）；
  若不复跑会误判回归。已"单跑 3 次 + 全量 3 次"确定性化并加锁，而非用 `--test-threads=1` 掩盖。
- **流程穿透（如实记录）**：① 回归基线未在改码前建立（skill 硬门禁），只能用 HEAD 独立 worktree 事后对照；
  ② 实现者改动了已冻结的红相文件（调用形状迁移本应只由 test-agent 做），Lead 核验"断言零改动"后接受；
  ③ 一次提交用 `git add docs/` 过宽，误将 teammate 产出的审计文档一并纳入。
  ④ Lead 在修 NFR 基准豁免计数时用 `open(p,'w')` 后才因 TypeError 中断，把 `.dev-team/nfr-baseline.json` 截断为空，
  已 `git checkout --` 立即还原并重做（未造成交付污染）。教训：改账本类文件必须「先读入→改内存→再写」，且写前确认已成功解析。

## 3. 分工契约与隔离有效性

- **测试分权有效**：红相由 test-agent 独立编写，多次挡回实现（T7 红相证明修复前 Write 真落盘）；
  `c141_1` 断言改写采用「生产可观察行为」而非「适配垫片自证」，是分权制的正收益。
- **契约迁移边界模糊**：删除 `set_context` 属 API 形状变更，测试迁移与实现同批落地，
  导致「红相先于实现」在物理上不可复现（迁移时 F2 已落地）。应先冻结 API 形状再冻结红相。
- **节奏不均**：frontend-executor 前半程空转等红相，test-agent 串行写 5 个测试文件，一人忙一人等。
- **Lead 负载**：全部 git 写、门禁复跑、基准修订、ADR/提案、审查收据补写都在 Lead；
  teammate 未覆盖「文档治理」「收据 schema 合规」两类工作。

## 4. 元协议迭代建议

1. **审查收据骨架前置**：在 L3/L3-P 派发模板里内联最小 JSON（`schema_version/reviewer.session_id/attestation`）
   并附 `dev-review-lint` 自检一行；本波 3 次往返即可省。
2. **API 形状先冻结再冻结红相**：含公共方法删除/改签的任务，先用 Protocol Agent 落新形状空桩并锚定，
   再冻结红相，否则「红相先于实现」不可验证。
3. **NFR 基准官方支持 `exceptions[]`**：本波流式 idle 语义套不上 `external_call_timeout_ms=3000`，
   首轮只能靠 L3-P 抓出。建议 schema 增加例外条目（`guarantee` 必须可被代码行验证、`known_gap_ref` 必填），
   并让 `dev-nfr-lint` 禁止例外中出现绝对化承诺词，除非有 elapsed 上界证据。
4. **门禁去重**：全量 `cargo test --workspace` 与 `npx vitest run` 由 Lead 在栅栏跑一次并落收据，
   teammate 只跑自身 scope 子集——本波可省 5 次重复全量跑。
5. **既有 flaky 进基线白名单**：`regression-baseline-<wave>.json` 增 `known_flaky[]`，
   后续波次直接比对而非重新定责（本波为一条既有 flaky 花了 2 轮取证）。
