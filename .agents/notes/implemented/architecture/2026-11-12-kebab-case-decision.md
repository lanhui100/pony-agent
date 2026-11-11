# kebab-case 技能名称校验裁决与非 ASCII 技能名处理

Status: implemented
Date: 2026-11-12

## Context
在技能加载流程中，为了统一规范并避免歧义，我们引入了 kebab-case 名称校验（`^[a-z0-9]+(?:-[a-z0-9]+)*$`）。但在实施中，该门禁与已冻结的用例 `wave58_singleton_isolation` R3（要求不可信载荷必须转义保留，不得整体丢弃）发生了正面冲突。此外，非 ASCII 技能名（如中文目录）目前在扫描期即被静默丢弃，影响了多语言环境下的可调性。

## Decision
我们决定采纳方案 B：
1. 校验逻辑区分维度：仅在模型自动调用侧执行严格的 kebab-case 拦截（`model_invocable` 为 false）；人类侧保持兼容，对于非 kebab-case 或非 ASCII 名称，系统在目录中以 `omitted skill:<name> (invalid_name)` 状态呈现，确保 `/skill:<name>` 路径在解析时能够正确映射到其物理目录，且不破坏现有测试契约。
2. 非 ASCII 处理：支持 UTF-8 目录名，通过映射机制访问。

## Alternatives considered
### 方案 A：授权修改冻结用例
修改 `wave58` 契约，将非法名称处理方式调整为“丢弃且不以任何形式进入前缀”。
- **否决理由**：需承担破坏既有冻结测试契约的风险，且需建立完善的 TEST-DEFECT 流程，成本过高。
