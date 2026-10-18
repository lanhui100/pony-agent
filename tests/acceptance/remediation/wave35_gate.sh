#!/usr/bin/env bash
# wave35_gate.sh — wave-35 修复波 独立反作弊验收门禁(Test Agent 所有;Executor 只读)
#
# 模式:
#   --static-only   仅 G1(反作弊静态门禁)+ G2-static(用例数/平台门禁守恒普查)。不碰 cargo。
#   --snapshot      生成基线快照到 tests/acceptance/receipts/wave35/ 后退出。
#   --full          G1 + G2(cargo --list 实测)+ G3(全量绿)+ G4(无挂起,900s 硬顶)。
#                   **只在 Lead 的 Join Barrier 通告后运行**,避免与 Executor 争用 cargo target 锁。
#   --verify-log <log>        机械校验任一全量 log 的完整性(非空/有终态汇总/无挂起标记/无编译 error)
#   --compare-logs <a> <b>    比对两份 log 的 test result 计数是否一致(剔除耗时抖动)
#
# 任一断言不成立 → 非零退出并逐条打印 VIOLATION 明细。
# 基线: commit f70b1f3;lib 用例数下限 1058;#[ignore] 属性数基线 0。
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "$REPO_ROOT" || { echo "FATAL: 无法进入仓库根 $REPO_ROOT"; exit 3; }

BASELINE_REV="${WAVE35_BASELINE_REV:-f70b1f3}"
RECEIPT_DIR="$REPO_ROOT/tests/acceptance/receipts/wave35"
LIB_TEST_FLOOR="${WAVE35_LIB_TEST_FLOOR:-1062}"   # Barrier 更新:基线 1058 + freeze-2 新增 4(path_permission 2 + retry 2)
RUN_TIMEOUT="${WAVE35_RUN_TIMEOUT:-900}"
CRATE="pony-agent-core"
LIB_SRC_TREE="crates/pony-agent-core/src"

# 声明写域:本波 Executor 允许触碰的文件/目录(runtime/control_plane 在仓库中是模块目录)
SCOPE_SPECS=(
  "crates/pony-agent-core/src/agent/tools.rs"
  "crates/pony-agent-core/src/agent/governed_executor.rs"
  "crates/pony-agent-core/src/agent/document_conversion.rs"
  "crates/pony-agent-core/src/agent/path_permission.rs"
  "crates/pony-agent-core/src/agent/runtime.rs"
  "crates/pony-agent-core/src/agent/runtime"
  "crates/pony-agent-core/src/agent/control_plane.rs"
  "crates/pony-agent-core/src/agent/control_plane"
  "crates/pony-agent-core/src/agent/retry.rs"
  "crates/pony-agent-core/tests/tool_surface_wave20_contract.rs"
  "crates/pony-agent-core/tests/wave31_lsp_client_test.rs"
)
# C 组:retry.rs 含生产判据变更,须有新增真断言测试覆盖(Barrier 追加要求)
RETRY_FILE="crates/pony-agent-core/src/agent/retry.rs"
RETRY_SUBSET_FLOOR=48
# 新增 #[allow(..)] 一律判 FAIL 的文件(Lead 裁决 c + Barrier 追加)
FATAL_ALLOW_FILES=(
  "crates/pony-agent-core/tests/tool_surface_wave20_contract.rs"
  "crates/pony-agent-core/tests/wave31_lsp_client_test.rs"
)
# 编译兼容衍生豁免文件:只允许删除未使用导入;断言/函数体/字符串任何改动 → FAIL
DERIVED_EXEMPT_FILES=(
  "crates/pony-agent-core/tests/wave31_lsp_client_test.rs"
)

# Lead 裁决(d,2026-10-18)显式豁免:属另一会话(TUI e2e 波)的未提交改动 /
# 已提交 release bump(Cargo.lock 仅 0.1.149→0.1.150),不在本波 5 文件范围。
# 依据:untracked .dev-team/l2at-done-tui-e2e.json 与 tests/acceptance/l2at-tui-e2e.sh 为同批产物。
EXCLUDED_PATHS=(
  "crates/pony-agent-core/src/bin/tui.rs"
  "Cargo.lock"
)
# Lead 裁决(c):unused-import 必须以"删除导入"修复,不接受 #[allow(unused_imports)] 消警。
TOOL_SURFACE_FILE="crates/pony-agent-core/tests/tool_surface_wave20_contract.rs"

MODE="static"; ARG1=""; ARG2=""
case "${1:---static-only}" in
  --static-only|--static) MODE="static" ;;
  --full)                 MODE="full" ;;
  --snapshot)             MODE="snapshot" ;;
  --verify-log)           MODE="verifylog";  ARG1="${2:?--verify-log 需要 log 路径}" ;;
  --compare-logs)         MODE="comparelogs"; ARG1="${2:?--compare-logs 需要 logA}"; ARG2="${3:?--compare-logs 需要 logB}" ;;
  -h|--help)              sed -n '2,11p' "${BASH_SOURCE[0]}"; exit 0 ;;
  *) echo "FATAL: 未知参数 $1(可用: --static-only | --snapshot | --full | --verify-log <log> | --compare-logs <a> <b>)"; exit 3 ;;
esac

TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
mkdir -p "$RECEIPT_DIR"
VIOL_FILE="$TMP/violations.txt"; : > "$VIOL_FILE"
viol() { printf 'VIOLATION [%s] %s\n' "$1" "$2" | tee -a "$VIOL_FILE"; }
info() { printf 'INFO      %s\n' "$1"; }
warn() { printf 'WARNING   %s\n' "$1"; }

git rev-parse --verify --quiet "$BASELINE_REV^{commit}" >/dev/null \
  || { echo "FATAL: 基线 commit $BASELINE_REV 不存在"; exit 3; }

