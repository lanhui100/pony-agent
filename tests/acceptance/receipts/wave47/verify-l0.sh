#!/usr/bin/env bash
# Wave-47 L0：在真实工作树上验证 ob2 变更门禁的拦截力（正相违背 → 退出 2；清理后 → 退出 0）
set -uo pipefail
cd "$(dirname "$0")/../../../.."
GATE=/home/dm/.agents/skills/observability-onboarding/scripts/audit-diff.sh
PROBE1=crates/pony-agent-core/src/agent/_l0_probe_forbidden.rs
PROBE2=src/components/common/_L0ProbeBoundary.vue

cleanup() { rm -f "$PROBE1" "$PROBE2"; }
trap cleanup EXIT

echo "### [1/3] 违背 http_client 收口点（手写 traceparent 头）"
cat > "$PROBE1" <<'RS'
pub fn bad_send(req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    req.header("traceparent", "00-00000000000000000000000000000000-0000000000000000-01")
}
RS
bash "$GATE" --repo "$PWD" --decl-approved 2>&1 | grep -E "FAIL|OK"
echo "EXIT_HTTP_CLIENT=${PIPESTATUS[0]}"
rm -f "$PROBE1"

echo
echo "### [2/3] 违背 error_boundary 收口点（散落 onErrorCaptured）"
cat > "$PROBE2" <<'VUE'
<script setup lang="ts">
import { onErrorCaptured } from "vue";
onErrorCaptured(() => {});
</script>
VUE
bash "$GATE" --repo "$PWD" --decl-approved 2>&1 | grep -E "FAIL|OK"
echo "EXIT_ERROR_BOUNDARY=${PIPESTATUS[0]}"
rm -f "$PROBE2"

echo
echo "### [3/3] 清理后基线（应放行）"
bash "$GATE" --repo "$PWD" --decl-approved 2>&1 | grep -E "FAIL|OK"
echo "EXIT_CLEAN=${PIPESTATUS[0]}"
