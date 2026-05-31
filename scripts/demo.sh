#!/usr/bin/env bash
# demo.sh — agentwiki 프로토타입을 사람이 눈으로 보는 데모.
#
# 임시 디렉터리에 한국어 커널 노트를 몇 개 만들고, append-only 추가,
# 검색, KV, 세션 다이제스트까지 실제 출력을 보여준다.
#
# 사용법: ./scripts/demo.sh   (먼저 `cargo build` 되어 있어야 함)

set -euo pipefail
REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
AW="$REPO_DIR/target/debug/aw"
[[ -x "$AW" ]] || { echo "먼저 'cargo build' 를 실행하세요"; exit 1; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
export HOSTNAME="${HOSTNAME:-demo-host}"
run() { echo "+ aw ${*}"; "$AW" --root "$WORK" "$@"; echo; }

echo "### 1. 커널 지식 노트 작성 (한국어)"
run note mm  "mmap_lock 규약"  "do_mmap 진입 시 호출자가 mmap_write_lock을 보유해야 한다" gotcha high
run note sched "런큐 락 순서"   "rq->lock 은 항상 task->pi_lock 다음에 잡는다" gotcha high
run note net "skb 할당 컨텍스트" "skb 할당은 softirq 에서 일어날 수 있어 GFP_ATOMIC 사용" concept med

echo "### 2. 같은 노트에 사실 추가 (append-only, D13)"
ID=$("$AW" --root "$WORK" search mmap_write_lock | head -1 | awk '{print $1}')
run note mm "mmap_lock 규약" "예외: nommu(CONFIG_MMU=n) 빌드에서는 규약이 다르다" gotcha med "$ID"
echo "--- 누적된 노트 ($ID) ---"
"$AW" --root "$WORK" get "$ID"; echo

echo "### 3. 검색 (영문 심볼 / 2글자 한국어 폴백 / 서브시스템 한정)"
run search pi_lock
run search "규약"
run search "할당" net

echo "### 4. 빠른 사실 KV (스코프 우선순위 D22)"
run kv-set src /home/me/linux "machine:$HOSTNAME"
run kv-set src /srv/linux global
run kv-get src

echo "### 5. 세션 시작용 최소 다이제스트 (D11 — 목차만, 본문 없음)"
run digest

echo "데모 완료. (작업 디렉터리 $WORK 는 정리됨)"
