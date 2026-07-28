# Anthropic 2026 年以来 Agent Harness 发展趋势

> 基于 Anthropic Engineering Blog 系列文章整理  
> 整理时间：2026 年

---

## 核心博客文章一览

| 发布日期 | 文章标题 | 核心主题 |
|----------|----------|----------|
| 2026-01-09 | [Demystifying evals for AI agents](https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents) | Agent 评估方法论 |
| 2026-02-05 | [Quantifying infrastructure noise in agentic coding evals](https://www.anthropic.com/engineering/infrastructure-noise) | 评估基础设施噪声 |
| 2026-03-24 | [Harness design for long-running application development](https://www.anthropic.com/engineering/harness-design-long-running-apps) | 生成器-评估器架构 |
| 2026-04-08 | [Scaling Managed Agents](https://www.anthropic.com/engineering/managed-agents) | 生产级 Harness 的虚拟化与接口设计 |
| 2026-06-10 | [The evolution of agentic surfaces](https://claude.com/blog/building-with-claude-managed-agents) | Managed Agents 的实践与演进 |
| 2026 年 | [Building a C compiler with a team of parallel Claudes](https://www.anthropic.com/engineering/building-c-compiler) | 多 Agent 并行协作 |
| 2026 年 | [How we built our multi-agent research system](https://www.anthropic.com/engineering/multi-agent-research-system) | 多 Agent 研究系统 |
| 2026 年 | [Writing effective tools for AI agents](https://www.anthropic.com/engineering/writing-tools-for-agents) | 工具设计最佳实践 |
| 2026 年 | [Building Effective AI Agents](https://www.anthropic.com/engineering/building-effective-agents) | Agent 构建通用原则 |
| 2026 年 | [Effective harnesses for long-running agents](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents) | 长时运行 Agent 的 Harness 设计 |

---

## 趋势一：从单体 Harness 到"脑-手-会话"三层解耦

这是 2026 年最核心的架构演进。**Managed Agents** 博客明确提出了三层虚拟化：

### 三层架构

```
┌─────────────────────────────────────────────────────────┐
│                    Managed Agents                       │
├─────────────────────────────────────────────────────────┤
│  Brain (大脑)                                           │
│  ┌─────────────────────────────────────────────────┐    │
│  │  Claude 模型 + Harness (主循环 / 工具路由)      │    │
│  │  → 调用 execute(name, input) → string           │    │
│  └─────────────────────────────────────────────────┘    │
│                          │                              │
│                          ▼                              │
│  Hands (手)                                             │
│  ┌─────────────────────────────────────────────────┐    │
│  │  Sandbox 执行环境 (容器 / 代码运行 / 文件编辑)  │    │
│  │  → 通过 execute 接口与 Brain 解耦                │    │
│  └─────────────────────────────────────────────────┘    │
│                          │                              │
│                          ▼                              │
│  Session (会话日志)                                     │
│  ┌─────────────────────────────────────────────────┐    │
│  │  Append-only 事件日志                            │    │
│  │  emitEvent(id, event)                           │    │
│  │  getSession(id) → 事件流                        │    │
│  └─────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────┘
```

### 关键设计原则

1. **Harness 离开容器**：Harness 不再活在容器内，它把容器当作普通工具调用 `execute(name, input) → string`
2. **容器 = 牲口**：容器死了就重建，不抢救。新容器通过 `provision({resources})` 标准配方初始化
3. **Harness = 牲口**：Harness 崩溃后通过 `wake(sessionId)` 从会话日志恢复，断点续跑
4. **凭证隔离**：Git token 在 sandbox 初始化时注入，OAuth 存 vault 中通过 MCP proxy 代理，harness 全程不接触凭证

### 性能收益

- p50 TTFT（首 Token 时间）下降约 **60%**
- p95 TTFT 下降超过 **90%**
- 原因：不需要先启动容器再推理，推理可以立即开始，容器按需启动

### 核心理念

> "Harnesses encode assumptions about what Claude can't do on its own. However, those assumptions need to be frequently questioned because they can go stale as models improve."

Managed Agents 定位为 **meta-harness**——不规定具体的 harness 实现，而是提供稳定的接口（session / sandbox / harness），让底层实现可以独立演进。

---

## 趋势二：生成器-评估器（Generator-Evaluator）模式

### 架构演进脉络

```
2025.11 ────────────────────────────────────────────────── 2026.03 ────────────────→
                    │                                            │
┌──────────────────────────────┐    ┌──────────────────────────────────────────┐
│  Initializer Agent           │    │  Planner Agent                           │
│  + Coding Agent              │    │  + Generator Agent                       │
│                              │    │  + Evaluator Agent                       │
│  跨 session 依赖             │    │                                          │
│  context reset               │    │  单次连续 session                        │
│                              │    │  (Opus 4.5 消除了 context anxiety)       │
└──────────────────────────────┘    └──────────────────────────────────────────┘
```

### 三 Agent 分工

| Agent | 职责 | 关键设计 |
|-------|------|----------|
| **Planner（规划器）** | 将用户 1-4 句话扩展为完整产品规格 | 聚焦产品上下文和高层技术设计，不做细粒度实现细节，避免错误级联 |
| **Generator（生成器）** | 一次实现一个 feature（sprint 模式） | 自评 + git 版本控制，sprint 结束前与 Evaluator 协商契约 |
| **Evaluator（评估器）** | 独立评估 Generator 产出 | 使用 Playwright MCP 真实点击应用，评估产品深度、功能、视觉设计、代码质量 |

### 为何需要独立评估器

- **自我评价偏差**：让模型批判自己的产出远比让独立的评估器变得 skeptical 更困难
- **外部反馈循环**：一旦评估器作为外部反馈存在，生成器就有了具体的迭代目标
- **GAN 启发**：将"做的人"和"评判的人"分离，是受 GAN 架构启发的核心设计决策

### Sprint 契约机制

每个 sprint 开始前，Generator 和 Evaluator 进行**契约谈判**：
1. Generator 提出要构建什么以及如何验证成功
2. Evaluator 审查提案，确保建的是正确的东西
3. 双方迭代直至达成一致

### 成本数据

| 项目 | 模型 | 耗时 | 成本 |
|------|------|------|------|
| 复古游戏制作（3-Agent 架构） | Opus 4.5 | ~6 小时 | $200 |
| DAW 数字音频工作站（优化后） | Opus 4.6 | ~4 小时 | $124 |

---

## 趋势三：评估的系统化工程

### 三种评分器

```
┌─────────────────────────────────────────────────────────┐
│                    评估评分器                            │
├─────────────────────────────────────────────────────────┤
│  Code-based (基于代码的)                                │
│  ┌─────────────────────────────────────────────────┐    │
│  │  精确匹配、单元测试、编译检查、AST 对比         │    │
│  │  确定性最高，推荐优先使用                        │    │
│  └─────────────────────────────────────────────────┘    │
│                                                         │
│  Model-based (基于模型的)                                │
│  ┌─────────────────────────────────────────────────┐    │
│  │  LLM-as-Judge 打分                              │    │
│  │  适合自由文本、研究质量等主观维度                │    │
│  │  需要校准对齐人工判断                            │    │
│  └─────────────────────────────────────────────────┘    │
│                                                         │
│  Human (人工)                                           │
│  ┌─────────────────────────────────────────────────┐    │
│  │  发现自动化评估遗漏的 edge case                  │    │
│  │  用于校准 LLM 评分器                             │    │
│  │  识别模型偏见（如总是选 SEO 内容农场）           │    │
│  └─────────────────────────────────────────────────┘    │
└─────────────────────────────────────────────────────────┘
```

### 评估类型

| 类型 | 问题 | 目标通过率 | 用途 |
|------|------|------------|------|
| **能力评估（Capability）** | "这个 Agent 能做什么？" | 从低开始爬坡 | 衡量进展，给团队爬坡目标 |
| **回归评估（Regression）** | "老功能还正常吗？" | 接近 100% | 防止回退，CI/CD 首道防线 |

### 实践建议

- **20-50 个任务起步**：从真实失败案例中提取，不要等凑到上百个才动手
- **评估饱和度**：当通过率接近 100% 时，需要引入更难的任务
- **基础设施噪声**：资源配给差异可导致 6 个百分点的评分差异，需标准化并报告
- **评估工具生态**：Harbor（大规模并行评估）、Braintrust（评估+可观测性）、LangSmith/Langfuse

---

## 趋势四：多 Agent 团队协作

### 案例一：C 编译器项目

**目标**：用多个并行 Claude Agent 构建一个完整的 C 编译器

**Agent 分工**：

| Agent 角色 | 职责 |
|-----------|------|
| 核心开发者 | 修复编译失败，逐个通过测试用例 |
| 代码合并者 | 识别并合并重复代码 |
| 性能优化者 | 改善编译器自身性能 |
| 代码输出优化者 | 优化生成的目标代码质量 |
| 代码评审者 | 从 Rust 开发者视角批判设计，做结构性改进 |
| 文档维护者 | 维护项目文档 |

**协作机制**：

```
┌─────────────────────────────────────────────────────────┐
│                    协作流程                              │
├─────────────────────────────────────────────────────────┤
│  1. 每个 Agent 从上游 git 仓库 clone 本地副本            │
│  2. 通过写入 current_tasks/ 文件获取任务锁               │
│  3. 工作完成后 pull upstream → merge → push            │
│  4. 释放锁，循环下一轮                                   │
│  5. merge conflict 频发，但 Claude 能自行解决            │
└─────────────────────────────────────────────────────────┘
```

**成果**：
- 10 万行 Rust 代码，完全 clean-room 实现（无互联网访问）
- 能编译 Linux 6.9（x86 / ARM / RISC-V）
- 能编译 QEMU、FFmpeg、SQLite、PostgreSQL、Redis
- GCC torture test suite 通过率 99%
- 能编译并运行 Doom

### 案例二：多 Agent 研究系统

**挑战**：
- 开放式问题，没有标准答案
- 不同 Agent 可能走完全不同的路径
- 需要 source quality heuristic（避免 SEO 内容农场）

**解决方案**：
- 使用 LLM-as-Judge 按 rubric 评分（事实准确性、引用准确性、完整性、源质量、工具效率）
- 人工测试发现并修复偏见
- 分步 fan-out/fan-in 模式

---

## 趋势五：从 Agent SDK 到 Managed Agents 的产品化路径

### 产品演进

```
Claude Code (2025.02)          Agent SDK (2025)          Managed Agents (2026.04)
┌─────────────────┐     ┌──────────────────┐     ┌────────────────────────┐
│ 内置 Harness    │────→│ 开放 Harness API │────→│ 托管服务，三层解耦     │
│ 推理循环        │     │ 开发者可构建     │     │ 脑/手/会话分离         │
│ 工具执行        │     │ 自定义 Agent     │     │ 生产级基础设施         │
│ 子 Agent        │     │ 上下文管理       │     │ 自托管沙箱可选         │
│ 上下文管理      │     │ 自定义提示       │     │ 自动恢复/断点续跑       │
└─────────────────┘     └──────────────────┘     └────────────────────────┘
```

### Managed Agents 的额外能力

| 能力 | 说明 | 阶段 |
|------|------|------|
| **Dreaming（梦境机制）** | 定时扫描 session 和 memory store，提取模式，整理记忆，让 Agent 变好 | 研究预览 |
| **Outcomes（结果评估）** | 独立的 grader 评估输出，按 rubric 循环直到通过，可提升最多 10 分 | 公开 Beta |
| **Multiagent Orchestration** | 主 Agent 拆分任务委托给多个 specialist，并行协作 | 公开 Beta |
| **Webhooks** | 定义 outcome 后自动运行，完成时通知，无需人类监控 | 公开 Beta |
| **自托管 Sandbox** | 在用户 VPC 内运行，代码和文件不离开企业边界 | 公开 Beta |
| **MCP Tunnel** | 连接私有网络内的 MCP 服务器，无需暴露公网端口 | 研究预览 |

---

## 趋势六：Harness 设计原则——假设会过时，接口要稳定

### 核心哲学

> "Harness 编码了对模型不会做什么的假设，但这些假设需要经常被质疑，因为它们会随着模型进步而过时。"

### 实例：从 Opus 4.5 到 Opus 4.6 的 Harness 简化

| 组件 | Opus 4.5（需要） | Opus 4.6（简化） | 原因 |
|------|------------------|------------------|------|
| Context Reset | 必需 | 删除 | 模型不再有 context anxiety |
| Sprint 构造 | 必需 | 删除 | 模型能原生处理长任务 |
| 逐 Sprint 评估 | 每 sprint 一次 | 改为单次终评 | 模型可靠性提升 |
| 规划器 | 用户提供详细规格 | 自动生成产品规格 | 模型规划能力增强 |

### 设计准则

1. **先从简单方案开始**：最简单的方案往往最好，只在必要时增加复杂度
2. **工具设计优先**：Agent 的性能更多取决于工具设计而非提示词
3. **命名空间化**：将工具按服务/资源分组前缀，减少 Agent 选错工具的概率
4. **评估驱动迭代**：每次新模型发布后，重新审视 harness，移除不再承重的部分
5. **保持接口稳定**：底层 harness 可以换，但 session、sandbox 等接口应保持稳定

---

## 总结：2026 年 Agent Harness 六大趋势

| 趋势 | 描述 | 关键文档 |
|------|------|----------|
| 1. 架构解耦 | Brain (模型+harness) / Hands (沙箱) / Session (日志) 三层分离，各自独立演进 | [Scaling Managed Agents](https://www.anthropic.com/engineering/managed-agents) |
| 2. 生成器-评估器模式 | 独立的评估 Agent 解决模型自我评价偏差 | [Harness design for long-running apps](https://www.anthropic.com/engineering/harness-design-long-running-apps) |
| 3. 评估工程化 | 从"靠感觉调试"转向 20-50 个 eval 任务起步，三种评分器组合 | [Demystifying evals](https://www.anthropic.com/engineering/demystifying-evals-for-ai-agents) |
| 4. 多 Agent 团队 | 多个 Agent 并行协作，各自承担不同角色，独立完成复杂项目 | [Building a C compiler](https://www.anthropic.com/engineering/building-c-compiler) |
| 5. 产品化路径 | Claude Code → Agent SDK → Managed Agents，封装复杂性为托管服务 | [Building with Claude Managed Agents](https://claude.com/blog/building-with-claude-managed-agents) |
| 6. 假设检验 | 持续质疑 harness 对模型能力的假设，随着模型进步简化设计 | [Effective harnesses](https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents) |

---

## 附录：其他相关文章

| 文章 | 核心内容 |
|------|----------|
| [Building Effective AI Agents](https://www.anthropic.com/engineering/building-effective-agents) | 从简单模式到复杂 Agent 的渐进式构建原则 |
| [Writing effective tools for AI agents](https://www.anthropic.com/engineering/writing-tools-for-agents) | 工具命名空间化、响应优化、评估驱动改进 |
| [Quantifying infrastructure noise](https://www.anthropic.com/engineering/infrastructure-noise) | 资源配给可导致 6% 评分差异，需标准化报告 |
| [A harness for every task](https://claude.com/blog/a-harness-for-every-task-dynamic-workflows-in-claude-code) | Claude Code 动态工作流，model 可自写 harness |