# ------------------------------------------------------------- 断言普查器 -----
# 输出 TSV: 路径|类别(test/fn)|函数名|代码行|assert!|assert_eq!|assert_ne!|assert_matches!|
#           err断言|panic!|unwrap_or|let _|assert总数|unwrap()|起始行|结束行
cat > "$TMP/census.awk" <<'AWK'
function strip_line(s,   i,c,out,instr,inchr) {
  out=""; instr=0; inchr=0
  for (i=1; i<=length(s); i++) {
    c = substr(s,i,1)
    if (instr) { if (c=="\\") { i++; continue }; if (c=="\"") instr=0; continue }
    if (inchr) { if (c=="\\") { i++; continue }; if (c=="'") inchr=0; continue }
    if (c=="/" && substr(s,i+1,1)=="/") break
    if (c=="\"") { instr=1; continue }
    if (c=="'" && substr(s,i+2,1)=="'") { inchr=1; continue }
    out = out c
  }
  return out
}
function reset() {
  c_assert=0; c_aeq=0; c_ane=0; c_amat=0; c_err=0; c_pan=0; c_uor=0; c_let=0; c_unw=0
  c_code=0; depth=0; seen_open=0; start=0
}
function count_line(l0,   l,t) {
  t=l0; gsub(/^[ \t]+|[ \t]+$/,"",t)
  if (t != "" && t !~ /^\/\//) c_code++
  l=l0; c_aeq  += gsub(/(^|[^_A-Za-z0-9])assert_eq!/, "", l)
  l=l0; c_ane  += gsub(/(^|[^_A-Za-z0-9])assert_ne!/, "", l)
  l=l0; c_amat += gsub(/(^|[^_A-Za-z0-9])assert_matches!/, "", l)
  l=l0; c_assert += gsub(/(^|[^_A-Za-z0-9])assert!/, "", l)
  l=l0; c_err  += gsub(/expect_err/, "", l)
  l=l0; c_err  += gsub(/unwrap_err/, "", l)
  l=l0; c_pan  += gsub(/(^|[^_A-Za-z0-9])panic!/, "", l)
  l=l0; c_uor  += gsub(/unwrap_or/, "", l)
  l=l0; c_unw  += gsub(/unwrap\(\)/, "", l)
  l=l0; c_let  += gsub(/let _ =/, "", l)
}
function emit() {
  printf "%s\t%s\t%s\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\t%d\n", \
    FILENAME, kind, name, c_code, c_assert, c_aeq, c_ane, c_amat, c_err, c_pan, c_uor, c_let, \
    (c_assert+c_aeq+c_ane+c_amat), c_unw, start, NR, gated
}
BEGIN { in_fn=0; pending=0; cfg_seen=0; gated=0; reset() }
{
  clean = strip_line($0)
  trimmed = clean; gsub(/^[ \t]+|[ \t]+$/,"",trimmed)
  if (!in_fn) {
    if (trimmed ~ /^#\[cfg\(/ && trimmed !~ /^#\[cfg\(test\)\]/) { cfg_seen=1; next }
    if (trimmed ~ /^#\[(tokio::)?test(\(.*\))?\]$/) { pending=1; next }
    if (trimmed ~ /^#\[/) next
    if (trimmed ~ /^(pub[ \t]+)?(default[ \t]+)?(async[ \t]+)?fn[ \t]+[A-Za-z_][A-Za-z0-9_]*/) {
      match(trimmed, /fn[ \t]+[A-Za-z_][A-Za-z0-9_]*/)
      nm = substr(trimmed, RSTART+2, RLENGTH-2); gsub(/[ \t]/,"",nm)
      reset(); name=nm; kind=(pending?"test":"fn"); pending=0; in_fn=1; start=NR
      gated=cfg_seen; cfg_seen=0
    } else { next }
  }
  if (in_fn) {
    bodyc = clean
    oc = gsub(/\{/,"",bodyc); cc = gsub(/\}/,"",bodyc)
    if (depth>=1 || oc>0) count_line(clean)      # 单行函数体:本行即函数体
    depth += oc - cc
    if (oc>0) seen_open=1
    if (seen_open && depth<=0) { emit(); in_fn=0; reset() }
  }
}
AWK
census() { awk -f "$TMP/census.awk" "$1"; }

# 按 路径|类别|函数名 聚合(同名函数可能出现在多个 impl/mod 中;行号会随编辑漂移,故不入 key)
# 输出: key 路径 类别 函数名 代码行 指标(x7) 断言总数 unwrap 起 止 gated
cat > "$TMP/agg.awk" <<'AWK'
BEGIN { FS=OFS="\t" }
{ k = $1"|"$2"|"$3; if (!(k in seen)) { seen[k]=1; order[++n]=k; p[k]=$1; c[k]=$2; nm[k]=$3 }
  for (i=4;i<=14;i++) s[k,i]+=$i
  if ($17+0 > g[k]) g[k]=$17+0
}
END { for (j=1;j<=n;j++) { k=order[j]
    printf "%s\t%s\t%s\t%s", k, p[k], c[k], nm[k]
    for (i=5;i<=14;i++) printf "\t%d", s[k,i]+0
    printf "\t%d\t%d\n", s[k,4]+0, g[k]+0 } }
AWK
agg() { awk -f "$TMP/agg.awk" "$1"; }

# --------------------------------------------------------------- scope 解析 --
scope_files() {
  local spec
  for spec in "${SCOPE_SPECS[@]}"; do
    if [ -d "$spec" ]; then
      git ls-files -- "$spec" | grep '\.rs$'
      git ls-files --others --exclude-standard -- "$spec" | grep '\.rs$'
    elif [ -f "$spec" ]; then
      printf '%s\n' "$spec"
    fi
  done | sort -u
}
changed_scope_files() {
  git diff --name-only "$BASELINE_REV" -- "${SCOPE_SPECS[@]}"
  git ls-files --others --exclude-standard -- "${SCOPE_SPECS[@]}"
}

# lib 源码树里 #[test]/#[tokio::test] 函数名普查(基线集合必须被当前集合包含)
static_test_names() {
  local rev="$1" f
  if [ "$rev" = "WORKTREE" ]; then
    { git ls-files -- "$LIB_SRC_TREE" | grep '\.rs$'; git ls-files --others --exclude-standard -- "$LIB_SRC_TREE" | grep '\.rs$'; } \
      | sort -u | while IFS= read -r f; do
        [ -f "$f" ] && awk -v F="$f" 'BEGIN{p=0} {t=$0; gsub(/^[ \t]+|[ \t]+$/,"",t);
          if (t ~ /^#\[(tokio::)?test(\(.*\))?\]$/) {p=1; next}
          if (t ~ /^#\[/) next
          if (t ~ /^[^\/]*fn[ \t]+[A-Za-z_][A-Za-z0-9_]*/) {
            if (p) { match(t,/fn[ \t]+[A-Za-z_][A-Za-z0-9_]*/); nm=substr(t,RSTART+2,RLENGTH-2); gsub(/[ \t]/,"",nm); print F"::"nm }
            p=0 } }' "$f"
      done | sort -u
  else
    git ls-tree -r --name-only "$rev" -- "$LIB_SRC_TREE" | grep '\.rs$' | while IFS= read -r f; do
      git show "$rev:$f" | awk -v F="$f" 'BEGIN{p=0} {t=$0; gsub(/^[ \t]+|[ \t]+$/,"",t);
        if (t ~ /^#\[(tokio::)?test(\(.*\))?\]$/) {p=1; next}
        if (t ~ /^#\[/) next
        if (t ~ /^[^\/]*fn[ \t]+[A-Za-z_][A-Za-z0-9_]*/) {
          if (p) { match(t,/fn[ \t]+[A-Za-z_][A-Za-z0-9_]*/); nm=substr(t,RSTART+2,RLENGTH-2); gsub(/[ \t]/,"",nm); print F"::"nm }
          p=0 } }'
    done | sort -u
  fi
}

# 把 diff 的新增行映射到「新增后的行号 + 内容」,供测试区域归因
added_lines_numbered() { # $1 = file
  git diff -U0 "$BASELINE_REV" -- "$1" | awk '
    /^@@/ { match($0, /\+[0-9]+/); cur = substr($0, RSTART+1, RLENGTH-1) + 0; next }
    /^\+\+\+/ { next }
    /^\+/   { print cur "\t" substr($0,2); cur++; next }
    /^-/    { next }
    { cur++ }'
}

# 通用:成员判定
in_list() { local needle="$1"; shift; local x; for x in "$@"; do [ "$x" = "$needle" ] && return 0; done; return 1; }

# 编译兼容衍生豁免核验:剥掉全部 use/导入行后,文件其余部分必须逐字节一致
strip_imports_py() {
  python3 - "$1" <<'PY'
import re, sys
src = open(sys.argv[1], encoding="utf-8").read()
# 整条 use 语句(连同其后换行)一并移除,避免"删几条导入就多留几个空行"的假阳性
sys.stdout.write(re.sub(r'(?ms)^[ \t]*(pub[ \t]+)?use[ \t]+[^;]*;[ \t]*\n?', '', src))
PY
}

derived_exempt_check() { # $1=file $2=base_tsv $3=cur_tsv
  local f="$1" base_tsv="$2" cur_tsv="$3"
  git show "$BASELINE_REV:$f" > "$TMP/ex_base.rs" 2>/dev/null || { viol "DERIVED_BASELINE_MISSING" "$f: 基线版本缺失,无法做衍生豁免核验"; return; }
  strip_imports_py "$TMP/ex_base.rs" > "$TMP/ex_base.norm"
  strip_imports_py "$f"                  > "$TMP/ex_cur.norm"
  if diff -q "$TMP/ex_base.norm" "$TMP/ex_cur.norm" >/dev/null 2>&1; then
    info "G1 $f: 衍生豁免核验 PASS —— 剔除 use 行后其余内容逐字节一致(仅删未使用导入)"
  else
    viol "DERIVED_NON_IMPORT_CHANGE" "$f: 仅允许删未使用导入;非导入内容已变更 → $(diff -u "$TMP/ex_base.norm" "$TMP/ex_cur.norm" | grep -E '^[+-][^+-]' | head -3 | paste -sd' ¶ ' -)"
  fi
  # 断言/区域普查必须逐项完全相等(既不许减,也不许增)
  awk -F'\t' 'NR==FNR { k=$1"|"$2"|"$3; for(i=5;i<=14;i++) b[k,i]=$i+0; seen[k]=1; next }
    { k=$1"|"$2"|"$3; cur[k]=1
      for (i=5;i<=14;i++) if ($i+0 != b[k,i]+0) printf "DIFF\t%s\tcol%d\tbase=%d\tnow=%d\n", k, i, b[k,i]+0, $i+0 }
    END { for (k in seen) if (!(k in cur)) printf "GONE\t%s\n", k }
  ' "$base_tsv" "$cur_tsv" > "$TMP/ex_cmp.txt"
  while IFS=$'\t' read -r tag key col bv nv; do
    case "$tag" in
      DIFF) viol "DERIVED_CENSUS_CHANGED" "$f: 区域 $key 的 $col 由 $bv 变为 $nv(衍生豁免只允许删导入)" ;;
      GONE) viol "DERIVED_REGION_GONE"    "$f: 基线区域 $key 消失" ;;
    esac
  done < "$TMP/ex_cmp.txt"
}

# ================================================================ G1 =========
g1() {
  echo "=== G1 反作弊静态门禁 (git diff $BASELINE_REV -- <scope>) ==="
  local changed n_changed f
  changed="$(changed_scope_files | sed '/^$/d' | sort -u)"
  n_changed="$(printf '%s\n' "$changed" | sed '/^$/d' | wc -l)"
  info "G1 scope=$n_changed 个文件相对基线有变化"
  [ "$n_changed" -gt 0 ] || warn "G1: scope 内暂无 diff —— 门禁空过(Executor 尚未落盘),此结果无信息量"

  for f in $changed; do
    [ -f "$f" ] || { viol "FILE_DELETED" "$f: scope 内文件被删除"; continue; }
    local base_tsv="$TMP/base.tsv" cur_tsv="$TMP/cur.tsv" ranges="$TMP/ranges.txt"
    if git cat-file -e "$BASELINE_REV:$f" 2>/dev/null; then
      git show "$BASELINE_REV:$f" > "$TMP/base_src.rs"
      census "$TMP/base_src.rs" | sed "s#^$TMP/base_src.rs#$f#" > "$base_tsv"
    else
      : > "$base_tsv"; info "G1: $f 为基线后新增文件(按全量新增内容审查)"
    fi
    census "$f" > "$cur_tsv"
    awk -F'\t' '$2=="test"{print $15"\t"$16"\t"$3}' "$cur_tsv" > "$ranges"

    local added="$TMP/added.tsv"; added_lines_numbered "$f" > "$added"
    # 新增行按「是否落在测试函数体内」归因。**必须在消费它的检查之前构建**
    # (否则 G1.1c/1.1e 会读到上一文件的陈旧文件或空文件 —— 曾导致 retry.rs 计数失真与 WARN 假阳性)。
    awk -F'\t' -v OFS='\t' '
      NR==FNR { rs[++n]=$1; re[n]=$2; next }
      { for (i=1;i<=n;i++) if ($1+0>=rs[i]+0 && $1+0<=re[i]+0) { print $0; next } }
    ' "$ranges" "$added" > "$TMP/added_in_tests.tsv"

    # ---- G1.1 全域硬禁令(新增行) ----
    local lnno body
    while IFS=$'\t' read -r lnno body; do
      [ -n "$body" ] || continue
      case "$body" in *'#[ignore'*)             viol "NEW_IGNORE"        "$f:$lnno 新增 #[ignore] → $body" ;; esac
      case "$body" in *'#[cfg(not(test))]'*)    viol "NEW_CFG_NOT_TEST"  "$f:$lnno 新增 #[cfg(not(test))] → $body" ;; esac
      case "$body" in *'#![cfg('*)              viol "NEW_FILE_LEVEL_CFG" "$f:$lnno 新增文件级 #![cfg(..)] → $body" ;; esac
      case "$body" in *'#[cfg(not('*)           viol "NEW_NEGATED_CFG"   "$f:$lnno 新增否定式 cfg 整块跳过 → $body" ;; esac
      case "$body" in *'assert!(true)'*|*'assert!(!false)'*) viol "VACUOUS_ASSERT" "$f:$lnno 新增恒真断言 → $body" ;; esac
      case "$body" in *'unwrap_or(true)'*)      viol "UNWRAP_OR_TRUE"    "$f:$lnno 新增 unwrap_or(true)(错误当成功)→ $body" ;; esac
      case "$body" in
        *'--test-threads'*|*'RUST_TEST_THREADS'*) viol "THREAD_CAP_CHEAT" "$f:$lnno 测试线程串行化 → $body" ;;
      esac
      case "$body" in
        *'assert'*)
          case "$body" in *'|| true'*|*'&& true'*|*'== true'*|*'!= false'*)
            viol "TAUTOLOGY_ASSERT" "$f:$lnno 断言含恒真操作数(口径稀释)→ $body" ;; esac ;;
      esac
      case "$body" in
        *'cfg!('*|*'env::var('*|*'Instant::now'*|*'.elapsed()'*)
          case "$body" in *return*|*continue*|*skip*)
            viol "CONDITIONAL_SKIP" "$f:$lnno 新增条件性跳过 → $body" ;; esac ;;
      esac
      case "$body" in
        *'#[allow('*)
          if in_list "$f" "${FATAL_ALLOW_FILES[@]}"; then
            viol "FATAL_ALLOW_ATTR" "$f:$lnno 以 #[allow(..)] 掩盖告警(Lead 裁决 c:不接受)→ $body"
          else
            printf 'WARNING   %s\n' "WARN [ALLOW_ATTR] $f:$lnno 新增 #[allow(..)](需 review 是否掩盖告警)→ $body"
          fi ;;
      esac
    done < "$added"

    # ---- G1.1b tool_surface_contract:必须以删改 use 行修复 unused-import(Lead 裁决 c) ----
    if [ "$f" = "$TOOL_SURFACE_FILE" ]; then
      local rawdiff="$TMP/rawdiff.txt" a_use r_use a_allow
      git diff -U0 "$BASELINE_REV" -- "$f" > "$rawdiff"
      a_use=$(grep -cE '^\+.*\buse\b' "$rawdiff")
      r_use=$(grep -cE '^-.*\buse\b' "$rawdiff")
      a_allow=$(grep -cE '^\+.*#\[allow\(' "$rawdiff")
      info "G1 $f: use 行 删除 $r_use / 新增 $a_use;新增 #[allow] $a_allow"
      [ "$a_allow" -eq 0 ] || viol "TOOL_SURFACE_ALLOW_ATTR" "$f: 新增 #[allow(..)] $a_allow 处,拒绝用 allow 消警"
      [ $((r_use + a_use)) -gt 0 ] || viol "TOOL_SURFACE_IMPORT_UNTOUCHED" "$f: 未见 use 行增删,无法证明 unused-import 由删导入修复"
    fi

    # ---- G1.1c retry.rs:生产区改动必须伴随新增真断言测试(Barrier 追加要求) ----
    if [ "$f" = "$RETRY_FILE" ]; then
      local prod_added btnt atnt
      prod_added=$(( $(wc -l < "$added") - $(wc -l < "$TMP/added_in_tests.tsv") ))
      btnt=$(awk -F'\t' '$2=="test"' "$base_tsv" | wc -l)
      atnt=$(awk -F'\t' '$2=="test"' "$cur_tsv" | wc -l)
      info "G1 $f: 生产区新增行=$prod_added;测试函数数 基线=$btnt 当前=$atnt"
      if [ "$prod_added" -gt 0 ] && [ "$atnt" -le "$btnt" ]; then
        viol "PROD_CHANGE_WITHOUT_TEST" "$f: 生产代码新增 $prod_added 行但测试函数数未增($btnt→$atnt)—— 行为变更无新增覆盖"
      fi
      info "G1 $f: 生产判据变更已由新增测试覆盖的判定 = $( [ "$prod_added" -gt 0 ] && [ "$atnt" -gt "$btnt" ] && echo YES || echo NO )"
    fi

    # ---- G1.1d 编译兼容衍生豁免文件:只允许删未使用导入 ----
    if in_list "$f" "${DERIVED_EXEMPT_FILES[@]}"; then
      derived_exempt_check "$f" "$base_tsv" "$cur_tsv"
    fi

    # ---- G1.1e Unicode/URL 边界覆盖(源自本波 P0:index+1 字节切片在多字节字符后 panic) ----
    # 若生产新增行引入 `://` 扫描或字节索引切片,而新增测试行没有"多字节字符紧贴 scheme"的输入,
    # 则报 WARN:该 panic 类缺陷在修好的代码上恒绿、测试无法捕获(需 L3 读码确认)。
    local prod_lines="$TMP/prod_added.tsv"
    awk -F'\t' 'NR==FNR{seen[$1]=1; next} !($1 in seen)' "$TMP/added_in_tests.tsv" "$added" > "$prod_lines"
    local idx_lines
    idx_lines=$(grep -nE '://|rfind\(|char_indices|as_bytes\(\)|\[\.\.[a-z_]*\+|\[\.\.=' "$prod_lines" | head -5)
    if [ -n "$idx_lines" ]; then
      if grep -qP '[^\x00-\x7F](https?|ftp|wss?)://' "$TMP/added_in_tests.tsv" || grep -qP '[^\x00-\x7F].{0,8}://' "$TMP/added_in_tests.tsv"; then
        info "G1 $f: 生产新增了 URL/字节索引运算,新增测试含多字节紧邻 URL 的输入(Unicode 边界有覆盖)"
      else
        printf 'WARNING   %s\n' "WARN [UNICODE_URL_BOUNDARY_UNCOVERED] $f: 生产新增 URL/索引切片运算,但新增测试无非 ASCII 紧邻 scheme 的输入 → 字节切片 panic 类缺陷无法被测试捕获(需 L3 复核)"
        printf '%s\n' "$idx_lines" | sed "s/^/          证据 $f:/"
      fi
    fi

    # ---- G1.2 测试区域内软吞/弱化净增长(仅统计新增行落在测试函数体内的部分) ----
    local in_test_n; in_test_n=$(wc -l < "$TMP/added_in_tests.tsv")
    info "G1 $f: 新增行落在测试函数体内 $in_test_n 行"
    while IFS=$'\t' read -r lnno body; do
      [ -n "$body" ] || continue
      case "$body" in *'let _ ='*)      viol "LET_UNDERSCORE_GROWTH" "$f:$lnno 测试区内新增 let _ = (吞异常)→ $body" ;; esac
      case "$body" in *'unwrap_or_default'*|*'unwrap_or('*|*'unwrap_or_else'*)
        viol "UNWRAP_FALLBACK_GROWTH" "$f:$lnno 测试区内新增 unwrap_or* 兜底(断言口径稀释)→ $body" ;; esac
      case "$body" in
        *'assert'*) case "$body" in *'is_ok()'*|*'is_err()'*|*'is_some()'*|*'is_none()'*)
          printf 'WARNING   %s\n' "WARN [ASSERT_NARROWED_BOOL] $f:$lnno 断言退化为 bool 判定 → $body" ;; esac ;;
      esac
      case "$body" in *'return;'*|*'return Ok('*|*'return Err('*)
        printf 'WARNING   %s\n' "WARN [TEST_EARLY_RETURN] $f:$lnno 测试区内新增 return → $body" ;; esac
    done < "$TMP/added_in_tests.tsv"

    # ---- G1.3 逐区域断言普查(聚合口径) ----
    # 列: 1key 2path 3kind 4name 5assert! 6assert_eq 7assert_ne 8assert_matches 9err_assert
    #     10panic 11unwrap_or 12let_ 13assert_total 14unwrap 15code_lines 16gated
    agg "$base_tsv" > "$TMP/base_agg.tsv"
    agg "$cur_tsv"  > "$TMP/cur_agg.tsv"
    local bt ct na new_asserts
    bt=$(awk -F'\t' '{s+=$13} END{print s+0}' "$TMP/base_agg.tsv")
    ct=$(awk -F'\t' '{s+=$13} END{print s+0}' "$TMP/cur_agg.tsv")
    na=$(awk -F'\t' 'NR==FNR{b[$1]=1; next} ($3=="test" && !($1 in b)){s+=$13} END{print s+0}' "$TMP/base_agg.tsv" "$TMP/cur_agg.tsv")
    new_asserts="$na"
    info "G1 $f: 断言总数(文件级) 基线=$bt 当前=$ct;新增测试的断言数=$new_asserts"

    awk -F'\t' '
      BEGIN { split("path,kind,name,code_lines,assert_bang,assert_eq,assert_ne,assert_matches,err_assert,panic,unwrap_or,let_underscore,assert_total,unwrap,start,end", CN, ",") }
      NR==FNR { k=$1; for(i=5;i<=14;i++) b[k,i]=$i+0; seen[k]=1; bg[$1]=$15+0; next }
      { k=$1; cur[k]=1
        split("5 6 7 8 9 10 13", C, " ")
        for (j in C) { i=C[j]+0; if ($i+0 < b[k,i]+0) printf "DROP\t%s\t%s\t%d\t%d\t%d\n", k, CN[i], b[k,i]+0, $i+0, $16+0 }
        if ($15+0 <= bg[k]*0.5 && bg[k]>=10 && $13+0 >= b[k,13]+0 && $3=="test")
          printf "GUTTED\t%s\t%s\n", $1, $4
      }
      END { for (k in seen) if (!(k in cur)) printf "REMOVED\t%s\n", k }
    ' "$TMP/base_agg.tsv" "$TMP/cur_agg.tsv" > "$TMP/cmp.txt"

    while IFS=$'\t' read -r tag key col bv nv gated; do
      case "$tag" in
        REMOVED) viol "TEST_FN_REMOVED" "$f: 基线函数 $key 在当前版本消失(删除或改名)" ;;
        DROP)
          local drop=$((bv-nv)) name="${key##*|}"
          if [ "$gated" = "1" ] && [ "$ct" -ge "$bt" ] && [ "$new_asserts" -ge "$drop" ]; then
            warn "G1: $f 区域 $key 的 $col 由 $bv 降为 $nv —— 平台门禁拆分且已由新增测试的 $new_asserts 条断言补偿(文件级 $bt→$ct),判为合规"
          else
            viol "ASSERT_CENSUS_DROP" "$f: 区域 $key 的 $col 由 $bv 降为 $nv(未获补偿:gated=$gated 文件级 $bt→$ct 新增断言=$new_asserts)"
          fi ;;
        GUTTED) viol "GUTTED_TEST" "$f: 测试 $key 代码行腰斩而断言数不变(疑似抽空被测行为)" ;;
      esac
    done < "$TMP/cmp.txt"

    # ---- G1.4 新增测试零断言 / 新增空壳断言助手 ----
    awk -F'\t' 'NR==FNR{seen[$1]=1; next}
      { if ($1 in seen) next
        if ($3=="test" && $13+0==0) printf "ZERO_ASSERT_TEST\t%s\t%s\n", $2, $4
        else if ($3=="fn" && $4 ~ /^(assert|expect|check|verify|ensure|require)/ && $13+0==0 && $10+0==0 && $14+0==0)
          printf "EMPTY_GUARD_HELPER\t%s\t%s\n", $2, $4 }' "$TMP/base_agg.tsv" "$TMP/cur_agg.tsv" > "$TMP/newfns.txt"
    while IFS=$'\t' read -r tag tf nm; do
      case "$tag" in
        ZERO_ASSERT_TEST)   viol "ZERO_ASSERT_TEST"    "$tf: 新增测试 $nm 无任何断言(零假测试)" ;;
        EMPTY_GUARD_HELPER) viol "EMPTY_GUARD_HELPER"  "$tf: 新增断言助手 $nm 体内无 assert/panic/unwrap(断言洗白)" ;;
      esac
    done < "$TMP/newfns.txt"

    # ---- G1.5 文件级断言总量必须不下降 ----
    [ "$ct" -ge "$bt" ] || viol "ASSERT_TOTAL_DROP" "$f: 文件级断言总数 $bt→$ct"
  done
}

