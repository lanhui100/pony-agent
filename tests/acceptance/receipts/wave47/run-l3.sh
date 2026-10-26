#!/usr/bin/env bash
# Wave-47 L3 runner：显式配置 ingest 端点，证明「零出网」不是空端点早退的假象。
set -euo pipefail
cd "$(dirname "$0")/../../../.."
export VITE_PONYSENTRY_INGEST_URL="http://127.0.0.1:${WAVE47_INGEST_PORT:-39123}"
export VITE_PONYSENTRY_CLIENT_TOKEN="wave47-local-probe-token"
exec npx vitest run tests/acceptance/wave47/ob2-error-boundary-realpath.spec.ts --reporter=verbose "$@"
