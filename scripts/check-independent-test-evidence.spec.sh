#!/usr/bin/env bash
# ============================================================
# check-independent-test-evidence.spec.sh —— 「非执行者测试证据」门的正负样本自检
#
# 为什么必须有：本门判的是「dev-team 批次是否有一条非执行者产生的测试证据」。它一旦判错方向，
# 后果不对称——误判通过 ⇒ 单点自证（执行者自跑 exit 0）继续放行，正是 `backlog:B1795571368443`
# 要消灭的那类软约束；误判失败 ⇒ 只是多一条显式证据要补。故正反两侧各打一枪，另加两条
# 「不该判」的边界（无 dev-team 标记 / 纯文档）与两条 fail-closed 边界（无法解析 base）。
#
# 用法：bash scripts/check-independent-test-evidence.spec.sh
# 退出码：全通过 exit 0；任一断言失败 exit 1（逐项打印 FAIL）。
# 隔离：全程在 mktemp -d 下建独立 git 仓库，不触碰真实工作区/真实 .git。
# ============================================================
set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECK="$ROOT/scripts/check-independent-test-evidence.mjs"
FAIL=0

fail() { printf 'FAIL  %s\n' "$*" >&2; FAIL=1; }
ok()   { printf 'PASS  %s\n' "$*"; }

[ -f "$CHECK" ] || { echo "test-evidence.spec: 缺 $CHECK" >&2; exit 1; }
command -v node >/dev/null 2>&1 || { echo "test-evidence.spec: 需要 node" >&2; exit 1; }
command -v git  >/dev/null 2>&1 || { echo "test-evidence.spec: 需要 git"  >&2; exit 1; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/test-evidence-spec.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

REPO="$WORK/repo"
mkdir -p "$REPO"
(
  cd "$REPO" || exit 1
  git init -q
  git config user.email lead@example.com
  git config user.name lead
  git config commit.gpgsign false
  mkdir -p src
  printf 'fn main() {}\n' > src/lib.rs
  git add src/lib.rs
  git commit -qm "chore: init" >/dev/null 2>&1
) || { echo "test-evidence.spec: 初始化临时仓库失败" >&2; exit 1; }

BASE_SHA="$(cd "$REPO" && git rev-parse HEAD)"

# commit_case <名字> <路径> <提交信息文件内容>
# 在 BASE 之上建一条提交（每条都基于同一 BASE，用分支隔离），回显 head sha。
commit_case() {
  local name="$1" path="$2" message="$3"
  (
    cd "$REPO" || exit 1
    git checkout -q -B "case-$name" "$BASE_SHA"
    mkdir -p "$(dirname "$path")"
    printf '// %s\n' "$name" >> "$path"
    git add -A
    printf '%s\n' "$message" | git commit -q -F - >/dev/null 2>&1
    git rev-parse HEAD
  )
}

# expect_rc <期望退出码> <名字> <sha> [额外参数...]
expect_rc() {
  local want="$1" name="$2" sha="$3"
  shift 3
  local out rc
  out="$(cd "$REPO" && node "$CHECK" --base "$BASE_SHA" --head "$sha" "$@" 2>&1)"; rc=$?
  if [ "$rc" -ne "$want" ]; then
    fail "$name：期望 exit $want，实际 $rc；输出：$out"
  else
    ok "$name（exit $rc）"
  fi
}

# ---- P1：非执行者证据 ⇒ 放行 ------------------------------------------------
SHA="$(commit_case p1 src/feature.rs "feat: dev-team 批次实现

Team-Run: 2026-11-25-b1795571368443
Test-Evidence: test-agent | cargo test -p pony-agent-core | exit=0
")"
expect_rc 0 "P1 非执行者证据放行" "$SHA"

# ---- P2：只有执行者自证 ⇒ 拒绝 ---------------------------------------------
SHA="$(commit_case p2 src/feature.rs "feat: dev-team 批次实现

Team-Run: 2026-11-25-b1795571368443
Test-Evidence: lead | cargo test -p pony-agent-core | exit=0
")"
expect_rc 1 "P2 执行者自证必须判红" "$SHA"

# ---- P3：dev-team 批次但完全没有证据 ⇒ 拒绝 --------------------------------
SHA="$(commit_case p3 src/feature.rs "feat: dev-team 批次实现

Team-Run: 2026-11-25-b1795571368443
")"
expect_rc 1 "P3 缺证据必须判红" "$SHA"

# ---- P4：无 dev-team 标记 ⇒ 跳过（不打扰普通 PR）--------------------------
SHA="$(commit_case p4 src/feature.rs "fix(provider): 普通修复，无团队标记
")"
expect_rc 0 "P4 无团队标记跳过" "$SHA"

# ---- P5：dev-team 批次但只改文档/账本 ⇒ 跳过 -------------------------------
SHA="$(commit_case p5 docs/note.md "docs: dev-team 批次只改文档

Team-Run: 2026-11-25-b1795571368443
")"
expect_rc 0 "P5 纯文档跳过" "$SHA"

# ---- P6：dev-team 字面出现在信息里也算标记（不只认 trailer）-----------------
SHA="$(commit_case p6 src/feature.rs "feat: dev-team 批次实现（用 dev-team 跑）
")"
expect_rc 1 "P6 dev-team 字面标记同样触发判定" "$SHA"

# ---- P7：证据产出者大小写/空格不匹配作者 ⇒ 视为独立（严格字面比较）---------
SHA="$(commit_case p7 src/feature.rs "feat: dev-team 批次实现

Team-Run: 2026-11-25-b1795571368443
Test-Evidence: test-agent | cargo test | exit=0
Test-Evidence: lead | cargo check | exit=0
")"
expect_rc 0 "P7 混合证据里有一条独立即放行" "$SHA"

# ---- N1：无法解析 base ⇒ fail-closed exit 2（不得伪装通过）-----------------
out="$(cd "$REPO" && node "$CHECK" --base "0000000000000000000000000000000000000000" --head HEAD 2>&1)"; rc=$?
if [ "$rc" -eq 2 ]; then
  ok "N1 无法解析 base 时 fail-closed（exit 2）"
else
  fail "N1 期望 exit 2，实际 $rc；输出：$out"
fi

# ---- N2：未知参数不改变语义但 --help 可用 ---------------------------------
out="$(cd "$REPO" && node "$CHECK" --help 2>&1)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -qF 'Test-Evidence'; then
  ok "N2 --help 打印载体格式并 exit 0"
else
  fail "N2 --help 期望 exit 0 且含载体说明，实际 exit $rc；输出：$out"
fi

if [ "$FAIL" -ne 0 ]; then
  echo "== check-independent-test-evidence.spec：FAIL（见上）==" >&2
  exit 1
fi
echo "== check-independent-test-evidence.spec：全部通过（P1–P7 正负样本 + N1–N2 边界）=="
