#!/usr/bin/env bash
# wave35_flake_masking_probe.sh — 判定 freeze-2 首次全量那例 flake 与本波 C3 分类修复的因果关系
#
# 假说:失败形态是「首个请求 1ms connect refused 到 http://127.0.0.1:42967/...」。
#   freeze-1 之前:URL 未被剥离 ⇒ 端口 `42967` 里的 `429` 命中 RATE_LIMIT 码表 ⇒ 被判为可重试
#   ⇒ 重试(带退避)给了 mock server 绑端口的时间 ⇒ 测试反而"稳过"(缺陷掩盖了启动竞态)。
#   freeze-2 之后:URL 片段被剥离 ⇒ 该 message 不再含 429 ⇒ NonRetryable ⇒ 立刻 Abort
#   ⇒ 同一启动竞态被暴露为硬失败。
#
# 方法:逐字提取 retry.rs 的 `strip_endpoint_fragments` 与 `RATE_LIMIT_LONG_BACKOFF_HINTS`,
#       对「flake 日志里的原始 details 文本」分别计算 剥离前/剥离后 是否命中限流码表。
set -uo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
RETRY="${1:-$REPO_ROOT/crates/pony-agent-core/src/agent/retry.rs}"
FLAKE_LOG="${2:-$REPO_ROOT/tests/acceptance/receipts/wave35/full_lib_run_freeze2.log}"
OUT_DIR="$REPO_ROOT/tests/acceptance/receipts/wave35"
STAMP="$(date +%Y%m%dT%H%M%S)"
CRATE="$(mktemp -d /tmp/w35flakeprobe-XXXXXX)"
LOG="$OUT_DIR/flake_masking_probe_${STAMP}.log"
trap 'rm -rf "$CRATE"' EXIT

{
  echo "# flake masking probe @ $(date -Is)"
  echo "# retry.rs sha256 = $(sha256sum "$RETRY" | cut -d' ' -f1)"
  echo "# flake log = $FLAKE_LOG (sha256 $(sha256sum "$FLAKE_LOG" 2>/dev/null | cut -d' ' -f1))"
  echo "# HEAD = $(cd "$REPO_ROOT" && git rev-parse HEAD)"
} > "$LOG"

mkdir -p "$CRATE/src"
cat > "$CRATE/Cargo.toml" <<'TOML'
[package]
name = "w35flakeprobe"
version = "0.0.0"
edition = "2021"
[dependencies]
TOML

sed -n '/^fn strip_endpoint_fragments/,/^}/p' "$RETRY" > "$CRATE/fn.rs"
sed -n '/^pub(crate) const RATE_LIMIT_LONG_BACKOFF_HINTS/,/^];/p' "$RETRY" > "$CRATE/hints.rs"
{
  cat "$CRATE/fn.rs"; echo; cat "$CRATE/hints.rs"; cat <<'EOF'

fn hints_match(lower: &str, hints: &[&str]) -> bool {
    hints.iter().any(|hint| lower.contains(hint))
}

#[cfg(test)]
mod probe {
    use super::*;

    /// 从 flake 收据里取出的原始失败 details(与日志逐字一致)
    const FLAKE_DETAILS: &str = "调用 provider 失败:error sending request for url (http://127.0.0.1:42967/v1/chat/completions);type=connect+request;elapsed=1ms;url=http://127.0.0.1:42967/v1/chat/completions;source=client error (Connect)";

    #[test]
    fn pre_fix_behaviour_raw_text_hits_rate_limit() {
        let raw = FLAKE_DETAILS.to_ascii_lowercase();
        let hit = hints_match(&raw, RATE_LIMIT_LONG_BACKOFF_HINTS);
        println!("PRE_FIX raw_hits_rate_limit={hit} (raw 含 429? {})", raw.contains("429"));
        assert!(hit, "freeze-1(未剥离 URL)下该 message 应命中限流码表 ⇒ 判为可重试");
    }

    #[test]
    fn post_fix_behaviour_stripped_text_misses_rate_limit() {
        let raw = FLAKE_DETAILS.to_ascii_lowercase();
        let stripped = strip_endpoint_fragments(&raw);
        let hit = hints_match(&stripped, RATE_LIMIT_LONG_BACKOFF_HINTS);
        println!("POST_FIX stripped_hits_rate_limit={hit}");
        println!("POST_FIX stripped_text={stripped}");
        assert!(!hit, "freeze-2(剥离 URL)下不再命中 ⇒ NonRetryable ⇒ 立即 Abort");
        assert!(!stripped.contains("429"), "剥离后不应残留端口数字");
    }
}
EOF
} > "$CRATE/src/lib.rs"

( cd "$CRATE" && cargo test -- --nocapture 2>&1 ) >> "$LOG"
RC=$?
echo "EXIT=$RC" >> "$LOG"
echo "=== 结果 ==="; grep -E 'PRE_FIX|POST_FIX|test result|panicked' "$LOG" | head -8
echo "" >> "$LOG"
if [ "$RC" -eq 0 ]; then
  echo "MASKING: CONFIRMED —— 同一 message 在 freeze-1 判可重试(掩盖启动竞态)、freeze-2 判不可重试(暴露该竞态)" | tee -a "$LOG"
else
  echo "MASKING: NOT CONFIRMED —— 见日志" | tee -a "$LOG"
fi
echo "raw: $LOG"