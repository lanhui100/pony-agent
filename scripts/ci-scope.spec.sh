#!/usr/bin/env bash
# ============================================================
# ci-scope.spec.sh —— scripts/ci-scope.mjs 的正负样本自检
#
# 为什么必须有：ci-scope.mjs 决定「本次改动要不要跑 Rust 全量门（cargo check /
# clippy 棘轮 / cargo test）」。它一旦判错方向，门禁不是变红而是**静默消失**——
# 纯文档 PR 被误判为「不需要 Rust」只是省了几分钟；代码 PR 被误判为「不需要」
# 却是整块门禁跳过，而 CI 依然全绿。收益与代价不对称，所以两个方向各打一枪。
#
# 用法：bash scripts/ci-scope.spec.sh
# 退出码：全通过 exit 0；任一断言失败 exit 1（逐项打印 FAIL）。
# 隔离：全程在 mktemp -d 下建独立 git 仓库，不触碰真实工作区/真实 .git。
# ============================================================
set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CI_SCOPE="$ROOT/scripts/ci-scope.mjs"
FAIL=0

fail() { printf 'FAIL  %s\n' "$*" >&2; FAIL=1; }
ok()   { printf 'PASS  %s\n' "$*"; }

[ -f "$CI_SCOPE" ] || { echo "ci-scope.spec: 缺 $CI_SCOPE" >&2; exit 1; }
command -v node >/dev/null 2>&1 || { echo "ci-scope.spec: 需要 node" >&2; exit 1; }
command -v git  >/dev/null 2>&1 || { echo "ci-scope.spec: 需要 git"  >&2; exit 1; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/ci-scope-spec.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

REPO="$WORK/repo"
mkdir -p "$REPO"
(
  cd "$REPO" || exit 1
  git init -q
  git config user.email spec@example.com
  git config user.name spec
  git config commit.gpgsign false
  printf 'initial\n' > .keep
  git add .keep
  git commit -qm "chore: init" >/dev/null 2>&1
) || { echo "ci-scope.spec: 初始化临时仓库失败" >&2; exit 1; }

BASE_SHA="$(cd "$REPO" && git rev-parse HEAD)"

# commit_case <名字> <路径...>：在 BASE 之上建一条只含这些文件的提交，回显 head sha。
commit_case() {
  local name="$1"
  shift
  local file
  (
    cd "$REPO" || exit 1
    git checkout -q -B "case/$name" "$BASE_SHA"
    for file in "$@"; do
      mkdir -p "$(dirname "$file")"
      printf 'x\n' > "$file"
    done
    git add -A
    git commit -qm "chore: $name" >/dev/null 2>&1
    git rev-parse HEAD
  )
}

# assert_scope <用例名> <base> <期望 needs_rust> <期望 scope_reason> [head]
assert_scope() {
  local name="$1" base="$2" want_needs="$3" want_reason="$4" head="${5:-HEAD}"
  local out code needs reason
  out="$(cd "$REPO" && node "$CI_SCOPE" --base "$base" --head "$head" 2>&1)"
  code=$?
  needs="$(printf '%s\n' "$out" | sed -n 's/^needs_rust=//p')"
  reason="$(printf '%s\n' "$out" | sed -n 's/^scope_reason=//p')"

  if [ "$code" -ne 0 ]; then
    fail "$name：脚本必须恒 exit 0，实测 exit=$code（输出：$out）"
    return
  fi
  if [ "$needs" != "$want_needs" ]; then
    fail "$name：needs_rust 期望 $want_needs，实测 ${needs:-<空>}"
    return
  fi
  # reason 只对前缀做匹配：error 分支带自由文本
  case "$reason" in
    "$want_reason"*) ok "$name（needs_rust=$needs, reason=$reason）" ;;
    *) fail "$name：scope_reason 期望 ${want_reason}*，实测 ${reason:-<空>}" ;;
  esac
}

# assert_flag <用例名> <base> <head> <字段名> <期望值>
assert_flag() {
  local name="$1" base="$2" head="$3" key="$4" want="$5"
  local out got
  out="$(cd "$REPO" && node "$CI_SCOPE" --base "$base" --head "$head" 2>&1)"
  got="$(printf '%s\n' "$out" | sed -n "s/^${key}=//p")"
  if [ "$got" != "$want" ]; then
    fail "$name：${key} 期望 ${want}，实测 ${got:-<空>}"
  else
    ok "$name（${key}=${got}）"
  fi
}

# ---------- 负样本：纯文档/账本改动必须跳过 Rust ----------
HEAD_DOCS="$(commit_case docs-only docs/architecture/guide.md)"
assert_scope "A 文档树改动跳过 Rust" "$BASE_SHA" false docs-only "$HEAD_DOCS"

