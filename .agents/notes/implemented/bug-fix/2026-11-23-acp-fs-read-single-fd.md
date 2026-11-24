# Agent Note: ACP 文本读取改为「一次 open，全程同一 fd」，关掉 TOCTOU 窗口（B177 ⑦）

Status: implemented

## Problem

`acp/handlers.rs` 的 `read_text_file` 对同一个目标路径做了**三段独立**操作：

1. `classify()` → `canonicalize` 得到中间路径；
2. `std::fs::metadata(&canonical)` 判 `is_file()`、读 `len()`（决定是否超 `ACP_FS_READ_MAX_BYTES`）；
3. `read_slice()` 内部**又一次** `std::fs::File::open(path)`。

第 2 步和第 3 步之间，路径可以被替换（换文件、改符号链接目标）。后果是：`is_file` 与大小上限
判定的是**旧版本**文件，真正读出来的却是**新版本**——"检查的那个"和"读的那个"不是同一个 inode。
这正是经典的 canonicalize → metadata → open TOCTOU。

## Decision

**只打开一次，后续全部基于同一个文件句柄**：

- `read_text_file` 里 `File::open(&canonical)` 一次拿到 fd；`NotFound` 在这里归为 `NotFound`，其余归 `Internal`；
- 用 **`file.metadata()`**（fd 上的元数据）做 `is_file()` 与大小上限判定，不再 `std::fs::metadata(path)`；
- `read_slice` 的签名从 `(path: &Path, …)` 改成 `(file: std::fs::File, …)`，内部直接 `BufReader::new(file)`，
  **绝不再按路径重开**。

行为面上：错误归类与文案保持（`NotFound` / `读取文件失败` / `读取文件元数据失败`），
分片语义、上限判定、非 UTF-8 归类（ADR `2026-11-23-acp-read-slice-utf8-classification`）全部不变。

## Verification

- `bash scripts/cargo-lane.sh check -p pony-agent-core --lib` → Finished（exit 0）
- `bash scripts/cargo-lane.sh test -p pony-agent-core --test acp_capabilities_contract`
  → **11 passed / 0 failed**（含 `g6_fs_handler_fails_closed_outside_workspace`、
  `g5b_fs_handler_reads_authorized_external_path`、`g7_terminal_handler_full_chain_and_no_residue`）
- **结构性不变量（可 grep 复查）**：`grep -n "File::open\|std::fs::metadata" acp/handlers.rs`
  → 只剩 `let file = std::fs::File::open(&canonical)` 一处，`std::fs::metadata` **零出现**

## 为什么没有"红相测试"

要真正复现这个竞态，需要在 metadata 与 open 之间精确替换文件——那是靠 sleep/线程抢跑才能触发的
flaky 测试，本仓已因同类时序断言在 Windows CI 上连红四次（见
`bug-fix/2026-11-23-turn-trace-traceid-semantics.md`）。因此本条以**不变量**作为判据：
读取路径上只允许出现一次 `open`，这一点可以 grep 死守（后续任何人再引入 `metadata(path)` 都肉眼可见）。

## Alternatives considered

- **保留 `metadata(path)` 只把 open 提前**：仍有两个路径操作，窗口只是变窄不是消失。
- **Linux 专属 `O_NOFOLLOW` + `openat`**：防符号链接替换更彻底，但 Windows 无对应原语（本仓是跨平台桌面
  应用），且当前 `classify()` 已做 workspace 边界与 canonical 校验；先用"单 fd"这个跨平台不变量收口。
- **加一条并发替换文件的测试**：见上，抢跑型 flaky，收益不抵风险。