# ============================================ G2-static: 用例数守恒(免 cargo) =
g2_static() {
  echo "=== G2-static 用例数/平台门禁守恒普查(不调用 cargo) ==="
  static_test_names "$BASELINE_REV" > "$TMP/libnames_base.txt"
  static_test_names WORKTREE      > "$TMP/libnames_cur.txt"
  local nb nc
  nb=$(wc -l < "$TMP/libnames_base.txt"); nc=$(wc -l < "$TMP/libnames_cur.txt")
  info "G2-static 基线静态 #[test] 函数数=$nb;当前=$nc;cargo --list 实测下限常量=$LIB_TEST_FLOOR"
  comm -23 "$TMP/libnames_base.txt" "$TMP/libnames_cur.txt" > "$TMP/missing_names.txt"
  if [ -s "$TMP/missing_names.txt" ]; then
    while IFS= read -r n; do viol "TEST_NAME_LOST" "G2: 基线测试 $n 在 lib 源码中消失(删除/改名/移出 lib)"; done < "$TMP/missing_names.txt"
  fi

  local ign
  ign=$(grep -rE '^[[:space:]]*#\[ignore' "$LIB_SRC_TREE" 2>/dev/null | wc -l)
  info "G2 #[ignore] 属性(整 src 树,基线=0): $ign"
  [ "$ign" -eq 0 ] || viol "IGNORE_ATTR_PRESENT" "G2: src 树存在 $ign 处 #[ignore] 属性(基线 0)"

  local cfgw_b cfgw_c n_new_tests n_new_cfgw
  cfgw_b=$(git grep -hE '^[[:space:]]*#\[cfg\(windows\)\]' "$BASELINE_REV" -- "$LIB_SRC_TREE" | wc -l)
  cfgw_c=$(grep -rhE '^[[:space:]]*#\[cfg\(windows\)\]' "$LIB_SRC_TREE" 2>/dev/null | wc -l)
  n_new_tests=$(comm -13 "$TMP/libnames_base.txt" "$TMP/libnames_cur.txt" | wc -l)
  n_new_cfgw=$(git diff -U0 "$BASELINE_REV" -- "${SCOPE_SPECS[@]}" | grep -cE '^\+[[:space:]]*#\[cfg\(windows\)\]')
  info "G2 cfg(windows) 基线=$cfgw_b 当前=$cfgw_c;scope 内新增 #[cfg(windows)]=$n_new_cfgw;新增测试函数=$n_new_tests"
  if [ "$n_new_cfgw" -gt 0 ]; then
    warn "G2: 存在平台门禁新增(cfg_windows=+$n_new_cfgw / 新增测试=+$n_new_tests)—— 计数守恒待 Lead 裁决"
    [ "$n_new_tests" -ge "$n_new_cfgw" ] || viol "CFG_WINDOWS_UNBALANCED" \
      "G2: 新增 #[cfg(windows)] $n_new_cfgw 处 > 新增测试 $n_new_tests 个 —— 缺 POSIX 等价测试"
  fi

  local cfgdiff
  cfgdiff=$(git diff "$BASELINE_REV" -- Cargo.toml crates/pony-agent-core/Cargo.toml .cargo 2>/dev/null || true)
  if [ -n "$cfgdiff" ]; then
    while IFS= read -r l; do
      case "$l" in
        *'--test-threads'*|*'RUST_TEST_THREADS'*) viol "THREAD_CAP_CHEAT" "构建配置新增: $l" ;;
        *'harness = false'*)                      viol "HARNESS_DISABLED" "构建配置新增: $l" ;;
        *'required-features'*)                    viol "TEST_GATED_BY_FEATURE" "构建配置新增: $l" ;;
      esac
    done < <(printf '%s\n' "$cfgdiff" | grep -E '^\+' | grep -v '^\+\+\+')
  fi

  # 显式豁免(Lead 裁决 d):非本波范围,不进入任何越界判定与普查
  local excl_hit
  excl_hit=$(git diff --name-only "$BASELINE_REV" | grep -x -F -e "${EXCLUDED_PATHS[0]}" -e "${EXCLUDED_PATHS[1]}" | paste -sd, -)
  info "G2 显式豁免路径(Lead 裁决 d): ${EXCLUDED_PATHS[*]}"
  info "G2 豁免依据: 属另一会话(TUI e2e 波)未提交改动 + 已提交 release bump(Cargo.lock 0.1.149→0.1.150),非本波 5 文件范围;本门禁不判定、不普查"
  [ -z "$excl_hit" ] || info "G2 已从越界判定中剔除: $excl_hit"

  local out_scope
  out_scope=$(git diff --name-only "$BASELINE_REV" -- crates/ Cargo.lock \
    | grep -v -E '^crates/pony-agent-core/(src/agent/(tools|governed_executor|document_conversion|path_permission|retry)\.rs|src/agent/(runtime|control_plane)/|tests/(tool_surface_wave20_contract|wave31_lsp_client_test)\.rs)' \
    | grep -v -x -F -e "${EXCLUDED_PATHS[0]}" -e "${EXCLUDED_PATHS[1]}" || true)
  [ -z "$out_scope" ] || warn "G2: scope 外且未豁免的改动(需 Lead 归因): $(printf '%s' "$out_scope" | paste -sd, -)"
}

