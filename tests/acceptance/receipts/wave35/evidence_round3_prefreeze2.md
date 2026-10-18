# wave35 验收证据(第三轮:freeze-1 作废 + P0 独立复现 + 预存 fail-open 核验)

时间: 2026-10-18 12:15 (+08:00) / 门禁基线 f70b1f3 / 环境: L3-B 判 FAIL,executor-c 修 task-7 中

## 1. freeze-1(b7139b0)我方 `--full` 结果作废标记
我方 `--full` 在 Lead 叫停前**已完成**(Exit 0 / 1061 passed / 12.39s),但既然 b7139b0 被 L3-B 判 FAIL,
下列收据首行已就地标记 `# PRE-FREEZE-2,非最终交付收据…`,**不作为验收依据**:
`full_gate_FROZEN_b7139b0_20261018T120751.log`、`lib_full_20261018T120756.log`、
`lib_list_20261018T120755.txt`、`retry_subset_20261018T120756.log`、`tool_surface_wave20_20261018T120756.log`。
freeze-2 落地后须重跑 `--full` + `--compare-logs` + G1/G2 复算 + `--list`。

## 2. P0(retry.rs char-boundary)**独立复现** —— 不是照抄 L3-B
方法:把 `retry.rs` 的 `strip_endpoint_fragments` **逐字**(`sed -n '/^fn strip_endpoint_fragments/,/^}/p'`)
提取进 /tmp 独立小 crate(不占本仓库 target 锁),对 4 类输入做探针。
可复用脚本: `tests/acceptance/remediation/wave35_retry_boundary_probe.sh`
原始输出: `retry_boundary_probe_20261018T121313.log`

AS-IS(freeze-1 代码)实测:

| 探针 | 输入形态 | 结果 |
|---|---|---|
| P1 | 与 freeze-1既有回归测试**同形**(URL 前为 ASCII `=`/空格) | **ok** |
| P2 | 中文紧贴 URL `错误https://…:42953/v1` | **panic** `byte index 23 is not a char boundary; it is inside '误' (bytes 22..25)` |
| P3 | 全角冒号紧贴 URL `错误：https://…:42953/v1` | **panic** `byte index 26 … inside '：' (bytes 25..28)` |
| P4 | emoji 紧贴 URL `🔒https://…` | **panic** `byte index 20 … inside '🔒' (bytes 19..23)` |

**独立结论(比 L3-B 更进一步的一条事实)**:P1 为 ok ⇒ **freeze-1 自带的新回归测试不足以捕获该 P0**
(它的输入在 URL 前总有 ASCII 分隔符)。因此 freeze-2 的回归测试**必须**包含"多字节字符紧贴 `://`"的输入,
否则修法可被退回而测试仍全绿。

变异敏感性(脚本已就绪,待 freeze-2 后给出最终读数):脚本会把 `let scheme_start = head …unwrap_or(0);`
机械替换回 `index + 1` 字节切片,并断言 AS-IS 全绿、MUTATED 出现 panic/FAIL。当前 freeze-1 两侧均为 FAILED(因代码本身仍含缺陷)。

## 3. 预存缺陷(非本波引入,**不修**,独立核验)
`governed_executor.rs:150-159` 的 Draft 写保护按 `args["session_id"]/["sessionId"]` → 环境变量 → `"default"` 解析会话,
而真实会话由 `runtime/mod.rs:298`(`DispatchContext.session_id`)注入、`write_file` 的 schema 不含 `session_id`:
- 门禁实际只查 `has_active_draft_plan(<解析出的 id>) || has_active_draft_plan("default") || (args 为对象时) has_active_draft_plan("test-session-draft-gate")`;
- 非 default 会话下,草稿计划登记在真实 session id 上 ⇒ 前两项均不命中 ⇒ **fail-open**(L3-A 结论成立,我方独立读码复核一致);
- 额外发现(我方):环境变量回退写的是 `.map(|_| "env_session")`,**忽略变量取值**并用常量 `"env_session"` 参与匹配,
  该常量不可能等于任何真实 session id ⇒ 亦无法闭合该 fail-open;第三个分支 `test-session-draft-gate` 由
  `_final_arguments.is_object()` 触发,属测试专用 id,不构成生产兜底。

**本波未削弱任何原本生效的保护**(独立核验,`git diff b7139b0^ -- governed_executor.rs` 全文仅两处):
1. 生产判据唯一改动 = `is_write_or_exec` 追加 `&& descriptor.identity.primitive_name != TOOL_PLAN_CONTROL`
   —— 只对**唯一一个** canonical primitive(`plan_control`,即 `start_execution` 解除草稿锁定的入口)豁免,
   其余 Write/Execute 描述符仍逐个过原门禁 ⇒ 保护面未被放宽。
   该豁免是"解除自锁死"的必需项(否则 merge/complete_step/replace/start_execution 全 Deny,plan 永离 Draft 态),理由已写在代码注释;
2. 其余改动在 `mod tests` 内(fixture 补 `expectedHash`,满足 wave-22 读前写不变量),断言总数 59→**59** 未降。

## 4. 待办(等 freeze-2)
1. `bash tests/acceptance/remediation/wave35_retry_boundary_probe.sh` → 期望 `PROBE: PASS`(AS-IS 0 / MUTATED ≠0);
2. `bash tests/acceptance/remediation/wave35_gate.sh --full`(独占锁,900s 硬顶)→ 新收据;
3. `--compare-logs <freeze-2 全量> <Lead 交叉对照>`;
4. G1 对 9 文件复算(retry.rs 基线仍取 f70b1f3,门禁每次现算,无缓存快照)+ `--list` 计数(#[ignore] 必须仍为 0)。