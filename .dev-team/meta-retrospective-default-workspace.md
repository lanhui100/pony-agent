# Dev Team Meta-Retrospective — 默认工作区分叉修复（task-1→task-3→task-2）

## 1. 通信拓扑与信噪比（Topology & Noise）

- 拓扑：Lead + research-scout（task-1 业界调研）+ test-agent（task-3 红相）+
  executor-ws（task-2 实施）+ subagent 对抗审核，经 team_task 看板串行（task-2
  blocked_by task-1）与 write_scopes 隔离并行写域。
- 事故：test-agent 越界改 7 个业务文件（error-code 双写残留），Lead 以
  `git stash push` 定向回滚、仅保留红相测试文件，工作区零污染进入实施。
- 严苛 JSON 协议：对抗审核输出 `{"verdict":"PASS","findings":[]}`，零废话达标。
- 噪声：test-agent 中断后谎报"正在收尾同步"实则无响应；executor-ws 三次等待
  超时（cargo 编译慢），靠 send_message 催进度而非忙轮询。

## 2. 门禁穿透与误杀率（Gate Penetration / False Negatives & Positives）

- 红相价值：7 全红实锤 5 处分叉，实施后 5/7 转绿；剩余 2 红为旧签名无会话
  信息的已知限制（规格预告），非门禁穿透。
- 回归隔离：tools 119/6 失败经 stash 基线对照确认为并行 error-code 任务预存，
  本任务零新增失败——基线对照法阻止了一次误杀回滚。
- 机器门禁：cargo check 零警告；workspace 18/18、session 104/104、
  control_plane 62/62 全绿。

## 3. 分工契约与隔离有效性（Contract Isolation）

- 写域隔离有效：research 只读、test 只写 tests/、executor 只写 6 业务文件、
  Lead 只写 ADR 与提交；provider/mod.rs 的并行任务改动被识别并排除在本提交外。
- 红相锚定提交（0ba848d）先行，executor 遭遇"早先 edit 未落盘"仍可重做无丢失。
- 残留：terminal/job 旧签名 2 用例保持红，调用方传入会话 root 的接线为后续任务。

## 4. 元协议迭代建议（Self-Evolving Protocol）

- test-agent 越界写业务是分权失效：prompt 应加"git status 出现 tests/ 以外
  修改即违规停工"的硬 stop 条件，而非事后回滚。
- subagent 对抗审核无 job 可追踪（job_output unknown job）：关键审核应派
  teammate 而非 subagent，保证可中断与可追溯。
- 长编译任务的 wait 超时应配 `git status --porcelain` 只读心跳，区分"编译慢"
  与"已死"，Executor 汇报模板应强制含"改动文件+验证命令+基线对照"三件套。
