#!/bin/sh
# ACP fake agent（stdio newline-delimited JSON-RPC 2.0）—— P0 传输层契约测试夹具。
#
# 用法：sh acp_fake_agent.sh <mode> [arg]
#   ok               逐请求应答（initialize → protocolVersion 1）
#   stderr           先向 stderr 写 96 行（> 32 行上限）再应答，用于验证环形缓冲有界且丢最旧
#   malformed_single 先输出 1 行非法 JSON 再正常应答（应被跳过、不致命）
#   malformed_streak 连续输出 <arg> 行非法 JSON 再应答（应达阈值 fail-closed）
#   timeout          延迟 <arg> 秒后应答（> request deadline → 客户端必须超时）
#   crash            立即以非零码退出（客户端必须返回错误而非 panic）
#   permission       向 client 发 session/request_permission（有 id），记录 client 应答后回 initialize
#   notification     向 client 发 session/update 通知（无 id）后回 initialize
#   capture          把收到的 session/cancel 原始行追加写入 <arg> 指定的文件
#   concurrent       回 {"echo": <params.sentinel>}，验证并发请求不串包
#
# P1（会话驱动）模式：
#   init_capture <file> <1|2> [sessionNewFile]
#                                 记录 initialize 原始请求行，按版本回 1 或 2；
#                                 若给第 4 个参数，则把 session/new 原始行追加到该文件（B174 mcpServers 证据）
#   flow <stopReason> <sessionId>  完整会话流：initialize → session/new → prompt 回指定 stopReason
#   updates <sessionId>            prompt 期间发 agent_message_chunk / agent_thought_chunk / tool_call
#   permission_prompt <file> <sessionId>
#                                  prompt 期间发 session/request_permission，把 client 的应答原始行
#                                  追加到 <file>，随后回 end_turn（P1-4 fail-closed）
#   slow_cancel <markerFile> <delaySeconds> <sessionId>
#                                  后台 sleep 模拟长任务；期间若收到 session/cancel 立即落证据
#                                  （<markerFile>.cancel）并回 cancelled（P1-11 工具级取消联动）
#   slow_prompt <delaySeconds> <sessionId>
#                                  initialize/session/new 即时，session/prompt 延迟指定秒数后回 end_turn
#                                  （P1-9 整轮预算用例：延迟 > ACP_REQUEST_TIMEOUT）
#   probe_fs <initFile> <respFile> <pathsFile>
#                                 记录 initialize 原始行；prompt 期间对 pathsFile 每行（JSON 路径字面量）
#                                 发 fs/read_text_file + fs/write_text_file 各一次，client 原始应答行
#                                 按序落 respFile（B175 fs 能力用例）
#   probe_terminal <initFile> <respFile> <markerFile>
#                                 markerFile 内一行是唯一 marker；仅把它拼进**终端子进程**的命令行
#                                 （fake agent 自身 argv 只含文件路径，避免 pgrep 自命中）
#                                 记录 initialize；prompt 期间发 terminal/create(echo hi + marker)
#                                 →output→wait_for_exit→kill→kill(幂等)→release，应答按序落 respFile
#   probe_terminal_wait_first <initFile> <respFile> <markerFile>
#                                 create(sleep 0.3; echo <marker>) → **wait_for_exit → output**（ACP 标准序）
#   env_dump <outFile>             initialize 时把子进程实收 env 全量写 outFile（B183 代理合并顺序）
#   session_list <initFile> <opencode|agy>
#                                 记录全部请求行；session/list 回 2 条（opencode，含 nextCursor）或空列表（agy）
#   session_load <initFile> <without-id|with-id>
#                                 记录全部请求行；session/load 回「无 sessionId」或「含 sessionId+models+modes」
#   session_close <initFile>      记录全部请求行；session/close 回 {}
#   session_delete_unsupported <initFile>
#                                 记录全部请求行；session/delete 回 -32601（能力降级）
#   emit_updates <initFile> <updatesFile>
#                                  记录 initialize；prompt 期间把 updatesFile 每行（一个 update JSON 对象）
#                                  作为 session/update 通知依次发出，最后回 end_turn（B166 变体/兜底/usage）
#   init_delay <delaySeconds>       initialize 内延迟再应答（B178 握手预算 i1）
#   env_capture <outFile>          initialize 时把实收 XDG_CONFIG_HOME/TERM/LANG/PATH 写 outFile（i2）
#   init_error <code> <message>    initialize 回 JSON-RPC error 帧（i3 错误归因）
#   probe_terminal_flood <initFile> <respFile> <markerFile> <polls>
#                                 create(`yes <marker>` + outputByteLimit=1MiB) → 期间轮询 output <polls> 次
#                                 （无界累积向量）→ 末次 output / kill / release 应答落 resp
#   probe_terminal_long <initFile> <respFile> <delaySeconds>
#                                 create(sleep <delay>; echo done) → wait_for_exit → output → output(无害重试)
#   cancel_hold <markerFile> <sessionId>
#                                  prompt 期间写 marker 文件并挂起，收到 session/cancel 通知后
#                                  才回 {"stopReason":"cancelled"}（P1-5 取消链路）
#
# 仅依赖 POSIX sh + sed + sleep；不读环境变量（参数全部走 argv，避免宿主 env 白名单影响）。
set -u