HEAD_BACKLOG="$(commit_case backlog-only backlog/tasks.jsonl)"
assert_scope "B 账本归档跳过 Rust" "$BASE_SHA" false docs-only "$HEAD_BACKLOG"

HEAD_ROOT_MD="$(commit_case root-md README.md)"
assert_scope "C 根级 markdown 跳过 Rust" "$BASE_SHA" false docs-only "$HEAD_ROOT_MD"

HEAD_ADR="$(commit_case adr-only .agents/notes/implemented/process/2026-01-01-x.md)"
assert_scope "D 决策记录跳过 Rust" "$BASE_SHA" false docs-only "$HEAD_ADR"

# ---------- 正样本：编译面与门禁真值源必须跑 Rust ----------
HEAD_CRATE="$(commit_case crate-src crates/pony-agent-core/src/lib.rs)"
assert_scope "E crate 源码触发 Rust" "$BASE_SHA" true code "$HEAD_CRATE"

HEAD_SCRIPT="$(commit_case scripts scripts/ci-scope.mjs)"
assert_scope "F scripts 自身触发 Rust" "$BASE_SHA" true code "$HEAD_SCRIPT"

HEAD_CI="$(commit_case ci-yaml .github/workflows/ci.yml)"
assert_scope "G CI 定义触发 Rust（改门禁必须自验）" "$BASE_SHA" true code "$HEAD_CI"

# 关键防退化点：.meta 下的 markdown 是 lint 表/门禁真值源，不能因后缀是 .md 就放行。
HEAD_META="$(commit_case meta-md .meta/lints/README.md)"
assert_scope "H .meta 下的 markdown 仍触发 Rust" "$BASE_SHA" true code "$HEAD_META"

HEAD_FRONT="$(commit_case frontend src/lib/open-external-url.ts)"
assert_scope "I 前端源码触发 Rust（含 typecheck 之外的编译面）" "$BASE_SHA" true code "$HEAD_FRONT"

# ---------- fail-safe 分支 ----------
assert_scope "J base 不可解析时回落全量" "0000000000000000000000000000000000000000" true no-base
assert_scope "K 空 diff 视为无需 Rust" "$BASE_SHA" false empty-diff "$BASE_SHA"

# ---------- B1795615060519 细粒度 crate / 前端分流 ----------
HEAD_TUI="$(commit_case tui-only crates/pony-tui/src/tui.rs)"
assert_flag "L1 仅改 pony-tui -> needs_tui=true" "$BASE_SHA" "$HEAD_TUI" needs_tui true
assert_flag "L2 仅改 pony-tui -> needs_core=false" "$BASE_SHA" "$HEAD_TUI" needs_core false
assert_flag "L3 仅改 pony-tui -> needs_cli=false" "$BASE_SHA" "$HEAD_TUI" needs_cli false
assert_flag "L4 仅改 pony-tui -> needs_frontend=false" "$BASE_SHA" "$HEAD_TUI" needs_frontend false

HEAD_CLI="$(commit_case cli-only crates/pony-cli/src/main.rs)"
assert_flag "M1 仅改 pony-cli -> needs_cli=true" "$BASE_SHA" "$HEAD_CLI" needs_cli true
assert_flag "M2 仅改 pony-cli -> needs_tui=false" "$BASE_SHA" "$HEAD_CLI" needs_tui false
assert_flag "M3 仅改 pony-cli -> needs_core=false" "$BASE_SHA" "$HEAD_CLI" needs_core false

HEAD_PROTO="$(commit_case proto-only crates/pony-protocol/src/lib.rs)"
assert_flag "N1 底层 pony-protocol 传递触发 needs_core=true" "$BASE_SHA" "$HEAD_PROTO" needs_core true
assert_flag "N2 底层 pony-protocol 传递触发 needs_tui=true" "$BASE_SHA" "$HEAD_PROTO" needs_tui true
assert_flag "N3 底层 pony-protocol 传递触发 needs_cli=true" "$BASE_SHA" "$HEAD_PROTO" needs_cli true
assert_flag "N4 底层 pony-protocol 传递触发 needs_tauri=true" "$BASE_SHA" "$HEAD_PROTO" needs_tauri true
assert_flag "N5 底层 pony-protocol 不触发 needs_frontend" "$BASE_SHA" "$HEAD_PROTO" needs_frontend false

assert_flag "O no-base fail-safe -> needs_tui=true" "0000000000000000000000000000000000000000" HEAD needs_tui true
assert_flag "P docs-only -> needs_frontend=false" "$BASE_SHA" "$HEAD_DOCS" needs_frontend false

if [ "$FAIL" -ne 0 ]; then
  echo "ci-scope.spec: 有断言失败" >&2
  exit 1
fi
echo "ci-scope.spec: 全部通过"
exit 0
