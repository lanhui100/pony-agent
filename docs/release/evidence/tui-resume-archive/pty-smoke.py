#!/usr/bin/env python3
"""TUI /resume + 归档 PTY 冒烟：真实进程、真实按键、真实终端输出（只读观测）。"""
import os, pty, select, sys, time, re, signal

BIN = "/home/dm/pony-agent/target-test/debug/pony"
HOME = "/tmp/pony-tui-smoke-home"

env = dict(os.environ)
env.update({
    "HOME": HOME,
    "XDG_DATA_HOME": HOME + "/.local/share",
    "XDG_CONFIG_HOME": HOME + "/.config",
    "XDG_CACHE_HOME": HOME + "/.cache",
    "TERM": "xterm-256color",
    "COLUMNS": "100",
    "LINES": "30",
    "PONYLLM_API_KEY": "smoke-dummy-key",
})

os.makedirs(HOME, exist_ok=True)
os.makedirs(env["XDG_DATA_HOME"], exist_ok=True)
os.makedirs(env["XDG_CONFIG_HOME"], exist_ok=True)

pid, fd = pty.fork()
if pid == 0:  # child
    os.execve(BIN, [BIN], env)

import fcntl, struct, termios
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))

buf = bytearray()
def pump(seconds):
    end = time.time() + seconds
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.2)
        if fd in r:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                return
            if not chunk:
                return
            buf.extend(chunk)

def send(text):
    os.write(fd, text.encode())

pump(4.0)
send("/resume"); pump(0.6); send("\r"); pump(2.5)
screen_resume = bytes(buf)
send("a"); pump(1.5)
screen_archived = bytes(buf)
send("\x1b"); pump(0.8)
send("/quit"); pump(0.4); send("\r"); pump(2.5)

try:
    os.kill(pid, signal.SIGKILL)
except ProcessLookupError:
    pass
os.waitpid(pid, os.WNOHANG)

raw = bytes(buf).decode("utf-8", "replace")
# 去 ANSI 控制序列，保留可见文本
plain = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b\][^\x07]*\x07|\x1b[=>]", "", raw)

def has(*needles):
    return {n: (n in plain) for n in needles}

print("BYTES:", len(raw))
for k, v in has("暂无历史会话", "当前工作区", "全部", "共", "会话", "↑↓", "已隐藏归档会话", "panicked", "会话不存在").items():
    print(f"  {k}: {v}")
print("QUIT_OK:", not re.search(r"panicked", plain))
open("/tmp/tui-smoke-plain.txt", "w").write(plain)
print("SCREEN_TAIL:")
print(plain[-1200:])
