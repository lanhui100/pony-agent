# PA-104: 交互式持久终端 PTY 工具族 (terminal_*)

## 1. 契约定义与 Schema (Acceptance Criteria Matrix)

### 工具清单
1. `terminal_open`
   - 参数: `{ "command"?: string, "args"?: string[], "cwd"?: string, "cols"?: number, "rows"?: number, "env"?: Record<string, string> }`
   - 返回: `{ "terminal_id": string, "pid": number }`
2. `terminal_send`
   - 参数: `{ "terminal_id": string, "input": string }`
   - 返回: `{ "bytes_written": number }`
3. `terminal_read`
   - 参数: `{ "terminal_id": string, "timeout_ms"?: number, "offset"?: number }`
   - 返回: `{ "output": string, "cursor": number, "has_more": boolean, "is_alive": boolean }`
4. `terminal_signal`
   - 参数: `{ "terminal_id": string, "signal": "SIGINT" | "SIGTERM" | "SIGKILL" | "BREAK" }`
   - 返回: `{ "success": boolean }`
5. `terminal_close`
   - 参数: `{ "terminal_id": string, "force"?: boolean }`
   - 返回: `{ "closed": boolean, "exit_code"?: number }`

### 验收矩阵
- **AC-01**: `terminal_open` 成功派生跨平台 PTY，并正确返回 UUID 终端 ID 与子进程 PID。
- **AC-02**: `terminal_send` 与 `terminal_read` 能够进行全双工输入输出交互，环形缓冲区具备上限控制（2MB）。
- **AC-03**: `terminal_close` 或进程自然退出时，必须级联收割进程树，Windows 下受 Job Object 管控，POSIX 下发送进程组信号，杜绝孤儿/僵尸进程。
- **AC-04**: 路径穿越与未授权工作区检测 fail-closed。

## 2. 状态记录
- **阶段**: Phase 1 契约冻结与空桩初始化
- **负责人**: Team Lead
