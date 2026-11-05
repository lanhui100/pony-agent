# 元框架复盘：Rust harness 可扩展性（ADR 0064–0068 + T1–T4）

> 任务：把「Rust harness 的可扩展性如何保证」从一次咨询落成 5 篇 ADR 与 4 个实施任务，明确不做 workflow。
> 规模：B 级偏 C（跨内核分派、进程边界、安全面），Lead + 4 常驻（Protocol / Test / Executor×2），审查走 Subagent 池。

## 1. 通信拓扑与信噪比

- **做对的**：侦察全部外派 —— 两个 Explorer（DSH Cordis 架构 / pony-agent 扩展面）与三路 Phase-0 参谋（产品 / 架构 / 风险）都不占常驻槽位，Lead 只保留裁决与装配。三路参谋直接改写了两处波次设计（T1∥T2 不可并行、T4 拆骨架/冻结），这是本轮 ROI 最高的外包。
- **做对的**：验收 API 表面由 Test Agent 一次性冻结后广播，「命名不一致 → 红相失败」的口径纠纷只发生一次就收敛。
- **信噪比问题**：文件所有权（mod.rs 单行、tests/ 归属、observability.yaml 双写）在开工前没一次性定死，前 40 分钟反复用消息协商。根因：派单只写「写域」，缺「装配面 / 共享单文件 / 文档面」三类所有权。
- **降噪缺口**：Executor 上报编译中途态对 Lead 没有决策价值，还触发了一次误判（把他人的 tui.rs 编译错误当本波红相）。应约定只报可判定节点。

## 2. 门禁穿透与误杀率

- **真实拦截（未穿透）**：① 只读模式按工具名嗅探的漏洞被 Test Agent 独立复现（外部写工具 `status=Ok`），修复后由 `permission_denied` 用例锁死；② `swap_tool_surface` 的「可见但不可执行」中间态被红相抓到；③ 追加外部工具会让内建工具面落入 fallback（重复 + 变序，破 KV-cache 前缀）——由 Executor 主动上报交界地雷，用 `[KV-CACHE-PREFIX]` 断言锁死。
- **误杀**：一次 TEST-DEFECT（swap 用例传全量 builtin 快照 + 部分 handler 表，与「整面自洽」不变量冲突），裁定为测试自相矛盾、只改测试；一次「并行测试污染全局审计缓冲」被 Executor 用串行/并行对照证据澄清为实现正确。两次都靠机器证据收敛，没有为绿灯改实现。
- **门禁被环境穿透**：`src/bin/tui.rs` 属他人未提交改动，使无目标过滤的 `cargo check`/`cargo test` 间歇红。收敛办法是把门禁降为目标级（`--lib` / `--test <name>` / `--bin`），并点名 PRE-EXISTING-EXTERNAL。教训：共享工作区里全量门禁不是可靠对照组。
- **未做**：L2-AT 破坏性注入没有独立成军，而是并入 Test Agent 验收集（插件超时/崩溃、MCP 崩溃/超时、配置全坏软降级）。属有意裁剪，代价是与 dev-team 全量 NFR 门禁不完全对齐。

## 3. 分工契约与隔离有效性

- **分权有效**：测试文件始终由 Test Agent 独占，两个 Executor 全程未写 `tests/`；「实现即真相」的自证路径被结构性掐断（否则 KV-cache 与只读漏洞极易被写成"看起来对"）。
- **并行实操**：T1 与 T3 真正正交（tools/dispatcher/provider vs plugin_host/hook_commands），并行无写冲突；T2 依赖 T1 的判断由架构参谋推翻了我的原计划，事后被 `provider/mod.rs` 的 fallback 地雷证实——架构视角的否定意见价值高于执行速度。
- **隔离失效点**：`agent/mod.rs` 单行注册被两模块争用（以「Lead 授权单行装配例外」收敛）；`docs/observability.yaml` 曾出现「归 Lead / 归 T2」的口径反复，最终由 T2 一次性收口双方新增面。
- **契约账本**：`.dev-team/contract-matrix-extensibility.md` 约束了「五契约 + 变更类型 + 兼容证据」，并在 T4 复核时纠正了 ADR 两处笔误（mcp-binding → mcp-config、source 枚举 5 → 4），证明「schema 是投影、代码是实况」的纪律生效。

## 4. 元协议迭代建议

1. **派单模板增补三类所有权**：`write_scopes`（代码）+ `assembly_owner`（共享单文件如 mod.rs / generate_handler!）+ `doc_owner`（observability.yaml / 契约 schema）；未声明默认由 Lead 装配。
2. **门禁模板增补目标级等价物**：共享工作区场景下，全量 `cargo check/test` 必须附 `--lib` + `--test` 目标清单与 PRE-EXISTING-EXTERNAL 判定口径。
3. **冻结「可观测面」而不只是签名**：本轮 `surface_contract_views` 需放宽为 `pub` 才可验收，暴露「冻结签名 ≠ 冻结可观测性」。
4. **中期汇报降噪**：只报可判定节点（编译通过/失败、用例红/绿、契约偏差），禁止中途态当进度。
5. **`.dev-team/blast-keywords.txt` 增补**：`Command::new`、`Stdio`、`env_clear`、`process_group`、`O_EXCL`、`deny_unknown_fields`（本轮新增进程与配置解析面全部命中）。
