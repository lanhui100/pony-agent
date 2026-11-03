# 元框架复盘（Wave-62：工具级取消）

> 对象：清偿 wave-60 §6-ter A9（长工具执行期不真杀）。工作区：隔离 worktree `/home/dm/wt/pony-wave62`。
> 交付：`.dev-team/wave62-delivery.md`；契约：`.dev-team/contracts/wave-62-tool-cancel.md`（v1 + 补遗 v2）。
> 审查收据：`review-wave62-concurrency.json`、`review-wave62-contract.json`（均 FAIL）、`review-wave62-recheck.json`（PASS，lint exit 0）。
> 交付区间：`60e7cb8` → `a09c10c`（tag `wave-62`；红相 tag `wave-62-red` = `e4bd02e`）。

## 1. 通信拓扑与信噪比
- 复用既有常驻 Teammate（protocol/test/executor-core），零新增名册；三个 L3 走 Subagent 池（峰值 4，≤8）。
- **写域隔离第一次真正生效**：因主工作区被并发 wave-61 占用，本波全程在 worktree 内完成，主工作区零污染（`git status` 仅 wave-61 自己的 3 个文件）。wave-60 那次"hunk 过滤提交"的临时手段这次没有再用。
- 信噪比：executor-core 主动预警"C5 必经的取消码升格点"（`tool_cancelled` 会被 handler_failure 改写成 `cancelled`），避免了实现后才发现判据不可达；test-agent 主动请裁决两处词表口径而非自行放宽断言。

## 2. 门禁穿透与误杀率
- **机器门禁全绿（7+2 用例）但判据与真实链路不符**：冻结的 C3 用例直连 `ToolRouter`，恰好绕过生产桥接 `GovernedToolExecutor→GovernedDispatcher→RouterPrimitiveHandler`，而后者用 `Default::default()` 重建上下文丢掉了取消令牌 ⇒ 真实 app 中"停止后命令仍跑完"。
- **两路 L3 独立复现同一 BLOCKING**（L3-A 附可跑反例：cancel 后 1217ms 返回、副作用 marker 已生成）⇒ 判定 FAIL，Lead 一票新增 **C5 端到端判据**（生产执行器 + marker 副作用断言）并红相复锚后修复。这是本波唯一实质缺陷，也是本波门禁体系最有价值的一次拦截。
- 误杀：0。L3 提的"取消分支未对称清理"从 ADVISORY 升级为必修（成本低、与 timeout 分支同源）；"Unix 无进程组杀"判为上游既有局限（`process.rs` 自述 direct-child only），不改判。
- 门禁自身局限：宿主 cgroup `pids.max=250` 下 `-j 2` 会触发 rustc fork EAGAIN ICE，全程必须 `-j 1`；这是环境事实，已在每条收据标注。

## 3. 分工契约与隔离有效性
- 测试分权再次守住：Executor 两次请求改测试形状均被拒（一次转 Test Agent 执行纯形状迁移，一次由 Lead 裁决词表口径），冻结用例全程零改动。
- 契约演化闭包良好：补遗 v2 以"A1–A3 + C5"四条钉死修复面，实施者按条对号，recheck 逐条 FIXED。
- Lead 侧浪费点：等 L3（两个 subagent）时用了 3 次 `sleep` 轮询（合计约 8 分钟空转），而 `wait_agent` 对 Subagent 恒返回 `no-active-peer`。改进同 wave-60 建议：改为"文件 mtime/完成通知驱动 + 期间做交付文档"。

## 4. 元协议迭代建议
1. **"判据必须穿过生产桥接"入 skill 硬条款**：凡工具/执行器类改造，验收用例必须至少一条经**生产默认装配**（本波 C5 就是这么补出来的），禁止只测底层组件。
2. **给 review 任务书附带"桥接图"**：Lead 应在任务书里写明"从入口到实现的装配链"，让审查者定向找"边界丢态"。本波正是靠审查者自己画链路才抓到。
3. **把 `-j 1` 这类宿主资源铁律写进任务书模板**：本波三次 ICE 都发生在未带并发限制的命令上，浪费约 6 分钟。
4. **merge 时机显式登记**：并发波次场景下，"分支已交付但未合并"必须作为交付状态字段（本波 `state.json` 已加 `merge_status: pending`）。
