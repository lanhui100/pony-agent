# wave-40 绿相收据（Shift+Enter 换行 · L2-E）

- 角色：Executor
- 任务板：`task-14`
- 契约：`.dev-team/contract-matrix-tui-enter-key.md`（含 §5 勘误）
- 结论：**绿相全通 —— 单元 4/4 passed、PTY 4/4 PASS、编译 Exit 0、回归无新增失败**

## 1. 门禁实测结果

### 1.1 单元测试
```
$ cargo test -p pony-agent-core --test tui_enter_key_wave40
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
EXIT=0
```

### 1.2 PTY 真实集成测试
```
$ python3 tools/tui-audit/check_shift_enter.py --bin target/debug/pony
RESULT scenario=shift-enter pass=4 fail=0 exit=0
PASS a2.1_probe_answered_and_flags_pushed :: probe_seen=True push_seen=True push_flags=1 raw_push=1b5b3e3175
PASS a2.2_shift_enter_inserts_newline_without_submit :: composer_row='   › abc' submit_markers=[] prompt_row_before=31 prompt_row_after=30 composer_grew(second visual line)=True
PASS a2.3_plain_enter_submits_and_clears_composer :: submitted=True composer_row='   › 写下任务，或输入 / 查看命令'
PASS a2.4_ctrl_c_exits_without_panic_and_restores_terminal :: exited=True child_exit=0 panic=False leave_alt_screen=True pop_seen=True pop_raw=1b5b3c3175 pop_required=True
EXIT=0
```

### 1.3 静态类型与警告检查
```
$ RUSTFLAGS="-D warnings" cargo check -p pony-agent-core --bin pony
    Finished `dev` profile [unoptimized] target(s) in 0.28s
EXIT=0
```

### 1.4 回归用例比对
- `tui_dialog_wave39`: 12 passed / 0 failed (EXIT 0)
- `tui_usage_wave39_delta`: 4 passed / 0 failed (EXIT 0)
- `wave38-tui-workspace-contract`: 15 passed / 0 failed (EXIT 0)
- `tui_acceptance`: 34 passed / 1 failed (与基线一致)
- `tui_render_layout`: 3 passed / 5 failed (与基线一致)
