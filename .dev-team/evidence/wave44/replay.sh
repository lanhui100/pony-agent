#!/usr/bin/env bash
# wave44 可回放采集脚本（只读）。输出每步 EXIT= 与关键数字。
set -uo pipefail
NOW=$(date +%s); S7=$((NOW-604800)); S24=$((NOW-86400))
T=http://127.0.0.1:13200; L=http://127.0.0.1:13100; P=http://127.0.0.1:19090
r(){ echo "----- $* -----"; }
r "0. audit-project.sh (auto-detect)"; bash /home/dm/.agents/skills/observability-onboarding/scripts/audit-project.sh /home/dm/pony-agent --json >/tmp/a1.json 2>/dev/null; echo "EXIT=$?"; python3 -c "import json;d=json.load(open('/tmp/a1.json'));print('summary',d['summary'])"
r "0b. audit-project.sh --service desktop-client --project pony-agent"; bash /home/dm/.agents/skills/observability-onboarding/scripts/audit-project.sh /home/dm/pony-agent --service desktop-client --project pony-agent --json >/tmp/a2.json 2>/dev/null; echo "EXIT=$?"; python3 -c "import json;d=json.load(open('/tmp/a2.json'));print('summary',d['summary']);print('static_missing',d['static_missing'],'dynamic_missing',d['dynamic_missing'])"
r "1. prom count(up) / sum(up==0)"; for q in 'count(up)' 'sum(up==0)'; do printf '%s => ' "$q"; curl -s -G "$P/api/v1/query" --data-urlencode "query=$q" | python3 -c "import sys,json;print(json.load(sys.stdin)['data']['result'][0]['value'][1])"; done; echo "EXIT=$?"
r "2. prom down targets"; curl -s -G "$P/api/v1/query" --data-urlencode 'query=up==0' | python3 -c "import sys,json;[print(x['metric']) for x in json.load(sys.stdin)['data']['result']]"; echo "EXIT=$?"
r "3. prom any desktop-client/pony-agent job"; curl -s -G "$P/api/v1/query" --data-urlencode 'query=up{job=~".*(desktop|pony-agent).*"}' | python3 -c "import sys,json;print('n=',len(json.load(sys.stdin)['data']['result']))"
r "4. prom top10 scrape_samples_scraped"; curl -s -G "$P/api/v1/query" --data-urlencode 'query=topk(10,sum by(job)(scrape_samples_scraped))' | python3 -c "import sys,json;[print(x['metric'].get('job'),x['value'][1]) for x in json.load(sys.stdin)['data']['result']]"
r "5. prom otelcol_* series count"; curl -s -G "$P/api/v1/query" --data-urlencode 'query=count({__name__=~"otelcol_.*"})' | python3 -c "import sys,json;r=json.load(sys.stdin)['data']['result'];print('n=',len(r), r[0]['value'][1] if r else 'ABSENT')"
r "6. tempo 7d/24h desktop-client (TraceQL search)"; for W in $S7 $S24; do printf 'window=%ss => ' "$((NOW-W))"; curl -s -G "$T/api/search" --data-urlencode 'q={ resource.service.name = "desktop-client" }' --data-urlencode "start=$W" --data-urlencode "end=$NOW" --data-urlencode 'limit=1000' | python3 -c "import sys,json;print('traces=',len(json.load(sys.stdin).get('traces') or []))"; done
r "7. tempo 7d error counts"; for q in '{ status = error }' '{ resource.service.name = "desktop-client" && status = error }' '{ resource.service.name = "desktop-client" && span.pony.trace.route = "genai" }' '{ span.pony.trace.route = "genai" }'; do printf '%-72s => ' "$q"; curl -s -G "$T/api/search" --data-urlencode "q=$q" --data-urlencode "start=$S7" --data-urlencode "end=$NOW" --data-urlencode 'limit=1000' | python3 -c "import sys,json;print(len(json.load(sys.stdin).get('traces') or []))"; done
r "8. tempo client.service.name distinct values (7d)"; curl -s -G "$T/api/v2/search/tag/resource.client.service.name/values" --data-urlencode "start=$S7" --data-urlencode "end=$NOW" | python3 -c "import sys,json;v=[x['value'] for x in json.load(sys.stdin)['tagValues']];print('n=',len(v),'pony-agent in list:', 'pony-agent' in v)"
r "9. loki namespace values"; curl -s "$L/loki/api/v1/label/namespace/values"; echo
r "10. loki 7d line counts"; for q in 'sum(count_over_time({namespace=~".+"} |= "desktop-client" [7d]))' 'sum(count_over_time({namespace=~".+"} |= "pony-agent" [7d]))'; do printf '%-70s => ' "$q"; curl -s -G "$L/loki/api/v1/query" --data-urlencode "query=$q" | python3 -c "import sys,json;r=json.load(sys.stdin)['data']['result'];print(r[0]['value'][1] if r else 0)"; done
r "11. ponysentry admin routes"; PID=$(pgrep -f pony-sentry-server|head -1); TOK=$(tr '\0' '\n' < /proc/$PID/environ | awk -F= '/^ADMIN_TOKEN=/{print $2}')
echo "token_present=$([ -n "${TOK:-}" ] && echo yes || echo no) (value never printed)"; echo "EXIT=$?"
for u in /api/v1/projects '/api/v1/issues?project=pony-agent&limit=100'; do printf '%-42s HTTP=' "$u"; curl -s -o /tmp/ps.json -w "%{http_code}\n" -H "Authorization: Bearer $TOK" "http://127.0.0.1:3000$u"; done
python3 -c "
import json;d=json.load(open('/tmp/ps.json'));import collections
print('issues=',len(d),'by_status=',dict(collections.Counter(x['status'] for x in d)),'sum_count=',sum(x['count'] for x in d))"
r "12. ponysentry events per issue cap"; ID=$(curl -s -H "Authorization: Bearer $TOK" 'http://127.0.0.1:3000/api/v1/issues?project=pony-agent&limit=100' | python3 -c "import sys,json;print([x['id'] for x in json.load(sys.stdin) if x['count']>20][0])"); for lim in 5 500; do printf "limit=%s rows=" $lim; curl -s -H "Authorization: Bearer $TOK" "http://127.0.0.1:3000/api/v1/issues/$ID/events?limit=$lim" | python3 -c "import sys,json;print(len(json.load(sys.stdin)))"; done
r "13. ponysentry /metrics counters (host)"; curl -s http://127.0.0.1:3000/metrics | grep -E "^pony_sentry_(events_total|issues_total\{status=\"unresolved\",environment=\"dev\")"
r "14. langfuse clickhouse 7d"; kubectl -n observability exec clickhouse-0 -- clickhouse-client -q "select count() traces, uniqExact(project_id) proj, min(timestamp), max(timestamp) from default.traces where timestamp > now() - interval 7 day"; echo "EXIT=$?"
kubectl -n observability exec clickhouse-0 -- clickhouse-client -q "select type, count() from default.observations where start_time > now() - interval 7 day group by type"
r "15. otel-collector genai pipeline in logs"; kubectl -n observability logs deploy/otel-collector --tail=200 2>&1 | grep -c genai; echo "EXIT=$?"
