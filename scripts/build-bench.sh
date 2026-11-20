#!/usr/bin/env bash
# build-bench.sh —— 构建墙钟 / 峰值内存标定（把「jobs 该给几」「sccache 值不值」从口头约定变成可重跑测量）。
#
# 为什么需要它：`max_jobs_per_lane` 与 sccache 开关此前都是拍脑袋值，没有本机墙钟数据。
# 本脚本在**既有槽位**（默认 target-check）上做单变量对照，输出可直接抄进 ADR 的收据表。
#
# 用法：
#   bash scripts/build-bench.sh --jobs 4,8,12            # jobs 标定（incremental 关，测「冷 crate 检查」）
#   bash scripts/build-bench.sh --jobs 4 --sccache 0,1   # sccache 单变量对照
#   bash scripts/build-bench.sh --jobs 4 --incremental 1 # 日常改代码路径（增量编译开着）
#   bash scripts/build-bench.sh --all-targets            # 含 15.7 万行测试的 --all-targets 检查
#
# 每次测量前 `touch` 目标文件，保证同样起点；测量走 `scripts/cargo-lane.sh`（并发预算 + flock +
# load 熔断），因此**不会**绕过本仓的构建治理。
#
# 峰值内存口径：只统计**本任务进程树**内 rustc 的 RSS 之和（共享机上其他会话/其他项目的 rustc
# 不参与），每 0.5s 采样取最大。
#
# 约束：不新建 target 目录（仓库膨胀根因，见 AGENTS.md）——默认复用 target-check 槽位。
# 退出码：0 全部轮次成功 / 1 至少一轮 cargo 失败 / 2 用法错误。
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CORE_MANIFEST="crates/pony-agent-core/Cargo.toml"
TOUCH_FILE="crates/pony-agent-core/src/lib.rs"

JOBS_LIST="4,8,12"
SCCACHE_LIST="0"
INCREMENTAL=0
TARGET_DIR="target-check"
SCOPE="lib"
LABEL=""

while [ $# -gt 0 ]; do
  case "$1" in
    --jobs) JOBS_LIST="${2:-}"; shift 2 || shift ;;
    --sccache) SCCACHE_LIST="${2:-}"; shift 2 || shift ;;
    --incremental) INCREMENTAL="${2:-0}"; shift 2 || shift ;;
    --target-dir) TARGET_DIR="${2:-target-check}"; shift 2 || shift ;;
    --touch) TOUCH_FILE="${2:-crates/pony-agent-core/src/lib.rs}"; shift 2 || shift ;;
    --all-targets) SCOPE="all"; shift ;;
    --label) LABEL="${2:-}"; shift 2 || shift ;;
    -h|--help) sed -n '2,22p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "[build-bench] 未知参数：$1" >&2; exit 2 ;;
  esac
done

# 逗号/空格分隔解析：空值必须早失败（`for x in ${EMPTY}` 会退化成语法错误）。
# min 区分语义：jobs 必须 ≥1；sccache 是开关，0 合法（= 关闭）。
parse_list() {
  local raw="$1" name="$2" min="$3" out=() item
  raw="${raw//,/ }"
  for item in $raw; do
    case "$item" in
      ''|*[!0-9]*) echo "[build-bench] $name 只接受逗号分隔的非负整数，收到：$raw" >&2; return 2 ;;
    esac
    [ "$item" -ge "$min" ] || { echo "[build-bench] $name 必须 ≥ $min，收到：$item" >&2; return 2; }
    out+=("$item")
  done
  [ "${#out[@]}" -gt 0 ] || { echo "[build-bench] $name 不能为空" >&2; return 2; }
  printf '%s\n' "${out[@]}"
}

JOBS_PARSED="$(parse_list "$JOBS_LIST" --jobs 1)" || exit 2
SCCACHE_PARSED="$(parse_list "$SCCACHE_LIST" --sccache 0)" || exit 2

case "$TARGET_DIR" in
  target|target-check|target-test|target-perf) ;;
  *) echo "[build-bench] 拒绝：--target-dir 只接受既有槽位（target/target-check/target-test/target-perf），收到 $TARGET_DIR" >&2; exit 2 ;;
esac

command -v python3 >/dev/null 2>&1 || { echo "[build-bench] 需要 python3 做表格排版" >&2; exit 2; }

# 本任务进程树里 rustc 的 RSS 之和（MB）。只认 root 的子孙——共享机上其他会话的 rustc 不计入。
peak_rss_tree_mb() {
  local root="$1"
  ps -eo pid=,ppid=,rss=,comm= 2>/dev/null | awk -v root="$root" '
    { pid[NR]=$1; pp[$1]=$2; rss[$1]=$3; comm[$1]=$4 }
    END {
      q[1]=root; n=1; total=0
      for (i=1; i<=n; i++) {
        cur=q[i]
        for (k in pp) if (pp[k]==cur) { n++; q[n]=k }
      }
      for (i=2; i<=n; i++) {
        p=q[i]
        if (comm[p] !~ /^rustc/) continue
        total += rss[p]
      }
      print int(total/1024)
    }'
}

# 采样：每 0.5s 更新一次峰值，直到被测进程消失。
sample_peak() {
  local root="$1" out="$2" peak=0 cur
  while kill -0 "$root" 2>/dev/null; do
    cur="$(peak_rss_tree_mb "$root")"
    cur="${cur:-0}"
    [ "$cur" -gt "$peak" ] && peak="$cur"
    sleep 0.1
  done
  # 进程树消失后不再补采（RSS 已归零）；短构建的漏采改由 0.1s 周期 + 下面的 n/a 标注兜住。
  if [ "$peak" -le 0 ]; then
    printf 'n/a\n' > "$out"
  else
    printf '%s\n' "$peak" > "$out"
  fi
}

