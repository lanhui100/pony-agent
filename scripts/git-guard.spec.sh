#!/usr/bin/env bash
# ============================================================
# git-guard.spec.sh —— scripts/git-guard.mjs 的负样本自检
#
# 为什么必须有：git-guard 是「多会话并发 git 契约」在本地的唯一执行点。
# 它若静默退化（历史教训：整段被 `if command -v pwsh` 包住 → Linux 侧永不生效，
# 见 docs/audit/engineering-quality-audit-2026-11-01.md:122），远端门禁仍会绿，
# 因为远端只看得到「已经推上来的东西」。本脚本在临时仓库里正负两侧各打一枪。
#
# 用法：bash scripts/git-guard.spec.sh
# 退出码：全通过 exit 0；任一断言失败 exit 1（逐项打印 FAIL）。
# 隔离：全程在 mktemp -d 下建独立 git 仓库，不触碰真实工作区/真实 .git。
# ============================================================
set -u

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GUARD="$ROOT/scripts/git-guard.mjs"
FAIL=0

fail() { printf 'FAIL  %s\n' "$*" >&2; FAIL=1; }
ok()   { printf 'PASS  %s\n' "$*"; }

[ -f "$GUARD" ] || { echo "git-guard.spec: 缺 $GUARD" >&2; exit 1; }
command -v node >/dev/null 2>&1 || { echo "git-guard.spec: 需要 node" >&2; exit 1; }
command -v git  >/dev/null 2>&1 || { echo "git-guard.spec: 需要 git"  >&2; exit 1; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/git-guard-spec.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

new_repo() { # $1 = 目录名
  local dir="$WORK/$1"
  mkdir -p "$dir/scripts"
  cp "$GUARD" "$dir/scripts/git-guard.mjs"
  (
    cd "$dir" || exit 1
    git init -q
    git config user.email spec@example.com
    git config user.name spec
    git config commit.gpgsign false
    printf 'initial\n' > .keep
    git add .keep
    git commit -qm "chore: init" >/dev/null 2>&1
  )
  printf '%s' "$dir"
}

# ---------------------------------------------------------------
# A. pre-commit 暂存卫生
# ---------------------------------------------------------------
A="$(new_repo precommit)"
mkdir -p "$A/target/debug"
printf 'bin\n' > "$A/target/debug/artifact.txt"
( cd "$A" && git add -f target/debug/artifact.txt )
if ( cd "$A" && node scripts/git-guard.mjs pre-commit ) >/dev/null 2>&1; then
  fail "A 负样本：暂存 target/ 下产物未被拦截"
else
  ok "A 负样本：暂存构建产物被拦截"
fi

A2="$(new_repo precommit-extra)"
mkdir -p "$A2/target-perf" "$A2/.venv"
printf 'perf\n' > "$A2/target-perf/out.bin"
printf 'venv\n' > "$A2/.venv/pyvenv.cfg"
( cd "$A2" && git add -f target-perf/out.bin .venv/pyvenv.cfg )
if ( cd "$A2" && node scripts/git-guard.mjs pre-commit ) >/dev/null 2>&1; then
  fail "A2 负样本：暂存 target-perf / .venv 未被拦截"
else
  ok "A2 负样本：暂存 target-perf / .venv 被拦截"
fi

B="$(new_repo newline)"
printf 'no trailing newline' > "$B/bad.md"
( cd "$B" && git add bad.md )
if ( cd "$B" && node scripts/git-guard.mjs pre-commit ) >/dev/null 2>&1; then
  fail "B 负样本：缺尾随换行的文本文件未被拦截"
else
  ok "B 负样本：缺尾随换行被拦截"
fi

# B2：先暂存无尾随换行的文件，随后只在工作区修复（未 re-stage）→ 仍必须拦截暂存区脏字节
printf 'fixed in working tree only\n' > "$B/bad.md"
if ( cd "$B" && node scripts/git-guard.mjs pre-commit ) >/dev/null 2>&1; then
  fail "B2 负样本：工作区已修但暂存区仍缺尾随换行未被拦截"
else
  ok "B2 负样本：只验暂存区 blob，工作区未暂存修复不绕过"
fi

C="$(new_repo clean)"
printf 'clean file\n' > "$C/good.txt"
( cd "$C" && git add good.txt )
if ( cd "$C" && node scripts/git-guard.mjs pre-commit ) >/dev/null 2>&1; then
  ok "C 正样本：合法暂存内容放行"
else
  fail "C 正样本：合法暂存内容被误拒"
fi

# C2：暂存合法文件后在工作区破坏尾随换行（未暂存）→ 不误拒已暂存的合法字节
printf 'broken in working tree only' > "$C/good.txt"
if ( cd "$C" && node scripts/git-guard.mjs pre-commit ) >/dev/null 2>&1; then
  ok "C2 正样本：暂存区合法时不被工作区未暂存改动误拒"
else
  fail "C2 正样本：暂存区合法却因工作区内容被误拒"
fi

# ---------------------------------------------------------------
# D. commit-msg 规范
# ---------------------------------------------------------------
printf '随手改点东西\n' > "$WORK/msg-bad.txt"
if ( cd "$C" && node scripts/git-guard.mjs commit-msg "$WORK/msg-bad.txt" ) >/dev/null 2>&1; then
  fail "D 负样本：非 Conventional 提交信息未被拦截"
else
  ok "D 负样本：非法提交信息被拦截"
fi

printf 'fix(tui): 修复焦点丢失\n' > "$WORK/msg-good.txt"
if ( cd "$C" && node scripts/git-guard.mjs commit-msg "$WORK/msg-good.txt" ) >/dev/null 2>&1; then
  ok "D 正样本：Conventional 提交信息放行"
else
  fail "D 正样本：合法提交信息被误拒"
fi

printf 'Merge branch main into feat/x\n' > "$WORK/msg-merge.txt"
if ( cd "$C" && node scripts/git-guard.mjs commit-msg "$WORK/msg-merge.txt" ) >/dev/null 2>&1; then
  ok "D 正样本：git 自动生成的 Merge 信息放行"
else
  fail "D 正样本：Merge 信息被误拒（会与 git 自洽性对抗）"
fi

# ---------------------------------------------------------------
# E. pre-push 推送纪律（stdin 模拟 git 的三列/四列协议）
# ---------------------------------------------------------------
P="$(new_repo prepush)"
HEAD_SHA="$( cd "$P" && git rev-parse HEAD )"
ZERO="0000000000000000000000000000000000000000"

push_case() { # $1=localRef $2=localSha $3=remoteRef $4=remoteSha → 返回 git-guard 退出码
  printf '%s %s %s %s\n' "$1" "$2" "$3" "$4" | ( cd "$P" && node scripts/git-guard.mjs pre-push ) >/dev/null 2>&1
}

if push_case refs/heads/main "$HEAD_SHA" refs/heads/main "$ZERO"; then
  fail "E 负样本：直接推送 main 未被拦截"
else
  ok "E 负样本：直推 main 被拦截"
fi

if push_case refs/heads/feat/git-flow "$HEAD_SHA" refs/heads/feat/git-flow "$ZERO"; then
  ok "E 正样本：合法特性分支推送放行"
else
  fail "E 正样本：合法特性分支被误拒"
fi

if push_case refs/heads/nakedname "$HEAD_SHA" refs/heads/nakedname "$ZERO"; then
  fail "E 负样本：无类型前缀的分支名未被拦截"
else
  ok "E 负样本：无类型前缀分支名被拦截"
fi

# 历史改写：远端 sha 存在但不是本地 HEAD 的祖先（模拟 force push 覆盖他人提交）
( cd "$P" && printf 'x\n' > x.txt && git add x.txt && git commit -qm "chore: second" >/dev/null 2>&1 )
REWRITE_SHA="$( cd "$P" && git rev-parse HEAD )"
( cd "$P" && git reset -q --hard HEAD~1 )
if push_case refs/heads/feat/git-flow "$HEAD_SHA" refs/heads/feat/git-flow "$REWRITE_SHA"; then
  fail "E 负样本：改写远端既有历史未被拦截"
else
  ok "E 负样本：改写远端既有历史被拦截"
fi

# 逃生门：灾备场景显式放行 main（否则门太硬，会把运维逼到 --no-verify）
if printf 'refs/heads/main %s refs/heads/main %s\n' "$HEAD_SHA" "$ZERO" \
   | ( cd "$P" && PONY_GUARD_ALLOW_MAIN_PUSH=1 node scripts/git-guard.mjs pre-push ) >/dev/null 2>&1; then
  ok "E 正样本：显式逃生门放行 main 直推（灾备路径）"
else
  fail "E 正样本：逃生门生效失败（main 直推无法在灾备场景执行）"
fi

# 保护分支与已发布 tag 删除必须被拦截；普通特性分支删除允许放行
if push_case "(delete)" "$ZERO" refs/heads/main "$HEAD_SHA"; then
  fail "E2 负样本：删除 main 分支未被拦截"
else
  ok "E2 负样本：删除 main 分支被拦截"
fi

if push_case "(delete)" "$ZERO" refs/tags/v0.1.200 "$HEAD_SHA"; then
  fail "E3 负样本：删除远端 tag 未被拦截"
else
  ok "E3 负样本：删除远端 tag 被拦截"
fi

if push_case "(delete)" "$ZERO" refs/heads/feat/done "$HEAD_SHA"; then
  ok "E4 正样本：删除普通特性分支放行"
else
  fail "E4 正样本：删除普通特性分支被误拒"
fi

if push_case refs/tags/v0.1.200 "$HEAD_SHA" refs/tags/v0.1.200 "$ZERO"; then
  ok "E5 正样本：新建发版 tag 推送放行"
else
  fail "E5 正样本：新建发版 tag 推送被误拒"
fi

if [ "$FAIL" -ne 0 ]; then
  echo "git-guard.spec: 有断言失败" >&2
  exit 1
fi
echo "git-guard.spec: 全部通过"
exit 0
