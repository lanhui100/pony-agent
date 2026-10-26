# 契约与收口点方案设计（Stage-1 Protocol Freeze）

> Protocol Agent 冻结产物（契约/NFR 唯一编写方）。
> 职责：制定并落盘 NFR 基准与空桩协议，确保空桩纯度与契约合法。

---

## 0. 背景与目标
解决 `ponygo observe status` 报告的 `enforcement_points 缺失，ob2 变更门禁将空转` 缺陷。
依据 `~/.agents/skills/observability-onboarding/references/declaration.schema.json` 与 `declaration-rubric.md`，
为 `docs/observability.yaml` 定义并冻结真实合法的横切收口点。

---

## 1. 唯一合法收口点（Enforcement Points）设计

### 1.1 `error_boundary`（异常边界与降级收口）
- **唯一合法路径 (path)**：`src/components/common/ErrorBoundary.vue`
- **承诺保证 (guarantees)**：`[breadcrumbs, trace_id]`
  - `breadcrumbs`：异常捕获时附带组件与全局面包屑轨迹；
  - `trace_id`：自动回填 `activeTraceId` 并调用 `reportError` 接入 PonySentry。
- **禁止越界正则 (forbidden_in_other_files)**：
  - `pattern`: `\bonErrorCaptured\s*\(`
  - `hint`: 前端组件级异常捕获必须统一使用 ErrorBoundary.vue 统一收口与上报，禁止在别处私自声明 onErrorCaptured

### 1.2 `http_client`（出网网络请求与追踪注入收口）
- **唯一合法路径 (path)**：`crates/pony-agent-core/src/agent/otel/mod.rs`
- **承诺保证 (guarantees)**：`[traceparent, timeout]`
  - `traceparent`：通过 `with_current_traceparent` 与 `traceparent_header_map_from` 统一管理 W3C Trace Context 注入；
  - `timeout`：严格遵循 3000ms 硬超时约束。
- **禁止越界正则 (forbidden_in_other_files)**：
  - `pattern`: `\bheader\s*\(\s*["']traceparent["']`
  - `hint`: 出网 HTTP 请求必须通过 otel::with_current_traceparent / traceparent_header_map_from 统一注入 traceparent，禁止手写硬编码请求头

---

## 2. 契约校验与门禁物理收据

1. **Schema 结构合法性校验**：
   - 验证工具：`jsonschema.validate` 严格校验 `references/declaration.schema.json`
   - 结果：`SUCCESS`（退出码 0，无未知字段，四维齐全，`reviewed_at` 为 ISO 日期）。

2. **声明一致性判定 (`check-declaration.sh`)**：
   - `metrics`: PASS（desktop 交付形态结构性 N-A）
   - `tracing`: PASS（static=pass, dynamic=pass）
   - `logs`: PASS（desktop 交付形态结构性 N-A）
   - `issues`: PASS（static=pass, dynamic=pass）
   - 结论：`PASS (exit=0)`

3. **变更级增量门禁 (`audit-diff.sh`)**：
   - 验证命令：`bash ~/.agents/skills/observability-onboarding/scripts/audit-diff.sh --decl-approved`
   - 结果：`[OK] audit-diff: 无越界实现、收口点完整 (exit=0)`

4. **NFR 基准落盘**：
   - 基线文件：`.dev-team/nfr-baseline-wave47.json`

---

## 3. 后续阶段约束（Hand-off Criteria）
- **Test Agent (Stage-2)**：依据本方案编写正反例测试，验证 audit-diff 在出现未授权的 `onErrorCaptured` 或硬编码 traceparent header 时能够可靠拦截（exit 2），并完成双模态红相锚定。
- **Executor (Stage-3)**：保持收口点契约纯洁性，实施必要代码收敛并确保 `ponygo observe audit --level ob2` 通过。