# ==================================================== G2-cargo / G3 / G4 =====
# 预检:等待其他 cargo 任务释放 target 锁。必须在 900s 计时之前完成,
# 否则锁等待会被算进 G4 耗时并误判为挂起。
preflight_wait_lock() {
  local max="${WAVE35_LOCK_WAIT:-1500}" waited=0 others detail
  while :; do
    # 只统计「真实 cargo 二进制 + cwd 在本仓库内」的任务:
    #  - 排除仅 cmdline 含 "cargo test" 的 bash 包装(僵尸管线会假占锁)
    #  - 排除其它仓库(如 ponyllm)的 cargo,它们不持有本仓库 target 锁
    detail=""
    local pid exe cwd
    for pid in $( { pgrep -x cargo 2>/dev/null; pgrep -f '/cargo (test|build|check|run)' 2>/dev/null; } | sort -u); do
      [ "$pid" = "$$" ] && continue
      exe=$(readlink "/proc/$pid/exe" 2>/dev/null) || continue
      case "$exe" in */cargo) ;; *) continue ;; esac
      cwd=$(readlink "/proc/$pid/cwd" 2>/dev/null) || continue
      case "$cwd" in "$REPO_ROOT"*) detail="$detail $pid" ;; esac
    done
    others=$(printf '%s' "$detail" | wc -w)
    [ "$others" -eq 0 ] && break
    if [ "$waited" -ge "$max" ]; then
      warn "G3 预检: 等待 ${waited}s 后本仓库仍有 $others 个 cargo 任务在跑($detail)—— 继续,但收据可能受 target 锁串行化影响"
      break
    fi
    if [ $((waited % 30)) -eq 0 ]; then
      info "G3 预检: 等待本仓库 cargo 任务释放 target 锁(${waited}s,pid:$detail)"
    fi
    sleep 5; waited=$((waited + 5))
  done
  info "G3 预检: 锁等待结束(累计 ${waited}s);此后才开始 ${RUN_TIMEOUT}s 计时"
}