printf '[build-bench] 仓库：%s\n' "$REPO_ROOT"
printf '[build-bench] 槽位：%s  范围：%s  incremental=%s  起点：touch %s\n' \
  "$TARGET_DIR" "$SCOPE" "$INCREMENTAL" "$TOUCH_FILE"
printf '[build-bench] 当前 load1：%s（> 核数×1.5 时 lane 会先等，墙钟会被等待时间污染）\n' \
  "$(awk '{print $1}' /proc/loadavg 2>/dev/null || echo '?')"

ROWS_FILE="$(mktemp)"
trap 'rm -f "$ROWS_FILE" "$ROWS_FILE.tmp"' EXIT

FAILED=0
FAILED_SUMMARY=()

run_one() {
  local jobs="$1" scc="$2" peak_out="$3" row_out="$4"
  (
    cd "$REPO_ROOT" || exit 1
    touch "$TOUCH_FILE"
    start="$(date +%s.%N)"
    if [ "$SCOPE" = "all" ]; then
      PONY_BUILD_JOBS="$jobs" PONY_BUILD_SCCACHE="$scc" CARGO_INCREMENTAL="$INCREMENTAL" \
        bash scripts/cargo-lane.sh check --manifest-path "$CORE_MANIFEST" \
        --target-dir "$TARGET_DIR" --all-targets -q
    else
      PONY_BUILD_JOBS="$jobs" PONY_BUILD_SCCACHE="$scc" CARGO_INCREMENTAL="$INCREMENTAL" \
        bash scripts/cargo-lane.sh check --manifest-path "$CORE_MANIFEST" \
        --target-dir "$TARGET_DIR" -p pony-agent-core --lib -q
    fi
    rc=$?
    end="$(date +%s.%N)"
    # 本轮结果先写**本轮独占**的 row_out：多轮共用 ROWS_FILE 会让「补峰值」这步无法区分是哪一行，
    # 早先实现用 awk 原地重写只保留末行，使 jobs=4/8/12 的对比表只剩最后一行（审核 P0）。
    printf '%s %s %s %s\n' "$jobs" "$scc" "$rc" \
      "$(python3 -c "print(round(${end}-${start},2))")" > "$row_out"
    exit "$rc"
  ) &
  local runner=$!
  sample_peak "$runner" "$peak_out" &
  local sampler=$!
  wait "$runner"
  local rc=$?
  wait "$sampler" 2>/dev/null || true
  local peak_mb
  peak_mb="$(cat "$peak_out" 2>/dev/null || echo 0)"
  # 行与峰值严格配对：本轮行 + 本轮峰值合成一条追加，历史行一字不动。
  if [ -s "$row_out" ]; then
    printf '%s %s\n' "$(cat "$row_out")" "${peak_mb:-0}" >> "$ROWS_FILE"
  else
    printf '%s %s %s 0 0\n' "$jobs" "$scc" "1" >> "$ROWS_FILE"
  fi
  rm -f "$peak_out" "$row_out"
  [ "$rc" -eq 0 ] || return 1
}

while IFS= read -r jobs; do
  while IFS= read -r scc; do
    peak_file="$(mktemp)"
    row_file="$(mktemp)"
    if run_one "$jobs" "$scc" "$peak_file" "$row_file"; then
      printf '[build-bench] jobs=%s sccache=%s → ok\n' "$jobs" "$scc"
    else
      FAILED=1
      FAILED_SUMMARY+=("jobs=$jobs sccache=$scc")
      printf '[build-bench] jobs=%s sccache=%s → FAILED（后续轮次继续，最终以非零退出）\n' "$jobs" "$scc" >&2
    fi
    rm -f "$peak_file" "$row_file"
  done <<< "$SCCACHE_PARSED"
done <<< "$JOBS_PARSED"

python3 - "$ROWS_FILE" "$INCREMENTAL" "$SCOPE" "$LABEL" <<'PY'
import sys
rows_path, incremental, scope, label = sys.argv[1:5]
rows = []
for line in open(rows_path, encoding="utf-8"):
    parts = line.split()
    if len(parts) < 5:
        continue
    jobs, scc, rc, wall, peak = parts[:5]
    rows.append((int(jobs), int(scc), int(rc), float(wall), peak))
print()
print("| jobs | sccache | rc | 墙钟(s) | 峰值 rustc RSS(MB) | 条件 |")
print("|---|---|---|---|---|---|")
for jobs, scc, rc, wall, peak in rows:
    cond = f"incremental={incremental}, {scope}"
    if label:
        cond += f", {label}"
    mark = "FAILED" if rc != 0 else "ok"
    peak_txt = "n/a（采样未命中）" if peak == "n/a" else f"{peak} MB"
    print(f"| {jobs} | {scc} | {rc} ({mark}) | {wall} | {peak_txt} | {cond} |")
print("\n注：同一 target 槽位内连续多轮为**热缓存**测量（首轮之后依赖已就绪），"
      "故 jobs 对照只在「同为热缓存」的轮次之间可比；冷编对比请先清 incremental 再跑。")
if any(rc != 0 for _, _, rc, _, _ in rows):
    print("\n⚠️ 存在失败轮次：失败轮的墙钟/内存不代表真实成本，先修失败再取数。")
PY

if [ "$FAILED" -ne 0 ]; then
  echo "[build-bench] FAIL：${#FAILED_SUMMARY[@]} 轮失败（${FAILED_SUMMARY[*]}）" >&2
  exit 1
fi
echo "[build-bench] 全部轮次成功"
exit 0
