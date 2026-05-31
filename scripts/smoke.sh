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
check "tools/list exposes code"        '"name":"code"'      "$mcp_out"
check "tools/list exposes recall"      '"name":"recall"'    "$mcp_out"
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
head "code index (M2: ctags/cscope)"
if command -v ctags >/dev/null 2>&1; then
  CSRC="$(mktemp -d)"
  CDATA="$(mktemp -d)"
  trap 'rm -rf "$WORK" "$MWORK" "$CSRC" "$CDATA"' EXIT
  cat > "$CSRC/mmap.c" <<'EOF'
int helper(int x) { return x + 1; }
int do_mmap(struct file *f, unsigned long addr)
{
	return helper(addr);
}
void caller(void) { do_mmap(0, 4096); }
EOF
  out=$("$AW" --root "$CDATA" code-reindex "$CSRC" 2>&1)
  check "ctags reindex finds symbols" "indexed" "$out"

  out=$("$AW" --root "$CDATA" where do_mmap "$CSRC" 2>&1)
  check "where locates definition"    "mmap.c" "$out"
  check "where reports function kind"  "[function]" "$out"

  out=$("$AW" --root "$CDATA" show do_mmap "$CSRC" 2>&1)
  check "show slices target body"      "helper(addr)" "$out"
  if [[ "$out" == *"int helper(int x)"* ]]; then
    bad "show must NOT include other functions (slicing)"
  else
    ok "show excludes other functions (function-level slicing)"
  fi

  # 심볼 닻 stale 감지(D14): 잘못된 sig_hash 노트를 만들고 recall.
  mkdir -p "$CDATA/notes/mm"
  cat > "$CDATA/notes/mm/mm-anchor.md" <<'EOF'
---
id: mm-anchor
title: do_mmap 락 규약
type: gotcha
subsystem: mm
tags: [locking]
code_refs:
  - sym: do_mmap
    file: mmap.c
    sig_hash: "DEADBEEF"
confidence: high
---
## 2026-01-01T00:00:00Z · host=test · confidence=high
do_mmap 호출 전 락 필요.
EOF
  out=$("$AW" --root "$CDATA" recall do_mmap 2>&1)
  check "recall links code def"        "definition: " "$out"
  check "recall links anchored note"   "mm-anchor" "$out"
  check "recall flags stale anchor"    "STALE" "$out"

  if command -v cscope >/dev/null 2>&1; then ok "cscope available"; else
    printf '  %sSKIP%s cscope not installed\n' "$YELLOW" "$RESET"; fi
else
  printf '  %sSKIP%s ctags not installed — M2 code-index checks skipped\n' "$YELLOW" "$RESET"
fi

# ---------------------------------------------------------------------------
head "hybrid search & compaction (M4)"
HWORK="$(mktemp -d)"
trap 'rm -rf "$WORK" "$MWORK" "$CSRC" "$CDATA" "$HWORK"' EXIT
# 노트 생성 출력에서 id를 직접 캡처(검색 의존 제거).
cre=$("$AW" --root "$HWORK" note mm "mmap 락 규약" "do_mmap 진입 시 mmap_write_lock 보유 필요" gotcha high 2>/dev/null)
ID=$(printf '%s' "$cre" | awk '/created/ {print $2}')
"$AW" --root "$HWORK" note net "소켓 버퍼" "skb 할당은 softirq 컨텍스트" concept med >/dev/null 2>&1

# 관련 질의는 mm 노트를 찾는다(FTS + 벡터 융합).
out=$("$AW" --root "$HWORK" search "mmap 보유" 2>/dev/null)
check "hybrid search finds related note" "mmap 락 규약" "$out"

# 완전 무관 질의는 임계값에 걸려 끌려오지 않는다(정밀도).
out=$("$AW" --root "$HWORK" search "자바스크립트 프론트엔드 렌더링" 2>/dev/null)
if [[ -z "$out" || "$out" != *"mmap"* ]]; then
  ok "vector threshold filters unrelated query"
else
  bad "unrelated query should not match mm note"
fi

# compact: 동일 본문 append → 중복 → 제거(D13).
"$AW" --root "$HWORK" note mm "mmap 락 규약" "do_mmap 진입 시 mmap_write_lock 보유 필요" gotcha high "$ID" >/dev/null 2>&1
before=$("$AW" --root "$HWORK" get "$ID" 2>/dev/null | grep -c '^## ')
out=$("$AW" --root "$HWORK" compact 2>/dev/null)
after=$("$AW" --root "$HWORK" get "$ID" 2>/dev/null | grep -c '^## ')
check "compact removes duplicate entry" "removed 1" "$out"
if [[ "$before" == "2" && "$after" == "1" ]]; then
  ok "compact: 2 entries -> 1 (dedup)"
else
  bad "compact entry count (before=$before after=$after)"
fi

# ---------------------------------------------------------------------------
head "regression: parsing & ordering bugs"
RWORK="$(mktemp -d)"
trap 'rm -rf "$WORK" "$MWORK" "$CSRC" "$CDATA" "$HWORK" "$RWORK"' EXIT

# 본문 속 마크다운 헤딩이 엔트리로 오분리되지 않아야(버그1 회귀).
# 엔트리 헤더는 타임스탬프(YYYY-MM-DDT)로 시작하므로 그 라인만 센다.
cre=$("$AW" --root "$RWORK" note mm "헤딩테스트" $'본문 시작\n\n## 주의\n헤딩 본문' gotcha high 2>/dev/null)
RID=$(printf '%s' "$cre" | awk '/created/ {print $2}')
ecount=$("$AW" --root "$RWORK" get "$RID" 2>/dev/null | grep -cE '^## [0-9]{4}-[0-9]{2}-[0-9]{2}T')
if [[ "$ecount" == "1" ]]; then ok "body markdown heading not split into entry"; else
  bad "markdown heading split (timestamp entries=$ecount, expected 1)"; fi

# 같은 제목, 다른 본문 → 같은 노트로 append(버그2 회귀).
"$AW" --root "$RWORK" note net "토픽A" "사실 하나" concept med >/dev/null 2>&1
"$AW" --root "$RWORK" note net "토픽A" "사실 둘 다른 내용" concept med >/dev/null 2>&1
nfiles=$(find "$RWORK/notes/net" -name '*.md' 2>/dev/null | wc -l | tr -d ' ')
if [[ "$nfiles" == "1" ]]; then ok "same topic appends to one note"; else
  bad "same topic forked into $nfiles files"; fi

# digest 정렬이 reindex 후에도 최신순 유지(버그4 회귀).
"$AW" --root "$RWORK" reindex >/dev/null 2>&1
dg=$("$AW" --root "$RWORK" digest 2>/dev/null)
check "digest survives reindex (non-empty)" "notes total" "$dg"

# ---------------------------------------------------------------------------
head "result"
printf '%s%d passed%s, %s%d failed%s\n' "$GREEN" "$PASS" "$RESET" \
  "$([[ $FAIL -gt 0 ]] && echo "$RED" || echo "$GREEN")" "$FAIL" "$RESET"
[[ $FAIL -eq 0 ]]
