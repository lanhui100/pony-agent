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
# 仅依赖 POSIX sh + sed + sleep；不读环境变量（参数全部走 argv，避免宿主 env 白名单影响）。
set -u

mode="${1:-ok}"
arg="${2:-}"

emit() { printf '%s\n' "$1"; }

# 顶层请求 id（测试侧为 serde_json 紧凑序列化，格式确定："id":<number>）
id_of() { printf '%s' "$1" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p'; }
method_of() { printf '%s' "$1" | sed -n 's/.*"method":"\([^"]*\)".*/\1/p'; }
sentinel_of() { printf '%s' "$1" | sed -n 's/.*"sentinel":"\([^"]*\)".*/\1/p'; }

respond_initialize() {
    emit "{\"jsonrpc\":\"2.0\",\"id\":$1,\"result\":{\"protocolVersion\":1,\"agentCapabilities\":{\"loadSession\":false},\"authMethods\":[]}}"
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
        *)
            id=$(id_of "$line")
            [ -n "$id" ] && respond_initialize "$id"
            ;;
    esac
done
