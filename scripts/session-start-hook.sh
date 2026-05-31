#!/usr/bin/env bash
# session-start-hook.sh — Claude Code SessionStart 훅 (D11).
#
# 세션 시작 시 agentwiki 의 **최소 다이제스트**(목차+진입점만, 본문 없음)를
# 컨텍스트에 주입한다. 본문은 에이전트가 필요할 때 wiki/recall 로 회수한다.
# 토큰 예산을 지키기 위해 노트 제목·id 목록과 사용 안내만 출력한다.
#
# 설치(.claude/settings.json):
#   {
#     "hooks": {
#       "SessionStart": [
#         { "hooks": [ { "type": "command",
#             "command": "AW_ROOT=/path/to/agentwiki-data /path/to/aw --root \"$AW_ROOT\" digest" } ] }
#       ]
#     }
#   }
#
# 또는 이 스크립트를 직접 호출:
#   AW_BIN=/path/to/aw AW_ROOT=/path/to/data ./scripts/session-start-hook.sh

set -uo pipefail

AW_BIN="${AW_BIN:-aw}"
AW_ROOT="${AW_ROOT:-.}"
MAX_NOTES="${AW_DIGEST_NOTES:-20}"

command -v "$AW_BIN" >/dev/null 2>&1 || { echo "(agentwiki: aw not found, skipping digest)"; exit 0; }

echo "## agentwiki 메모리 (이번 세션에서 활용)"
echo
echo "검색/회수는 MCP 도구로: wiki(op=search/get/propose/commit), code(where/show/xref),"
echo "recall(symbol), kv, admin. 거대 파일은 통째로 읽지 말고 code op=show 로 함수만 보기."
echo
"$AW_BIN" --root "$AW_ROOT" digest 2>/dev/null | sed 's/^/  /' || echo "  (다이제스트 없음 — 먼저 admin op=reindex)"
echo
echo "본문은 자동 주입하지 않음. 필요할 때 wiki op=get id=<id> 로 회수."