mode="${1:-ok}"
arg="${2:-}"
arg2="${3:-}"
arg3="${4:-}"
arg4="${5:-}"

emit() { printf '%s\n' "$1"; }

# 顶层请求 id（测试侧为 serde_json 紧凑序列化，格式确定："id":<number>）
id_of() { printf '%s' "$1" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p'; }
method_of() { printf '%s' "$1" | sed -n 's/.*"method":"\([^"]*\)".*/\1/p'; }
sentinel_of() { printf '%s' "$1" | sed -n 's/.*"sentinel":"\([^"]*\)".*/\1/p'; }

respond_initialize() {
    emit "{\"jsonrpc\":\"2.0\",\"id\":$1,\"result\":{\"protocolVersion\":1,\"agentCapabilities\":{\"loadSession\":false},\"authMethods\":[]}}"
}

respond_session_new() {
    emit "{\"jsonrpc\":\"2.0\",\"id\":$1,\"result\":{\"sessionId\":\"$2\"}}"
}

respond_stop() {
    emit "{\"jsonrpc\":\"2.0\",\"id\":$1,\"result\":{\"stopReason\":\"$2\"}}"
}

respond_permission() {
    emit '{"jsonrpc":"2.0","id":9001,"method":"session/request_permission","params":{"sessionId":"acp-sess-1","toolCall":{"toolCallId":"tc-1","title":"edit a.rs","kind":"edit"},"options":[{"optionId":"allow-once","name":"Allow once","kind":"allow_once"},{"optionId":"reject-once","name":"Reject once","kind":"reject_once"}]}}'
}

case "$mode" in
    crash) exit 3 ;;
esac