g2_cargo() {
  echo "=== G2 cargo --list 实测 ==="
  preflight_wait_lock
  local out="$RECEIPT_DIR/lib_list_$(date +%Y%m%dT%H%M%S).txt" rc n
  cargo test -p "$CRATE" --lib -- --list > "$out" 2>&1; rc=$?
  info "G2 收据: $out (exit=$rc)"
  [ "$rc" -eq 0 ] || { viol "LIB_LIST_CMD_FAILED" "G2: cargo test --lib -- --list 退出码 $rc"; tail -20 "$out" | sed 's/^/    /'; return; }
  n=$(grep ': test$' "$out" | wc -l)
  info "G2 lib 用例数 = $n(下限 $LIB_TEST_FLOOR)"
  [ "$n" -ge "$LIB_TEST_FLOOR" ] || viol "LIB_TEST_COUNT_DROP" "G2: lib 用例数 $n < 基线 $LIB_TEST_FLOOR"
}

g3_g4() {
  echo "=== G3/G4 全量套件(硬顶 ${RUN_TIMEOUT}s) ==="
  local ts out rc t0 t1 el failed out2 rc2
  ts="$(date +%Y%m%dT%H%M%S)"
  out="$RECEIPT_DIR/lib_full_${ts}.log"
  t0=$(date +%s)
  RUSTFLAGS="-D warnings" timeout "$RUN_TIMEOUT" cargo test -p "$CRATE" --lib > "$out" 2>&1; rc=$?
  t1=$(date +%s); el=$((t1-t0))
  info "G3 收据(raw stdout): $out ; exit=$rc ; 耗时=${el}s"
  tail -25 "$out" | sed 's/^/    /'
  if [ "$rc" -eq 124 ]; then viol "G4_HANG_TIMEOUT" "G4: 全量套件在 ${RUN_TIMEOUT}s 内未结束(超时击杀)"
  else [ "$el" -lt "$RUN_TIMEOUT" ] || viol "G4_TOO_SLOW" "G4: 全量耗时 ${el}s ≥ ${RUN_TIMEOUT}s"; fi
  [ "$rc" -eq 0 ] || [ "$rc" -eq 124 ] || viol "G3_LIB_NOT_GREEN" "G3: RUSTFLAGS=-D warnings cargo test --lib 退出码 $rc(要求 0)"
  failed=$(grep -oE '[0-9]+ failed' "$out" | awk '{s+=$1} END{print s+0}')
  [ "$failed" -eq 0 ] || viol "G3_FAILED_CASES" "G3: 汇总 $failed 个 failed 用例"
  grep -qE 'test result: ok\.' "$out" || viol "G3_NO_OK_SUMMARY" "G3: 输出无 'test result: ok.'"
  grep -qE 'error(\[E[0-9]+\])?:' "$out" && viol "G3_WARNINGS_AS_ERRORS" "G3: 输出含编译 error(见收据)"

  out2="$RECEIPT_DIR/tool_surface_wave20_${ts}.log"
  RUSTFLAGS="-D warnings" timeout "$RUN_TIMEOUT" cargo test -p "$CRATE" --test tool_surface_wave20_contract > "$out2" 2>&1; rc2=$?
  info "G3 收据: $out2 ; exit=$rc2"
  tail -15 "$out2" | sed 's/^/    /'
  [ "$rc2" -eq 0 ] || viol "G3_TOOL_SURFACE_RED" "G3: --test tool_surface_wave20_contract 退出码 $rc2(要求 0)"

  # G3c:retry 分类子集不得被放宽(Barrier 追加要求)
  local out3="$RECEIPT_DIR/retry_subset_${ts}.log" rc3 p3
  RUSTFLAGS="-D warnings" timeout "$RUN_TIMEOUT" cargo test -p "$CRATE" --lib -- retry > "$out3" 2>&1; rc3=$?
  p3=$(grep -E '^test result:' "$out3" | grep -oE '[0-9]+ passed' | grep -oE '[0-9]+' | awk '{s+=$1} END{print s+0}')
  info "G3 retry 子集: exit=$rc3 passed=$p3(下限 $RETRY_SUBSET_FLOOR);收据 $out3"
  grep -E '^test result:' "$out3" | sed 's/^/    /'
  [ "$rc3" -eq 0 ] || viol "G3_RETRY_SUBSET_RED" "G3: '-- retry' 子集退出码 $rc3(要求 0)"
  [ "$p3" -ge "$RETRY_SUBSET_FLOOR" ] || viol "G3_RETRY_SUBSET_SHRUNK" "G3: '-- retry' passed=$p3 < $RETRY_SUBSET_FLOOR(既有分类测试被删/停跑)"
}

