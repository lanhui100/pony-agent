# Wave-15 交付归档：Token 指标口径修复（输入上下文 / 生成速度）

- 日期：2026-10-16
- 契约：`.dev-team/contracts/wave-15-token-metrics.md`
- ADR：[0035 Token 指标口径](../../decisions/0035-token-metrics-per-call-semantics.md)
- 物理收据全表：`.dev-team/wave15-green-receipts.md`
- NFR 基准：`.dev-team/nfr-baseline-wave15.json`

## 用户可见结果

| 场景 | 修复前 | 修复后 |
|---|---|---|
| 多跳工具轮次（3 次模型调用：350K/360K/370K）的输入框上下文圆环与状态栏「输入」 | 1.08M（跨调用累加，圆环破 100%） | **370K**（最近一次已完成调用，跨跳不累加） |
| 多调用轮且无 per-call 来源 | 显示累加虚高值 | 显示无数据（S4 纪律，绝不显示聚合值） |
| 生成速度（每次模型调用完成时） | 轮次结束才有值/恒为 `—` | 该次调用完成即产出（100 output / 1200ms / TTFT 200ms → 100 t/s） |

## 非目标（未变更）

- 会话累计计费输入（S2）与消息级 `tokenCount`；
- 状态栏「输出 / 缓存读取」的会话累计口径；
- trace 事件模型整体结构、历史数据、依赖。

## NFR 达标（L3-P 五问基准映射否决制，全部 PASS）

| 问 | 基准字段 | 结论 |
|---|---|---|
| 外部调用超时 | `external_call_timeout_ms <= 3000` | PASS（本波无新增外部 IO） |
| 日志规范与防吞 | `logging.*`（PA-119 债务口径） | PASS（无新增裸 print/空 catch） |
| 资源泄漏防御 | `resource_release` | PASS（无新增队列/线程/句柄/监听器；活跃轮记录随轮复位） |
| 并发与幂等 | `concurrency_lock` 等 | PASS（克隆快照覆盖语义，幂等） |
| 配置与自检 | 兼容与门禁完整 | PASS（legacy 入口保留，验收与回归全绿） |

## 已知限制与后续工单

1. `wave15-D1`：事件折叠重建路径（`projection.rs` TurnEnd settle）仍会把 per-call duration 覆盖为整轮 elapsed；仅影响「重建后且无 providerCallRecords」轮次的速度回退。已登记 state.json 债务。
2. `wave15-P3`：初始调用完成时无 hop 事件，其 per-call 值在首个 followup 完成前不作为「最新已完成调用」展示（窗口内展示上一轮最近调用值）。

## 证据文件

- `browser-composer-context-circle.png`：真实 Chrome 无头渲染（composer 上下文圆环存在、Console Error = 0）
- `browser-streaming-state.png`：streaming 兜底态渲染
- 跨语言 wire 凭证：`.dev-team/evidence/wave15/wire-red-frozen.json`（红）/ `wire-green-frozen.json`（绿）
