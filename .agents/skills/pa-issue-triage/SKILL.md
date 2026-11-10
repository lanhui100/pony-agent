---
name: pa-issue-triage
description: 遇到错误异常或用户主动要求时记录当前会话问题（含 session_id 与排查线索）；支持按优先级聚合编排待办、制定可持久化且支持断点续做的执行计划、推进关键进展并在解决闭环后清理待办与临时文档。
---

# pa-issue-triage —— 会话问题分流、断点续做与任务治理

本 Skill 为当前工程在会话中遇到运行时/编译/逻辑错误，或用户下达排查要求时，提供**“捕获 $\to$ 聚合分流 $\to$ 计划与断点续做 $\to$ 证据闭环与清理”**的全生命周期规范。

---

## 1. 核心触发时机

1. **自动触发（错误捕获）**：
   - 命令执行返回非零退出码（Non-zero exit code）、编译崩溃、工具调用超时、Panic、未捕获异常。
   - 会话中遇到复杂的逻辑 Regression 或环境依赖缺失。
2. **人工主动触发**：
   - 用户明确指示：“记录这个问题”、“把这几个 bug 规划一下”、“待办整理与推进”、“继续上次没修完的问题”。

---

## 2. 状态模型与存储契约（Single Source of Truth）

所有待办项和断点状态统一落盘在 `.agents/issue-triage/` 目录下（受版本控制或本地隔离）：

```text
.agents/issue-triage/
├── issues.jsonl         # 活动问题待办池（JSONL 格式，追加/修改原子行，具备去重指引）
├── current-plan.json    # 当前编排计划与执行断点状态（Checkpointer，用于会话续做）
└── archive/             # 已闭环问题的归档日志（按日期/月度分割的 JSONL）
```

### 2.1 问题条目 Schema (`issues.jsonl`)
每行一个 JSON 对象：
```json
{
  "id": "ISSUE-20261110-001",
  "session_id": "pa_01a1c5a7-5c4e-7001-ac27-72fed3013c7d",
  "fingerprint": "core::runtime::executor_timeout_hop2",
  "priority": "P0",
  "status": "pending",
  "title": "简明问题标题",
  "context": {
    "command": "cargo test --test it",
    "exit_code": 101,
    "error_snippet": "thread 'main' panicked at...",
    "reproduce_hint": "复现步骤或最小入参"
  },
  "occurred_count": 1,
  "created_at": "2026-11-10T16:00:00+08:00",
  "updated_at": "2026-11-10T16:00:00+08:00"
}
```
* **状态枚举**：`pending`（待处理） | `in_progress`（处理中） | `blocked`（受阻） | `resolved`（已验证解决）

### 2.2 计划与断点 Schema (`current-plan.json`)
```json
{
  "plan_id": "PLAN-20261110-01",
  "target_issues": ["ISSUE-20261110-001"],
  "objective": "修复 core 执行超时并恢复回归测试",
  "status": "in_progress",
  "steps": [
    {
      "step_id": 1,
      "description": "定位 runtime/limits.rs 的超时阈值",
      "status": "completed",
      "evidence": "确认代码第 76 行硬编码 45s"
    },
    {
      "step_id": 2,
      "description": "修改参数并运行单元测试验证",
      "status": "in_progress",
      "evidence": null
    }
  ],
  "last_session_id": "pa_01a1c5a7-...",
  "updated_at": "2026-11-10T16:15:00+08:00"
}
```

---

## 3. 标准操作流程（SOP）

### 阶段一：问题捕获与去重记录（Capture & Fingerprint）
1. **生成指纹（Fingerprint）**：提取 `[模块/文件路径] + [错误类型/关键报错摘要]`。
2. **幂等查重**：
   - 检查 `issues.jsonl` 中是否存在同 `fingerprint` 且状态为 `pending`/`in_progress` 的条目。
   - 若存在：递增 `occurred_count`，追加当前会话的 `session_id` 与最新上下文，不新建条目。
   - 若不存在：分配新自增 ID（如 `ISSUE-YYYYMMDD-XXX`），填充上下文并追加。

### 阶段二：聚合排期与计划制定（Triage & Plan）
1. **查看与分级**：读取 `issues.jsonl`，按优先级（P0 阻塞/编译失败 > P1 核心功能降级 > P2 次要问题）排序。
2. **同类问题聚合**：将相同模块路径、相同根因的 issue 合并到同一个解决计划批次。
3. **写入断点计划**：生成或覆写 `current-plan.json`，并联动调用环境的 `TodoWrite` 同步向用户展示进度。

### 阶段三：断点恢复与执行推进（Resume & Execute）
1. **跨会话唤起检测**：
   - 新会话启动后，若指令涉及“继续做”、“恢复未完成任务”或检测到 `current-plan.json` 的 `status == in_progress`：
   - 首先执行环境自检：运行 `git status` 确认当前工作区是否干净；检查关键依赖与进程。
   - 载入 `current-plan.json` 中第一个 `status != completed` 的 step 继续推进。
2. **进展记录（Progress Milestones）**：
   - 每完成一个实质性 step，立即回写 `current-plan.json` 记录 `status: completed` 与机器输出证据（`evidence`）。

### 阶段四：物理验证与清理闭环（Resolution & Cleanup）
1. **证据契约（Objective Receipt）**：
   - 禁止在没有机器执行结果（测试用例 Exit 0、构建命令 Pass、输出 diff 符合预期）前标记 `resolved`。
2. **状态流转与归档**：
   - 验证通过后，将 `issues.jsonl` 中对应的条目移出，追加至 `archive/resolved-YYYYMM.jsonl`。
   - 将 `current-plan.json` 的 `status` 置为 `completed`（或重置为空闲模板）。
3. **环境卫生清理（Anti-Entropy Cleanup）**：
   - 清理排查期间产生的临时日志、dump 文件、无用测试脚本。
   - 确保 `git status` 无残留脏文件。