# ============================ 收据机械校验 / 交叉比对(Lead 指派) =============
# 校验任一全量 log 的完整性:非空、含 'test result:' 终态汇总、无挂起中断标记。
verify_log() {
  local log="$1" nres failed passed bytes
  echo "=== verify-log: $log ==="
  if [ ! -s "$log" ]; then viol "LOG_EMPTY" "log 不存在或为空: $log"; return; fi
  bytes=$(wc -c < "$log")
  nres=$(grep -cE '^test result:' "$log")
  passed=$(grep -E '^test result:' "$log" | sed -E 's/^test result: [A-Za-z]+\. //' | grep -oE '^[0-9]+ passed' | grep -oE '^[0-9]+' | awk '{s+=$1} END{print s+0}')
  failed=$(grep -E '^test result:' "$log" | grep -oE '[0-9]+ failed' | awk '{s+=$1} END{print s+0}')
  info "LOG 大小=${bytes}B;'test result:' 行=$nres;passed=$passed;failed=$failed"
  [ "$nres" -gt 0 ] || viol "LOG_NO_SUMMARY" "缺 'test result:' 终态汇总(疑似被中断/挂起)"
  grep -qE '^test result: FAILED' "$log" && viol "LOG_HAS_FAILURE" "含 FAILED 汇总行"
  grep -qE 'has been running for over [0-9]+ seconds' "$log" && viol "LOG_HANG_MARKER" "含 'has been running for over N seconds'(存在挂起用例)"
  grep -qE '^error(\[E[0-9]+\])?:' "$log" && viol "LOG_COMPILE_ERROR" "含编译 error"
  [ "$failed" -eq 0 ] || viol "LOG_FAILED_CASES" "汇总 failed=$failed > 0"
  if [ "$nres" -gt 0 ]; then
    [ "$passed" -ge "$LIB_TEST_FLOOR" ] || viol "LOG_PASSED_BELOW_FLOOR" "passed=$passed < 下限 $LIB_TEST_FLOOR"
  fi
  tail -3 "$log" | sed 's/^/    /'
}

