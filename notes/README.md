# notes/ — 지식 위키 (진실의 원천)

이 디렉터리의 마크다운 파일이 지식의 **진실의 원천**(D4)입니다. SQLite 인덱스
(`.aw/index.db`)는 여기서 재생성되는 파생물이므로 git에 올리지 않습니다(D5).

## 레이아웃 (D10)

```
notes/<subsystem>/<id>.md
```

- `<subsystem>` — 리눅스 커널 서브시스템(`mm`, `sched`, `net`, `fs`, ...). 검색
  범위를 좁히는 데 쓰입니다(`wiki op=search subsystem=mm`).
- `<id>` — `<subsystem>-<짧은해시>` (D21). 해시는 제목+본문에서 파생되어
  멀티머신 충돌을 피합니다.

## 노트 형식

- YAML frontmatter + append-only 엔트리 로그 본문(D13).
- 본문 엔트리는 `## <RFC3339> · host=<h> · confidence=<c>` 헤더로 구분합니다.
- 새 사실은 **기존 줄 수정이 아니라 새 엔트리 append**로 추가합니다.

frontmatter 필드:

| 필드 | 설명 |
|---|---|
| `id` | 노트 ID(파일명과 일치) |
| `title` | 제목 |
| `type` | `concept`/`gotcha`/`decision`/`howto`/`symbol-note` |
| `subsystem` | 서브시스템(디렉터리와 일치) |
| `tags` | 태그 목록 |
| `code_refs` | 심볼 닻 `{sym, file, sig_hash}` (라인 아님, D14/D21) |
| `confidence` | `high`/`med`/`low` |

예시는 [`mm/mm-example.md`](mm/mm-example.md) 참조.

> 멀티머신 운용 시 이 `notes/` 트리를 별도 데이터 레포(`agentwiki-data`)로 두고
> git 동기화하는 것을 권장합니다(D20). 여기 포함된 예시는 형식 참고용입니다.
