#!/usr/bin/env bash
# smoke.sh — agentwiki 프로토타입 end-to-end 스모크 테스트.
#
# 빌드 → 단위테스트 → CLI 흐름 → MCP 서버 JSON-RPC 라운드트립을 모두 검증한다.
# 네트워크/모델 다운로드 없이 동작한다(FTS5 단독). 종료코드 0 = 전부 통과.
#
# 사용법:
#   ./scripts/smoke.sh            # 전체
#   ./scripts/smoke.sh --no-build # 빌드 생략(이미 빌드된 바이너리 사용)

set -uo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR"

PASS=0
FAIL=0
RED=$'\033[31m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; RESET=$'\033[0m'

ok()   { PASS=$((PASS+1)); printf '  %sPASS%s %s\n' "$GREEN" "$RESET" "$1"; }
bad()  { FAIL=$((FAIL+1)); printf '  %sFAIL%s %s\n' "$RED" "$RESET" "$1"; }
head() { printf '\n%s== %s ==%s\n' "$YELLOW" "$1" "$RESET"; }

# check <description> <expected-substring> <actual>
check() {
  local desc="$1" expect="$2" actual="$3"
  if [[ "$actual" == *"$expect"* ]]; then
    ok "$desc"
  else
    bad "$desc"
    printf '       expected to contain: %q\n' "$expect"
    printf '       got: %q\n' "${actual:0:300}"
  fi
}

# ---------------------------------------------------------------------------
head "build & unit tests"
if [[ "${1:-}" != "--no-build" ]]; then
  if cargo build --workspace >/tmp/aw_build.log 2>&1; then ok "cargo build"; else bad "cargo build (see /tmp/aw_build.log)"; fi
  if cargo test --workspace >/tmp/aw_test.log 2>&1; then
    total=$(grep -oE '[0-9]+ passed' /tmp/aw_test.log | awk '{s+=$1} END{print s}')
    ok "cargo test (${total:-0} passed)"
  else
    bad "cargo test (see /tmp/aw_test.log)"
  fi
fi

AW="$REPO_DIR/target/debug/aw"
MCP="$REPO_DIR/target/debug/aw-mcp"
[[ -x "$AW" ]]  || { bad "aw binary missing — run without --no-build"; }
[[ -x "$MCP" ]] || { bad "aw-mcp binary missing — run without --no-build"; }

export HOSTNAME=smoke-host

# ---------------------------------------------------------------------------
head "CLI flow"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

out=$("$AW" --root "$WORK" note mm "mmap_lock 규약" \
  "do_mmap 진입 시 호출자가 mmap_write_lock을 보유해야 한다" gotcha high 2>&1)
check "create Korean note"        "created mm-" "$out"
ID=$(printf '%s' "$out" | awk '{print $2}')

out=$("$AW" --root "$WORK" note net "skb 할당" \
  "skb 할당은 softirq 컨텍스트에서 일어날 수 있다" concept med 2>&1)
check "create second note"        "created net-" "$out"

# append-only(D13): 같은 id로 추가
out=$("$AW" --root "$WORK" note mm "mmap_lock 규약" \
  "예외: nommu 빌드에서는 규약이 다르다" gotcha med "$ID" 2>&1)
check "append to existing note"   "appended $ID" "$out"

entries=$("$AW" --root "$WORK" get "$ID" 2>/dev/null | grep -c '^## ')
check "append produced 2 entries" "2" "$entries"

# 전문 검색 (영문 심볼)
out=$("$AW" --root "$WORK" search mmap_write_lock 2>&1)
check "FTS search by symbol"      "$ID" "$out"

# 2글자 한국어 검색 (trigram 폴백)
out=$("$AW" --root "$WORK" search "규약" 2>&1)
check "short Korean query (LIKE fallback)" "mmap_lock" "$out"

# 서브시스템 한정 검색
out=$("$AW" --root "$WORK" search "할당" net 2>&1)
check "subsystem-scoped search"   "net-" "$out"

# KV 스코프 우선순위(D22): machine > global
"$AW" --root "$WORK" kv-set src /global/linux global >/dev/null 2>&1
"$AW" --root "$WORK" kv-set src /home/smoke/linux "machine:smoke-host" >/dev/null 2>&1
out=$("$AW" --root "$WORK" kv-get src 2>&1)
check "KV machine-scope precedence" "/home/smoke/linux" "$out"

# 비밀 스캔 경고만(D15) — 차단하지 않고 노트는 생성
out=$("$AW" --root "$WORK" note misc creds "token: AKIAIOSFODNN7EXAMPLE" howto low 2>&1)
check "secret scan warns"         "secret scan warnings" "$out"
check "secret scan does not block" "created misc-" "$out"

# 다이제스트(D11)
out=$("$AW" --root "$WORK" digest 2>&1)
check "digest lists notes"        "notes total" "$out"

# ---------------------------------------------------------------------------
head "MCP server (JSON-RPC over stdio)"
MWORK="$(mktemp -d)"
trap 'rm -rf "$WORK" "$MWORK"' EXIT

# 한 번의 세션으로 여러 요청을 보내고 응답을 캡처한다.
mcp_out=$(AW_ROOT="$MWORK" "$MCP" 2>/dev/null <<'EOF'
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"wiki","arguments":{"op":"propose","subsystem":"mm","title":"RCU 임계영역","body":"rcu_read_lock 안에서는 잠들면 안 된다","type":"gotcha","confidence":"high"}}}
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"wiki","arguments":{"op":"commit","subsystem":"mm","title":"RCU 임계영역","body":"rcu_read_lock 안에서는 잠들면 안 된다","type":"gotcha","confidence":"high"}}}
{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"wiki","arguments":{"op":"search","query":"rcu_read_lock"}}}
{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"kv","arguments":{"op":"set","key":"build","value":"make defconfig","scope":"global"}}}
{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"kv","arguments":{"op":"get","key":"build"}}}
{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"admin","arguments":{"op":"digest"}}}
EOF
)

check "initialize returns serverInfo" '"name":"agentwiki"' "$mcp_out"
check "tools/list exposes wiki"        '"name":"wiki"'      "$mcp_out"
check "tools/list exposes kv"          '"name":"kv"'        "$mcp_out"
check "tools/list exposes admin"       '"name":"admin"'     "$mcp_out"
check "wiki propose shows draft"       'PROPOSAL id=mm-'    "$mcp_out"
check "wiki commit persists"           'committed mm-'      "$mcp_out"
check "wiki search finds committed"    'RCU'                "$mcp_out"
check "kv set+get roundtrip"           'make defconfig'     "$mcp_out"
check "admin digest works"             'notes total'        "$mcp_out"

# propose가 디스크에 쓰지 않았는지(반자동 D6): commit 전 propose만 한 새 세션
solo=$(AW_ROOT="$MWORK/solo" "$MCP" 2>/dev/null <<'EOF'
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"wiki","arguments":{"op":"propose","subsystem":"fs","title":"x","body":"y","type":"concept"}}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"admin","arguments":{"op":"reindex"}}}
EOF
)
check "propose does not persist (semi-auto)" 'reindexed 0 notes' "$solo"

# ---------------------------------------------------------------------------
head "result"
printf '%s%d passed%s, %s%d failed%s\n' "$GREEN" "$PASS" "$RESET" \
  "$([[ $FAIL -gt 0 ]] && echo "$RED" || echo "$GREEN")" "$FAIL" "$RESET"
[[ $FAIL -eq 0 ]]
