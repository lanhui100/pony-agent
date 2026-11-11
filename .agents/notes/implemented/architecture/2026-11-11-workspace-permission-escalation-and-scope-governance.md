# Agent Note: 工作区外部访问提权机制与受控授权协议

Status: implemented

## Problem

Pony Agent 作为运行于本地桌面的通用智能体，对工作区外部的文件访问采取物理防御边界：

1. **写操作死锁**：早期设计对工作区外部写操作一律物理阻断返回 `outside_workspace_write_denied`，且授权存储拒绝任何写 scope，导致跨项目协作、修改用户外部配置或产生成果物时被彻底死锁；
2. **缺乏人机交互闭环**：当发生越权拦截时，缺乏直观、可操作的人机审查（HITL）交互卡片，用户无法在知情前提下安全放行外部路径；
3. **权限作用域粗粒度隐患**：若缺乏细粒度 `read` 与 `read-write` 作用域约束，容易发生授权读同时意外放行写操作的安全穿透。

## Decision

1. **标准化升权引导契约**：
   - 越权写被阻断时，除明确错误码外，统一追加标准自愈标记：`[sandbox: escalation available — retry with explicit authorization]`；
   - 智能体据此知晓当前受阻并非不可逾越，可引导用户发起提权申请。
2. **细粒度 AuthorizeStore 作用域**：
   - 底层 `AuthorizeStore` 严格区分 `read` 与 `read-write` 作用域；
   - 路径分类器严格执行 Fail-Closed：只有显式获得 `read-write` 授权的外部路径才放行写入，普通只读授权一律拒绝写操作；
   - 路径一律进行规范化（`canonicalize`）并抵御词法逃逸（`..` 穿越）。
3. **人机确认（HITL）与受控卡片**：
   - 提权申请通过 `PendingControlRequest`（Kind: `approval`）生成，前端弹出提权确认卡片，展示目标路径、操作类型及智能体给出的理由；
   - 用户可明确选择拒绝、单次授权或持久授权，批准后写入白名单并解锁执行。
4. **历史溯源与接替**：
   - 本决策源自并完全接替历史决策 `docs/archive/decisions/0029-workspace-permission-escalation.md`。

## Alternatives considered

- **提供全局一键关闭沙箱开关**：
  在设置面板提供全盘读写免审查开关。
  *落选原因*：严重破坏 Fail-Closed 安全底线，失控或有缺陷的智能体可能误删系统关键目录，失去最小特权与审计追踪。
- **由后端进程自动判定信任并静默放行**：
  根据路径类型（如用户 Documents 目录）由后端自动隐式授权。
  *落选原因*：破坏透明度与用户知情权，越界访问必须经过明确的人工知情许可。

## Consequences

- 智能体具备合法可审计的外部路径提权与写操作能力；
- 保持严格的最小特权原则，只读与读写授权严格物理隔离；
- 前端有直观的提权审核交互，操作透明且可回溯。
