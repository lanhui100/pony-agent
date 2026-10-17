# W26 审查意见处置记录（L3 复审后）

审查报告：`.dev-team/review-w26.json`（两轮：d5ad4bcd FAIL 3 major + 8 minor；897c35db 复核 FAIL 1 blocker + 4 minor）

## blocker（897c35db）

| # | 发现 | 处置 | 证据 |
|---|---|---|---|
| 1 | `tools.rs:10874` 存量单测 `read_file_rejects_file_too_large` 仍断言 `file_too_large`，新分支已改 spill(ok) → 整包 lib 红 | 该测试按新契约迁移为 `read_file_spills_oversized_file_instead_of_failing`（断言 ok+spilled+content_ref+preview），并**新增** `read_file_rejects_file_beyond_spill_cap` 用稀疏文件覆盖 >MAX_SPILL_BYTES 仍失败 | `cargo test --lib read_file_` → 10 passed / 0 failed |

## minor

| # | 发现 | 处置 | 证据 |
|---|---|---|---|
| 2 | `SpillRead` 缺 display 中文名 | 产品名映射补 `TOOL_SPILL_READ => "SpillRead"`，展示名补「回读溢出」 | `cargo test --lib contract_view_tests` 12 passed |
| 3 | `gc_spill_root` 无生产调用点、目录 775 / 文件 664 | 新增 `maybe_gc`（5 分钟限频，24h / 512MiB 阈值）在 `spill_bytes_to` 落盘后触发；Unix 下目录 0700、文件 0600 | `cargo test --lib agent::spill` 11 passed（含 gc 单测） |
| 4 | `parent` 检查恒真（死防线）；已存在文件被信任、跟随 symlink、不比 sha256 | 读前 `canonicalize` 根与目标，要求真实路径父目录==规范化根 **且** 文件名==`<ref_id>.spill`，否则 `invalid_ref`；新增 unix 符号链接替换负例 | 新增 `symlinked_spill_file_is_rejected`（symlink → invalid_ref） |
| 5 | `result.limit` 回显未 clamp 入参 | `SpillSlice` 增加 `limit`（clamp 后值），handler 回显 `slice.limit`；文档写明「按回显 limit 翻页」 | 单测 `limit_is_hard_capped_and_zero_limit_returns_empty` |

## 门禁复跑（修复后）

```
cargo test --lib agent::spill                    11 passed
cargo test --lib read_file_                      10 passed
cargo test --test wave26_spill_test               8 passed
cargo test --test wave25_capability_honesty_test  4 passed
cargo test --test orchestration_acceptance_test   4 passed
cargo test --test dispatcher_matrix              27 passed
cargo test --test tool_surface_wave20_contract    2 passed
cargo test --lib provider::tests                 52 passed
cargo test --lib contract_view_tests             12 passed
cargo check -p pony-agent-core --all-targets     Exit 0
cargo check --manifest-path src-tauri/Cargo.toml Exit 0
```

## 全量 lib 套件基线对照（重要）

`cargo test -p pony-agent-core --lib -- --test-threads=1`（并行跑会互相干扰/超时，串行 147s 稳定）：

- 基线（stash 掉 W26 改动）：1022 passed / **12 failed**
- W26 修复后：1025 passed / **11 failed**
- 失败集差异：**仅少了 `read_file_rejects_file_too_large`（本波修掉的存量陈旧测试），无新增失败**
- 11 个存量失败与本波无关（Windows 路径/卷边界、symlink canary、sandbox fail-closed、governed vs legacy 对齐、edit_file 的 `fs_not_observed`）：属基线技术债，另立 backlog，不在 W26 夹带修复。
