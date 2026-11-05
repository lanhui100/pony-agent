#!/usr/bin/env python3
"""pony-plugin-v1 echo fixture（进程外插件宿主的端到端夹具）。

用途：在不引入任何第三方依赖的前提下，演示并验证 `plugin_host` 的完整生命周期：
spawn → initialize（apiVersion + 能力协商）→ tools/list（注册）→ tools/call → shutdown。

协议：stdio 上按行分隔的 JSON-RPC 2.0（与 MCP 同构）。
- 请求：``{"jsonrpc":"2.0","id":N,"method":"...","params":{...}}``
- 响应：``{"jsonrpc":"2.0","id":N,"result":{...}}`` / ``{"jsonrpc":"2.0","id":N,"error":{...}}``
- 通知（无 id）：只执行、不响应。

暴露的工具（见同目录 plugin.json）：
- ``echo``  → 原样回显入参：``{"echo": <arguments>}``
- ``env``   → 回显子进程完整环境：``{"env": {KEY: VALUE, ...}}``
                （验收用它逐键断言"env 集合精确等于白名单"，多一个即视为泄漏）
- ``sleep`` → 睡 ``arguments.ms`` 毫秒后返回：``{"sleptMs": N}``
                （验收用它触发调用超时，断言宿主 kill 后无残留子进程）
- ``crash`` → 立即 ``os._exit(3)`` 自杀
                （验收用它断言崩溃 fail-closed：返回 plugin_unavailable 且不重试）

跨平台：
- macOS / Linux：``chmod +x echo-plugin.py`` 后可直接作为 ``command`` 启动（shebang 生效）。
- Windows：**跳过**。Windows 无 shebang 执行语义，且本 fixture 依赖 POSIX 进程组语义
  （宿主侧 Windows 无进程组，孙进程只能"尽力回收"）。Windows 宿主路径由
  ``tests/plugin_host_contract.rs`` 用 ``#[cfg(windows)]`` 标注跳过，不做伪装覆盖。
"""

import json
import os
import sys
import time

API_VERSION = "pony-plugin-v1"
PLUGIN_NAME = "echo-fixture"
PLUGIN_VERSION = "0.1.0"

TOOLS = [
    {
        "name": "echo",
        "description": "Echo back the arguments.",
        "inputSchema": {"type": "object"},
    },
    {
        "name": "env",
        "description": "Return the child process environment.",
        "inputSchema": {"type": "object"},
    },
    {
        "name": "sleep",
        "description": "Sleep for arguments.ms milliseconds.",
        "inputSchema": {
            "type": "object",
            "properties": {"ms": {"type": "integer"}},
        },
    },
    {
        "name": "crash",
        "description": "Terminate the plugin process immediately.",
        "inputSchema": {"type": "object"},
    },
]


def respond(message_id, result):
    sys.stdout.write(
        json.dumps({"jsonrpc": "2.0", "id": message_id, "result": result}) + "\n"
    )
    sys.stdout.flush()


def respond_error(message_id, code, message):
    sys.stdout.write(
        json.dumps(
            {
                "jsonrpc": "2.0",
                "id": message_id,
                "error": {"code": code, "message": message},
            }
        )
        + "\n"
    )
    sys.stdout.flush()


def call_tool(params):
    name = params.get("name")
    arguments = params.get("arguments") or {}
    if name == "echo":
        return {"echo": arguments}
    if name == "env":
        return {"env": dict(os.environ)}
    if name == "sleep":
        milliseconds = int(arguments.get("ms", 0))
        time.sleep(max(milliseconds, 0) / 1000.0)
        return {"sleptMs": milliseconds}
    if name == "crash":
        # 自杀：宿主必须 fail-closed（plugin_unavailable）且不重试。
        sys.stdout.flush()
        os._exit(3)
    raise KeyError(name)


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError as error:
            respond_error(None, -32700, "parse error: {0}".format(error))
            continue

        method = message.get("method")
        message_id = message.get("id")
        params = message.get("params") or {}

        # 通知（无 id）：执行后不响应。
        if message_id is None:
            continue

        if method == "initialize":
            requested = params.get("apiVersion")
            if requested != API_VERSION:
                respond_error(
                    message_id,
                    -32602,
                    "unsupported apiVersion: {0}".format(requested),
                )
                continue
            respond(
                message_id,
                {
                    "apiVersion": API_VERSION,
                    "capabilities": ["tools"],
                    "pluginInfo": {"name": PLUGIN_NAME, "version": PLUGIN_VERSION},
                },
            )
        elif method == "tools/list":
            respond(message_id, {"tools": TOOLS})
        elif method == "tools/call":
            try:
                respond(message_id, call_tool(params))
            except KeyError:
                respond_error(message_id, -32601, "unknown tool")
        elif method == "shutdown":
            respond(message_id, {"ok": True})
            sys.stdout.flush()
            return
        else:
            respond_error(message_id, -32601, "method not found: {0}".format(method))

    return 0


if __name__ == "__main__":
    sys.exit(main())
