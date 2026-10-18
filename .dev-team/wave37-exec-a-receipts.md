# Wave-37 Executor A 物理收据（task-3）

- 执行者：w37-exec-a
- 时间：2026-10-18 16:35 (Asia/Shanghai)
- 写域：仅 `crates/pony-agent-core/src/agent/tools.rs`（git diff --stat：1 file, +482/-18）

## 1. cargo check -p pony-agent-core --lib

```
$ cargo check -p pony-agent-core --lib
    Checking pony-agent-core v0.1.125 (/home/dm/pony-agent/crates/pony-agent-core)
    Finished `dev` profile [unoptimized + target] in 2m 36s
EXIT=0
```

（此前一次运行曾出现 control_plane/mod.rs、runtime/mod.rs、governed_executor.rs 等 10 个编译错误，
均来自其他成员在飞变更，与本任务写域无关，后续运行已由相应成员收敛；当前 HEAD 运行 EXIT=0，
tools.rs 无任何 error/warning 归属。唯一 warning 为 governed_executor.rs `decide` dead_code，
非本任务文件。）

## 2. cargo test -p pony-agent-core --test wave37_tool_surface

```
running 9 tests
test t3_0_subagent_without_engine_returns_capability_unavailable ... ok
test t3_h_subagent_with_stub_engine_returns_real_output ... ok
test t3_i_failing_stub_returns_structured_error ... ok
test t3_l_timeout_returns_timeout_error ... ok
test t3_k_oversized_prompt_rejected ... ok
test t2b_write_file_with_env_allowance_overwrites ... ok
test t2_write_file_on_existing_file_without_expected_hash_fails_closed ... ok
test t3_j_concurrency_over_limit_returns_busy ... ok
test t1_model_visible_surface_is_exact_28_tool_list ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
EXIT=0
```

- t1（28 项精确清单 + kind/exposure/priority/permission facts）：转绿
- t2/t2b（C2 write_file 观察门 + 豁免路径 + fail-closed 内容不变）：转绿
- t3_*（C3 子代理引擎契约，task-4 交付后转绿）：绿

## 3. wave-36 回归核查

```
$ cargo test -p pony-agent-core --test wave36_adversarial_classification
test result: FAILED. 7 passed; 1 failed
```

- 失败用例：`t07_schemeless_host_port_does_not_drive_classification`（retry 分类器，wave-36 在飞变更相关）
- 基线对照：`git stash push` 暂存本任务 tools.rs diff 后重跑，**同样失败（7 passed; 1 failed）**，
  证明该失败与 task-3 改动无因果。已 `git stash pop` 还原。此失败归 Lead 裁定（wave-36 在飞变更）。