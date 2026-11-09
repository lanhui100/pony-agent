# wave-68 客观事实清单（L3-P 只允许引用本清单内证据）

## 1. 机器门禁（/tmp/pa171-join.log）
CHECK_EXIT=0
test result: ok. 89 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 6.63s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.77s
test result: ok. 96 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 6.42s
test result: ok. 98 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 6.35s
test result: ok. 1400 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 221.27s

## 2. Tier-1 冒烟（/tmp/pa171-smoke.log）
BUILD_EXIT=0
VER_EXIT=0
Usage: ponycli [init] [OPTIONS] [-p <prompt>]
HELP_EXIT=0

## 3. 观测门禁
audit-diff.sh --base d9425a70 → [OK] 无越界实现、收口点完整 (exit 0)
check-declaration.sh . → 结论: PASS (exit=0)

## 4. 红相回执
见 .dev-team/pa171-red-receipt.json（4 个测试 exit=101）

## 5. NFR 基准
见 .dev-team/nfr-baseline-wave68.json（dev-nfr-lint.py Exit 0）