# 比对两份 log 的 test result 计数(剔除耗时,避免抖动误判)
compare_logs() {
  local a="$1" b="$2" na nb
  echo "=== compare-logs: A=$a B=$b ==="
  [ -s "$a" ] || { viol "LOG_EMPTY" "A 为空: $a"; return; }
  [ -s "$b" ] || { viol "LOG_EMPTY" "B 为空: $b"; return; }
  na=$(grep -E '^test result:' "$a" | sed -E 's/; finished in.*$//' | sort | paste -sd'|' -)
  nb=$(grep -E '^test result:' "$b" | sed -E 's/; finished in.*$//' | sort | paste -sd'|' -)
  info "A 汇总: $na"
  info "B 汇总: $nb"
  if [ -z "$na" ] || [ -z "$nb" ]; then viol "LOG_NO_SUMMARY" "至少一份缺 'test result:' 行,无法比对"; return; fi
  [ "$na" = "$nb" ] || viol "LOG_MISMATCH" "两份 log 的 test result 计数不一致"
}

# =============================================================== snapshot ====
snapshot() {
  echo "=== 基线快照($BASELINE_REV)→ $RECEIPT_DIR ==="
  local snap="$RECEIPT_DIR/baseline_snapshot.tsv" facts="$RECEIPT_DIR/baseline_facts.txt" f
  : > "$snap"
  for f in $(scope_files); do
    if git cat-file -e "$BASELINE_REV:$f" 2>/dev/null; then
      git show "$BASELINE_REV:$f" > "$TMP/s.rs"
      census "$TMP/s.rs" | sed "s#^$TMP/s.rs#$f#"
    else
      printf '%s\t%s\t%s\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\t0\n' "$f" "new" "-"
    fi
  done > "$snap"
  static_test_names "$BASELINE_REV" > "$TMP/nb.txt"
  static_test_names WORKTREE      > "$TMP/nc.txt"
  {
    echo "# wave35 基线事实快照 $(date -Is)"
    echo "baseline_commit=$(git rev-parse "$BASELINE_REV")"
    echo "lib_test_floor_const=$LIB_TEST_FLOOR"
    echo "static_test_fns_baseline=$(wc -l < "$TMP/nb.txt")"
    echo "static_test_fns_current=$(wc -l < "$TMP/nc.txt")"
    echo "ignore_attrs_baseline_src=$(git grep -hE '^[[:space:]]*#\[ignore' "$BASELINE_REV" -- "$LIB_SRC_TREE" | wc -l)"
    echo "ignore_attrs_current_src=$(grep -rE '^[[:space:]]*#\[ignore' "$LIB_SRC_TREE" 2>/dev/null | wc -l)"
    echo "cfg_windows_baseline=$(git grep -hE '^[[:space:]]*#\[cfg\(windows\)\]' "$BASELINE_REV" -- "$LIB_SRC_TREE" | wc -l)"
    echo "cfg_windows_current=$(grep -rhE '^[[:space:]]*#\[cfg\(windows\)\]' "$LIB_SRC_TREE" 2>/dev/null | wc -l)"
    echo "scope_files=$(scope_files | paste -sd, -)"
    echo "scope_changed=$(changed_scope_files | sed '/^$/d' | paste -sd, -)"
  } > "$facts"
  info "写入 $snap($(wc -l < "$snap") 行函数级基线普查)"
  info "写入 $facts"
  sed 's/^/    /' "$facts"
}

# ================================================================ main ======
echo "wave35 gate | mode=$MODE | base=$BASELINE_REV | repo=$REPO_ROOT"
case "$MODE" in
  snapshot)    snapshot ;;
  static)      g1; g2_static ;;
  full)        g1; g2_static; g2_cargo; g3_g4 ;;
  verifylog)   verify_log "$ARG1" ;;
  comparelogs) compare_logs "$ARG1" "$ARG2" ;;
esac
NV=$(wc -l < "$VIOL_FILE")
echo "-------------------------------------------------------------"
if [ "$NV" -gt 0 ]; then
  echo "RESULT: FAIL — $NV 条违规(mode=$MODE)"; cat "$VIOL_FILE"; exit 1
fi
echo "RESULT: PASS — 0 违规(mode=$MODE)"
[ "$MODE" = "static" ] && echo "NOTE: G2-cargo/G3/G4 未执行(static 模式);须在 Lead Join Barrier 后以 --full 运行。"
exit 0