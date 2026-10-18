#!/usr/bin/env bash
# wave36_pa139_full_lib_50x.sh — PA-139 全量并行残留 flake 的 50 轮红相/验收脚本
#
# 验收条款(task board PA-139):
#   for i in $(seq 50); do RUSTFLAGS="-D warnings" cargo test -p pony-agent-core --lib; done
#   ⇒ 50/50 `0 failed`,并留逐轮 log。
#
# 已知 flake 形态(wave-35 收口 + wave-36 Lead 实测 1/8):
#   失败用例名 = agent::runtime::tests::parity::history_after_multi_hop_tool_turn_keeps_tool_content_out_of_assistant_text
#   形态       = 首个 provider 请求 1-2ms 内 `client error (Connect)`(mock 端口 connect refused)。
#   **禁止**用「放宽 classify」或「调大 MOCK_SERVER_FINISH_TIMEOUT」掩盖;本脚本只观测,不改实现。
#
# 用法:
#   wave36_pa139_full_lib_50x.sh            # 默认 50 轮
#   W36_PA139_ROUNDS=2 wave36_pa139_full_lib_50x.sh   # 冒烟轮数(用于脚本自证,不可当验收)
#   W36_PA139_FILTER=...                    # 可选:缩小到单个用例(诊断用,不可当验收)
#
# 退出码:0 = 全部轮次 0 failed;1 = 出现失败轮次(附失败用例与 connect 证据);3 = 环境/用法错误。
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$REPO_ROOT" || { echo "FATAL: 无法进入仓库根 $REPO_ROOT"; exit 3; }

ROUNDS="${W36_PA139_ROUNDS:-50}"
OUT="${W36_PA139_OUT:-$REPO_ROOT/tests/acceptance/receipts/wave36/pa139_runs}"
FILTER="${W36_PA139_FILTER:-}"
SUMMARY="$REPO_ROOT/tests/acceptance/receipts/wave36/T-W36-139-01_full_lib_${ROUNDS}x.log"
KNOWN_FLAKE="agent::runtime::tests::parity::history_after_multi_hop_tool_turn_keeps_tool_content_out_of_assistant_text"

mkdir -p "$OUT" "$(dirname "$SUMMARY")"
{
  echo "PA-139 50-round full-lib run"
  echo "REPO: $REPO_ROOT"
  echo "START: $(date -Is)"
  echo "ROUNDS: $ROUNDS"
  echo "FILTER: ${FILTER:-<none: full lib>}"
  echo "KNOWN_FLAKE: $KNOWN_FLAKE"
  echo "-------------------------------------------------------------"
} | tee "$SUMMARY"

fail_round=0
for n in $(seq -w 1 "$ROUNDS"); do
  log="$OUT/run_$n.log"
  started="$(date +%s)"
  if [ -n "$FILTER" ]; then
    CARGO_TERM_COLOR=never cargo test -p pony-agent-core --lib -- "$FILTER" > "$log" 2>&1
  else
    CARGO_TERM_COLOR=never cargo test -p pony-agent-core --lib > "$log" 2>&1
  fi
  code=$?
  elapsed=$(( $(date +%s) - started ))

  if ! grep -qE '^test result: (ok|FAILED)\.' "$log"; then
    {
      echo "ROUND $n: INVALID — log 缺少终态汇总(run 被中断?) exit=$code elapsed=${elapsed}s"
      tail -5 "$log"
    } | tee -a "$SUMMARY"
    fail_round=$n
    break
  fi

  summary="$(grep -E '^test result:' "$log" | tail -1)"
  if [ "$code" -ne 0 ] || ! grep -qE '^test result: ok\.' "$log"; then
    {
      echo ""
      echo "ROUND $n: FAILED exit=$code elapsed=${elapsed}s"
      echo "  summary: $summary"
      echo "  failed-test names:"
      grep -E '^test .* FAILED$' "$log" | sed 's/^/    /' | head -20
      echo "  failures block (head):"
      awk '/^failures:$/{flag=1} flag{print "    "$0} flag&&/^$/{if(++blank>1)exit}' "$log" | head -30
      echo "  connect-refused evidence count: $(grep -ci 'connect' "$log")"
      echo "  known-flake-hit: $(grep -c "$KNOWN_FLAKE" "$log")"
    } | tee -a "$SUMMARY"
    fail_round=$n
    break
  fi

  echo "ROUND $n: PASS ($summary) elapsed=${elapsed}s" | tee -a "$SUMMARY"
done

{
  echo "-------------------------------------------------------------"
  echo "END: $(date -Is)"
} | tee -a "$SUMMARY"

if [ "$fail_round" -ne 0 ]; then
  echo "RESULT: FAIL — round $fail_round panicked/failed; raw logs: $OUT" | tee -a "$SUMMARY"
  exit 1
fi
echo "RESULT: PASS — $ROUNDS/$ROUNDS rounds reported 'test result: ok. 0 failed' (raw logs: $OUT)" | tee -a "$SUMMARY"
if [ "$ROUNDS" -lt 50 ]; then
  echo "NOTE: ROUNDS=$ROUNDS < 50 — 仅脚本冒烟,不构成 PA-139 验收。" | tee -a "$SUMMARY"
fi
exit 0