while IFS= read -r line; do
    [ -z "$line" ] && continue
    case "$mode" in
        timeout)
            sleep "$arg"
            id=$(id_of "$line")
            [ -n "$id" ] && respond_initialize "$id"
            ;;
        stderr)
            i=0
            while [ "$i" -lt 96 ]; do
                printf 'stderr-line-%02d\n' "$i" >&2
                i=$((i + 1))
            done
            id=$(id_of "$line")
            [ -n "$id" ] && respond_initialize "$id"
            ;;
        malformed_single)
            emit 'this-is-not-json'
            id=$(id_of "$line")
            [ -n "$id" ] && respond_initialize "$id"
            ;;
        malformed_streak)
            i=0
            while [ "$i" -lt "$arg" ]; do
                emit '<<malformed-line>>'
                i=$((i + 1))
            done
            id=$(id_of "$line")
            [ -n "$id" ] && respond_initialize "$id"
            ;;
        permission)
            id=$(id_of "$line")
            emit '{"jsonrpc":"2.0","id":9001,"method":"session/request_permission","params":{"sessionId":"s-1","toolCall":{"toolCallId":"tc-1","title":"edit a.rs"},"options":[{"optionId":"allow-once","name":"Allow once","kind":"allow_once"},{"optionId":"reject-once","name":"Reject","kind":"reject_once"}]}}'
            if IFS= read -r answer; then
                outcome="other"
                case "$answer" in
                    *'"cancelled"'*) outcome="cancelled" ;;
                    *'"selected"'*) outcome="selected" ;;
                    *'"error"'*) outcome="error" ;;
                esac
            else
                outcome="eof"
            fi
            [ -n "$id" ] && emit "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"protocolVersion\":1,\"agentCapabilities\":{\"loadSession\":false},\"authMethods\":[],\"recordedOutcome\":\"$outcome\"}}"
            ;;
        notification)
            id=$(id_of "$line")
            emit '{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hello"}}}}'
            [ -n "$id" ] && respond_initialize "$id"
            ;;
        capture)
            m=$(method_of "$line")
            if [ "$m" = "session/cancel" ]; then
                printf '%s\n' "$line" >> "$arg"
            else
                id=$(id_of "$line")
                [ -n "$id" ] && respond_initialize "$id"
            fi
            ;;
        concurrent)
            id=$(id_of "$line")
            s=$(sentinel_of "$line")
            [ -n "$id" ] && emit "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"echo\":\"$s\"}}"
            ;;
        env_dump)
            m=$(method_of "$line")
            id=$(id_of "$line")
            case "$m" in
                initialize)
                    env > "$arg"
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
                session/new)
                    [ -n "$id" ] && respond_session_new "$id" "acp-sess-1"
                    ;;
                session/prompt)
                    [ -n "$id" ] && respond_stop "$id" end_turn
                    ;;
                *)
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
            esac
            ;;
        session_list)
            printf '%s\n' "$line" >> "$arg"
            m=$(method_of "$line")
            id=$(id_of "$line")
            case "$m" in
                initialize)
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
                session/list)
                    if [ "${arg2:-opencode}" = "agy" ]; then
                        emit "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"sessions\":[]}}"
                    else
                        emit "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"sessions\":[{\"sessionId\":\"sess-a\",\"cwd\":\"/tmp/ws\",\"title\":\"A\",\"updatedAt\":\"2026-11-15T00:00:00Z\"},{\"sessionId\":\"sess-b\",\"cwd\":\"/tmp/ws\",\"title\":\"B\"}],\"nextCursor\":\"cur-2\"}}"
                    fi
                    ;;
                session/new)
                    [ -n "$id" ] && respond_session_new "$id" "acp-sess-1"
                    ;;
                *)
                    [ -n "$id" ] && emit "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{}}"
                    ;;
            esac
            ;;
        session_load)
            printf '%s\n' "$line" >> "$arg"
            m=$(method_of "$line")
            id=$(id_of "$line")
            case "$m" in
                initialize)
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
                session/load)
                    if [ "${arg2:-without-id}" = "with-id" ]; then
                        emit "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"sessionId\":\"sess-loaded\",\"models\":{\"currentModelId\":\"m-1\"},\"modes\":{\"currentModeId\":\"code\",\"availableModes\":[{\"id\":\"code\",\"name\":\"Code\"}]}}}"
                    else
                        emit "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"configOptions\":[{\"type\":\"boolean\",\"id\":\"fast\",\"name\":\"Fast\",\"currentValue\":true}],\"modes\":{\"currentModeId\":\"code\",\"availableModes\":[{\"id\":\"code\",\"name\":\"Code\"}]}}}"
                    fi
                    ;;
                session/new)
                    [ -n "$id" ] && respond_session_new "$id" "acp-sess-1"
                    ;;
                *)
                    [ -n "$id" ] && emit "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{}}"
                    ;;
            esac
            ;;
        session_close)
            printf '%s\n' "$line" >> "$arg"
            m=$(method_of "$line")
            id=$(id_of "$line")
            case "$m" in
                initialize)
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
                session/close)
                    [ -n "$id" ] && emit "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{}}"
                    ;;
                session/new)
                    [ -n "$id" ] && respond_session_new "$id" "acp-sess-1"
                    ;;
                *)
                    [ -n "$id" ] && emit "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{}}"
                    ;;
            esac
            ;;
        session_delete_unsupported)
            printf '%s\n' "$line" >> "$arg"
            m=$(method_of "$line")
            id=$(id_of "$line")
            case "$m" in
                initialize)
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
                session/delete)
                    [ -n "$id" ] && emit "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32601,\"message\":\"Method not found\"}}"
                    ;;
                session/new)
                    [ -n "$id" ] && respond_session_new "$id" "acp-sess-1"
                    ;;
                *)
                    [ -n "$id" ] && emit "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{}}"
                    ;;
            esac
            ;;
        emit_updates)
            m=$(method_of "$line")
            id=$(id_of "$line")
            case "$m" in
                initialize)
                    printf '%s\n' "$line" >> "$arg"
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
                session/new)
                    [ -n "$id" ] && respond_session_new "$id" "acp-sess-1"
                    ;;
                session/prompt)
                    prompt_id="$id"
                    while IFS= read -r update_json <&3; do
                        [ -z "$update_json" ] && continue
                        emit "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"acp-sess-1\",\"update\":$update_json}}"
                    done 3< "$arg2"
                    [ -n "$prompt_id" ] && respond_stop "$prompt_id" end_turn
                    ;;
                *)
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
            esac
            ;;
        init_delay)
            m=$(method_of "$line")
            id=$(id_of "$line")
            case "$m" in
                initialize)
                    sleep "$arg"
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
                session/new)
                    [ -n "$id" ] && respond_session_new "$id" "acp-sess-1"
                    ;;
                session/prompt)
                    [ -n "$id" ] && respond_stop "$id" end_turn
                    ;;
                *)
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
            esac
            ;;
        env_capture)
            m=$(method_of "$line")
            id=$(id_of "$line")
            case "$m" in
                initialize)
                    : > "$arg"
                    for key in XDG_CONFIG_HOME TERM LANG PATH; do
                        eval "value=\${$key-__UNSET__}"
                        printf '%s=%s\n' "$key" "$value" >> "$arg"
                    done
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
                session/new)
                    [ -n "$id" ] && respond_session_new "$id" "acp-sess-1"
                    ;;
                session/prompt)
                    [ -n "$id" ] && respond_stop "$id" end_turn
                    ;;
                *)
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
            esac
            ;;
        init_error)
            m=$(method_of "$line")
            id=$(id_of "$line")
            case "$m" in
                initialize)
                    [ -n "$id" ] && emit "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":$arg,\"message\":\"$arg2\"}}"
                    ;;
                *)
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
            esac
            ;;
        probe_terminal_flood)
            m=$(method_of "$line")
            id=$(id_of "$line")
            case "$m" in
                initialize)
                    printf '%s\n' "$line" >> "$arg"
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
                session/new)
                    [ -n "$id" ] && respond_session_new "$id" "acp-sess-1"
                    ;;
                session/prompt)
                    prompt_id="$id"
                    marker_text=$(cat "$arg3" 2>/dev/null || printf '%s' "$arg3")
                    emit "{\"jsonrpc\":\"2.0\",\"id\":9500,\"method\":\"terminal/create\",\"params\":{\"sessionId\":\"acp-sess-1\",\"command\":\"sh\",\"args\":[\"-c\",\"yes $marker_text\"],\"outputByteLimit\":1048576}}"
                    IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                    tid=$(printf '%s' "$answer" | sed -n 's/.*"terminalId":"\([^"]*\)".*/\1/p')
                    idx=0
                    while [ "$idx" -lt "${arg4:-0}" ]; do
                        sleep 0.05
                        rid=$((9600 + idx))
                        emit "{\"jsonrpc\":\"2.0\",\"id\":$rid,\"method\":\"terminal/output\",\"params\":{\"sessionId\":\"acp-sess-1\",\"terminalId\":\"$tid\"}}"
                        IFS= read -r answer
                        idx=$((idx + 1))
                    done
                    emit "{\"jsonrpc\":\"2.0\",\"id\":9700,\"method\":\"terminal/output\",\"params\":{\"sessionId\":\"acp-sess-1\",\"terminalId\":\"$tid\"}}"
                    IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                    emit "{\"jsonrpc\":\"2.0\",\"id\":9701,\"method\":\"terminal/kill\",\"params\":{\"sessionId\":\"acp-sess-1\",\"terminalId\":\"$tid\"}}"
                    IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                    emit "{\"jsonrpc\":\"2.0\",\"id\":9702,\"method\":\"terminal/release\",\"params\":{\"sessionId\":\"acp-sess-1\",\"terminalId\":\"$tid\"}}"
                    IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                    [ -n "$prompt_id" ] && respond_stop "$prompt_id" end_turn
                    ;;
                *)
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
            esac
            ;;
        probe_terminal_wait_first)
            m=$(method_of "$line")
            id=$(id_of "$line")
            case "$m" in
                initialize)
                    printf '%s\n' "$line" >> "$arg"
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
                session/new)
                    [ -n "$id" ] && respond_session_new "$id" "acp-sess-1"
                    ;;
                session/prompt)
                    prompt_id="$id"
                    marker_text=$(cat "$arg3" 2>/dev/null || printf '%s' "$arg3")
                    emit "{\"jsonrpc\":\"2.0\",\"id\":9301,\"method\":\"terminal/create\",\"params\":{\"sessionId\":\"acp-sess-1\",\"command\":\"sh\",\"args\":[\"-c\",\"sleep 0.3; echo $marker_text\"]}}"
                    IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                    tid=$(printf '%s' "$answer" | sed -n 's/.*"terminalId":"\([^"]*\)".*/\1/p')
                    emit "{\"jsonrpc\":\"2.0\",\"id\":9302,\"method\":\"terminal/wait_for_exit\",\"params\":{\"sessionId\":\"acp-sess-1\",\"terminalId\":\"$tid\"}}"
                    IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                    emit "{\"jsonrpc\":\"2.0\",\"id\":9303,\"method\":\"terminal/output\",\"params\":{\"sessionId\":\"acp-sess-1\",\"terminalId\":\"$tid\"}}"
                    IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                    emit "{\"jsonrpc\":\"2.0\",\"id\":9304,\"method\":\"terminal/release\",\"params\":{\"sessionId\":\"acp-sess-1\",\"terminalId\":\"$tid\"}}"
                    IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                    [ -n "$prompt_id" ] && respond_stop "$prompt_id" end_turn
                    ;;
                *)
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
            esac
            ;;
        probe_terminal_long)
            m=$(method_of "$line")
            id=$(id_of "$line")
            case "$m" in
                initialize)
                    printf '%s\n' "$line" >> "$arg"
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
                session/new)
                    [ -n "$id" ] && respond_session_new "$id" "acp-sess-1"
                    ;;
                session/prompt)
                    prompt_id="$id"
                    emit "{\"jsonrpc\":\"2.0\",\"id\":9401,\"method\":\"terminal/create\",\"params\":{\"sessionId\":\"acp-sess-1\",\"command\":\"sh\",\"args\":[\"-c\",\"sleep $arg3; echo done\"]}}"
                    IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                    tid=$(printf '%s' "$answer" | sed -n 's/.*"terminalId":"\([^"]*\)".*/\1/p')
                    emit "{\"jsonrpc\":\"2.0\",\"id\":9402,\"method\":\"terminal/wait_for_exit\",\"params\":{\"sessionId\":\"acp-sess-1\",\"terminalId\":\"$tid\"}}"
                    IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                    emit "{\"jsonrpc\":\"2.0\",\"id\":9403,\"method\":\"terminal/output\",\"params\":{\"sessionId\":\"acp-sess-1\",\"terminalId\":\"$tid\"}}"
                    IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                    emit "{\"jsonrpc\":\"2.0\",\"id\":9404,\"method\":\"terminal/output\",\"params\":{\"sessionId\":\"acp-sess-1\",\"terminalId\":\"$tid\"}}"
                    IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                    emit "{\"jsonrpc\":\"2.0\",\"id\":9405,\"method\":\"terminal/release\",\"params\":{\"sessionId\":\"acp-sess-1\",\"terminalId\":\"$tid\"}}"
                    IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                    [ -n "$prompt_id" ] && respond_stop "$prompt_id" end_turn
                    ;;
                *)
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
            esac
            ;;
        probe_fs)
            m=$(method_of "$line")
            id=$(id_of "$line")
            case "$m" in
                initialize)
                    printf '%s\n' "$line" >> "$arg"
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
                session/new)
                    [ -n "$id" ] && respond_session_new "$id" "acp-sess-1"
                    ;;
                session/prompt)
                    prompt_id="$id"
                    idx=0
                    while IFS= read -r path_json <&3; do
                        [ -z "$path_json" ] && continue
                        rid=$((9100 + idx))
                        emit "{\"jsonrpc\":\"2.0\",\"id\":$rid,\"method\":\"fs/read_text_file\",\"params\":{\"sessionId\":\"acp-sess-1\",\"path\":$path_json}}"
                        IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                        wid=$((9500 + idx))
                        emit "{\"jsonrpc\":\"2.0\",\"id\":$wid,\"method\":\"fs/write_text_file\",\"params\":{\"sessionId\":\"acp-sess-1\",\"path\":$path_json,\"content\":\"PROBE-WRITE-$idx\"}}"
                        IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                        idx=$((idx + 1))
                    done 3< "$arg3"
                    [ -n "$prompt_id" ] && respond_stop "$prompt_id" end_turn
                    ;;
                *)
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
            esac
            ;;
        probe_terminal)
            m=$(method_of "$line")
            id=$(id_of "$line")
            case "$m" in
                initialize)
                    printf '%s\n' "$line" >> "$arg"
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
                session/new)
                    [ -n "$id" ] && respond_session_new "$id" "acp-sess-1"
                    ;;
                session/prompt)
                    prompt_id="$id"
                    marker_text=$(cat "$arg3" 2>/dev/null || printf '%s' "$arg3")
                    emit "{\"jsonrpc\":\"2.0\",\"id\":9201,\"method\":\"terminal/create\",\"params\":{\"sessionId\":\"acp-sess-1\",\"command\":\"sh\",\"args\":[\"-c\",\"echo hi; : $marker_text\"]}}"
                    IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                    tid=$(printf '%s' "$answer" | sed -n 's/.*"terminalId":"\([^"]*\)".*/\1/p')
                    emit "{\"jsonrpc\":\"2.0\",\"id\":9202,\"method\":\"terminal/output\",\"params\":{\"sessionId\":\"acp-sess-1\",\"terminalId\":\"$tid\"}}"
                    IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                    emit "{\"jsonrpc\":\"2.0\",\"id\":9203,\"method\":\"terminal/wait_for_exit\",\"params\":{\"sessionId\":\"acp-sess-1\",\"terminalId\":\"$tid\"}}"
                    IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                    emit "{\"jsonrpc\":\"2.0\",\"id\":9204,\"method\":\"terminal/kill\",\"params\":{\"sessionId\":\"acp-sess-1\",\"terminalId\":\"$tid\"}}"
                    IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                    emit "{\"jsonrpc\":\"2.0\",\"id\":9205,\"method\":\"terminal/kill\",\"params\":{\"sessionId\":\"acp-sess-1\",\"terminalId\":\"$tid\"}}"
                    IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                    emit "{\"jsonrpc\":\"2.0\",\"id\":9206,\"method\":\"terminal/release\",\"params\":{\"sessionId\":\"acp-sess-1\",\"terminalId\":\"$tid\"}}"
                    IFS= read -r answer && printf '%s\n' "$answer" >> "$arg2"
                    [ -n "$prompt_id" ] && respond_stop "$prompt_id" end_turn
                    ;;
                *)
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
            esac
            ;;
        init_capture)
            m=$(method_of "$line")
            id=$(id_of "$line")
            case "$m" in
                initialize)
                    printf '%s\n' "$line" >> "$arg"
                    if [ "${arg2:-1}" = "2" ]; then
                        [ -n "$id" ] && emit "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"protocolVersion\":2,\"agentCapabilities\":{},\"authMethods\":[]}}"
                    else
                        [ -n "$id" ] && respond_initialize "$id"
                    fi
                    ;;
                session/new)
                    [ -n "$arg3" ] && printf '%s\n' "$line" >> "$arg3"
                    [ -n "$id" ] && respond_session_new "$id" "acp-sess-1"
                    ;;
                session/prompt)
                    [ -n "$id" ] && respond_stop "$id" end_turn
                    ;;
                *)
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
            esac
            ;;
        flow|updates|permission_prompt|cancel_hold|slow_prompt|slow_cancel)
            m=$(method_of "$line")
            id=$(id_of "$line")
            case "$mode" in
                # slow_cancel：<marker> <delay> <sessionId>
                slow_cancel) session_id="${arg3:-acp-sess-1}" ;;
                *) session_id="${arg2:-acp-sess-1}" ;;
            esac
            case "$m" in
                initialize)
                    [ -n "$id" ] && respond_initialize "$id"
                    ;;
                session/new)
                    [ -n "$id" ] && respond_session_new "$id" "$session_id"
                    ;;
                session/prompt)
                    case "$mode" in
                        updates)
                            emit "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"$session_id\",\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"hello\"}}}}"
                            emit "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"$session_id\",\"update\":{\"sessionUpdate\":\"agent_thought_chunk\",\"content\":{\"type\":\"text\",\"text\":\"reasoning\"}}}}"
                            emit "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"$session_id\",\"update\":{\"sessionUpdate\":\"tool_call\",\"toolCallId\":\"tc-1\",\"title\":\"edit a.rs\",\"kind\":\"edit\",\"status\":\"pending\"}}}"
                            [ -n "$id" ] && respond_stop "$id" end_turn
                            ;;
                        permission_prompt)
                            respond_permission
                            if IFS= read -r answer; then
                                printf '%s\n' "$answer" >> "$arg"
                            fi
                            [ -n "$id" ] && respond_stop "$id" end_turn
                            ;;
                        slow_prompt)
                            # 先发一条 update（CLI jsonl 走线用例需要 turn:delta），再模拟长任务。
                            emit "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"sessionId\":\"$session_id\",\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\"hello\"}}}}"
                            sleep "$arg"
                            [ -n "$id" ] && respond_stop "$id" end_turn
                            ;;
                        slow_cancel)
                            prompt_id="$id"
                            printf '%s\n' "$id" >> "$arg"
                            # 后台 sleep 代表“agent 正在长时间执行”；主 shell 立即进入读循环，
                            # 因而 session/cancel 在睡眠期间即可被接收（取消响应性）。
                            sleep "${arg2:-5}" &
                            while IFS= read -r next; do
                                [ -z "$next" ] && continue
                                nm=$(method_of "$next")
                                if [ "$nm" = "session/cancel" ]; then
                                    printf '%s\n' "$next" >> "$arg.cancel"
                                    [ -n "$prompt_id" ] && respond_stop "$prompt_id" cancelled
                                    break
                                fi
                            done
                            ;;
                        cancel_hold)
                            printf '%s\n' "$id" >> "$arg"
                            prompt_id="$id"
                            while IFS= read -r next; do
                                [ -z "$next" ] && continue
                                nm=$(method_of "$next")
                                if [ "$nm" = "session/cancel" ]; then
                                    # 证据面：把收到的 session/cancel 原始行另存为 "<marker>.cancel"
                                    printf '%s\n' "$next" >> "$arg.cancel"
                                    [ -n "$prompt_id" ] && respond_stop "$prompt_id" cancelled
                                    break
                                fi
                            done
                            ;;
                        *)
                            [ -n "$id" ] && respond_stop "$id" "${arg:-end_turn}"
                            ;;
                    esac
                    ;;
                session/cancel)
                    :
                    ;;
                *)
                    [ -n "$id" ] && respond_stop "$id" end_turn
                    ;;
            esac
            ;;
        *)
            id=$(id_of "$line")
            [ -n "$id" ] && respond_initialize "$id"
            ;;
    esac
done
