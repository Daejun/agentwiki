# agentwiki — LLM을 위한 로컬 지식 저장소 설계

> 목표: Claude Code 같은 코딩 에이전트가 **세션이 바뀌어도 지식을 잃지 않고**,
> 세션 간 데이터를 **효율적으로 공유**하며, 특히 **리눅스 커널처럼 거대한
> 코드베이스에서 컨텍스트 낭비를 최소화**하도록 로컬에서 읽고 쓰는 지식
> 저장소(wiki + KV + 경량 DB)를 만든다.

---

## 1. 문제 정의

코딩 에이전트의 컨텍스트 윈도우는 **상태가 없다(stateless)**. 세션이 끝나거나
컨텍스트가 압축/소실되면:

1. **지식 손실** — 어렵게 알아낸 사실(이 락은 어디서 잡힌다, 이 함수는 인터럽트
   컨텍스트에서 호출된다 등)이 사라지고, 다음 세션이 똑같은 탐색을 반복한다.
2. **세션 간 공유 부재** — 한 세션의 발견을 다른 세션이 재사용할 표준 경로가 없다.
3. **컨텍스트 낭비** — 거대한 코드를 통째로 읽으며 토큰을 소모한다. 리눅스 커널은
   파일 하나가 수천 줄이고, `grep` 한 번에 수백 개 매치가 쏟아진다. 답을 얻기까지
   읽는 양이 윈도우를 압도한다.

이 세 가지는 사실 **두 종류의 문제**다:

- **(A) 코드 구조 탐색** — "X는 어디 정의됐나", "Y를 누가 호출하나". → 인덱스 문제.
- **(B) 누적된 서사적 지식** — "왜 이렇게 됐나", "함정", "결정 사항", "디버깅 인사이트".
  → 메모리/위키 문제.

기존 도구들은 둘 중 하나만 잘한다. 이 설계는 **둘을 분리하되 한 시스템에서 연결**한다.

---

## 2. 기존 프로젝트 조사 및 장단점

| 프로젝트 | 접근 | 장점 | 단점 (특히 커널 관점) |
|---|---|---|---|
| **mem0 / OpenMemory** | 대화에서 사실을 자동 추출 → 벡터DB 저장/검색 | 자동화, MCP로 Claude Code 연동 | 임베딩 인프라 필요, 검색이 퍼지(fuzzy)해 정확한 코드 사실엔 부적합, 추출 단계에서 손실/환각 |
| **Letta (MemGPT)** | 컨텍스트를 가상메모리처럼 계층화한 에이전트 런타임 | 장기 실행 에이전트에 강함 | 무겁고, 에이전트를 Letta 안에서 돌려야 함(우리는 Claude Code 안에서 쓰고 싶음) |
| **CLAUDE.md / auto memory** | 세션 시작 시 파일을 컨텍스트에 주입 | 단순, 인프라 0, 사람이 읽기 좋음 | 검색이 없어 **전부 로드** → 200줄 넘으면 토큰 폭증·준수율 저하. 무한정 커짐 |
| **claude-mem** | 세션 활동을 AI로 압축해 다음 세션에 주입 | 자동, 활동 타임라인 보존 | 손실 압축, 간접적, "정밀한 사실"보다 "분위기" 보존에 가까움 |
| **Serena (LSP + MCP)** | LSP로 심볼 단위 검색/편집 | 토큰 효율 최고, IDE급 정밀도 (whole-file read/grep 대체) | **clangd/LSP가 커널 규모에서 한계**: compile_commands.json 수백만 줄, 인덱싱 메모리·시간 폭증, jump-to-def 불안정 |
| **ctags / cscope** | 심볼·크로스레퍼런스 인덱스 (텍스트 기반) | **커널 규모로 잘 확장**(`make cscope`), 저렴·빠름 | 의미(semantic) 부족, 서사 지식 못 담음, 라인 번호가 코드 변경에 취약 |
| **basic-memory류** | Obsidian 스타일 마크다운 + MCP | git 친화·사람이 읽기 좋음·검색 가능 | 구조적 질의·코드 인덱스는 별도로 필요 |

### 조사에서 끌어낸 핵심 교훈

1. **벡터/임베딩은 코드 사실에 과하다.** 정확한 심볼/사실 회수에는 FTS(전문 검색) +
   심볼 인덱스가 더 정확하고 가볍다. 임베딩은 "비슷한 노트 찾기"의 선택적 보조로만.
2. **"전부 주입"은 안티패턴이다.** CLAUDE.md의 한계가 이를 증명한다. **검색 후
   필요한 것만 로드**해야 한다.
3. **커널에선 clangd보다 ctags/cscope가 현실적이다.** LSP는 정밀하지만 확장에서
   깨진다. ctags/cscope는 거칠지만 커널이 공식 지원한다.
4. **라인 번호는 닻으로 쓰면 안 된다.** 커널은 끊임없이 바뀐다 → **심볼 이름**에
   닻을 내려야 지식이 썩지 않는다.
5. **사람이 읽고 git으로 버전 관리되는 평문**이 가장 견고하다(인프라 0, diff 가능,
   에이전트가 평소 도구로 읽고 씀).

---

## 3. 핵심 설계 원칙

- **P1. 검색 우선, 주입 최소(Retrieval-first).** 세션 시작 시엔 작은 "다이제스트"만
  주입하고, 나머지는 질의로 필요할 때만 가져온다. 토큰 예산을 명시적으로 둔다.
- **P2. 평문이 진실의 원천(Markdown is source of truth).** 지식은 git-버전 마크다운.
  DB는 그 위의 **파생 인덱스**일 뿐이며 언제든 재생성 가능.
- **P3. 코드는 심볼에 닻을 내린다.** `file:line`이 아니라 `함수/구조체 이름`으로
  참조. 라인은 보조 메타데이터로만.
- **P4. 두 레이어를 분리.** (A) 코드 인덱스(ctags/cscope) ↔ (B) 지식 위키(마크다운).
  둘은 심볼 닻으로 연결된다.
- **P5. 정밀 회수 > 퍼지 회수.** 기본은 FTS5 + 심볼 정확 매치. 임베딩은 옵션.
- **P6. 토큰 효율을 1급 지표로.** 모든 도구는 출력에 토큰 상한을 두고, "함수 한 개만
  잘라 보기" 같은 슬라이싱을 기본 제공.

---

## 4. 아키텍처

```
┌─────────────────────────────────────────────────────────────┐
│  Claude Code 세션                                            │
│   - SessionStart hook → 컴팩트 "메모리 다이제스트" 주입       │
│   - 작업 중: agentwiki CLI / MCP 도구 호출 (검색·읽기·기록)   │
│   - /remember (Stop hook) → 새 지식을 노트로 증류             │
└───────────────┬─────────────────────────────────────────────┘
                │  CLI 또는 MCP 서버 (얇은 접근 계층)
   ┌────────────┴───────────────┐
   │                            │
┌──▼───────────────┐   ┌────────▼───────────────────────────┐
│ (B) 지식 위키     │   │ (A) 코드 인덱스                     │
│  notes/*.md       │   │  ctags(tags) + cscope(cscope.out)  │
│  (진실의 원천)    │   │  (커널 트리에서 생성, 닻 = 심볼)    │
└──┬───────────────┘   └────────┬───────────────────────────┘
   │ 파생                       │ 파생
┌──▼────────────────────────────▼───────────────────────────┐
│ index.db  (SQLite, 재생성 가능한 파생 인덱스)               │
│  - notes_fts (FTS5 전문검색)   - kv (빠른 사실)             │
│  - notes (메타·frontmatter)    - symbols (심볼→파일·종류)   │
│  - links (노트↔노트, 노트↔심볼)                            │
└────────────────────────────────────────────────────────────┘
```

**왜 하이브리드(마크다운 + SQLite)인가?**
- 순수 마크다운만: 검색이 grep뿐 → 커지면 느리고 토큰 비효율.
- 순수 DB만: 사람이 못 읽고, git diff가 무의미하고, 에이전트가 평소 도구로 못 봄.
- **하이브리드**: 마크다운으로 읽기/버전관리/신뢰성, SQLite로 빠른 정밀 검색.
  DB는 손상돼도 마크다운에서 `reindex`로 복구.

---

## 5. 데이터 모델

### 5.1 노트(마크다운) — 진실의 원천

`notes/<type>/<slug>.md`, YAML frontmatter:

```yaml
---
id: mm-do_mmap-locking          # 안정적 ID
title: do_mmap의 mmap_lock 규약
type: gotcha                    # concept | gotcha | decision | howto | symbol-note
tags: [mm, locking, mmap]
subsystem: mm                   # 커널 서브시스템
code_refs:                      # 라인 아님 — 심볼 닻
  - sym: do_mmap
    file: mm/mmap.c
  - sym: mmap_write_lock
config_deps: [CONFIG_MMU]       # 커널 config 의존
arch: [generic]                 # 또는 [x86, arm64]
confidence: high                # 지식의 확신도
source: "세션 직접 확인 / commit abc123 / LWN 기사"
updated: 2026-05-31
---

본문: 왜·함정·요약. "do_mmap 진입 시 호출자가 mmap_write_lock을 …" 등.
```

타입별 의미:
- `concept` — 서브시스템/메커니즘 개념 정리
- `gotcha` — 함정, 락 규약, 컨텍스트 제약(인터럽트/원자적 컨텍스트 등)
- `decision` — 우리 작업에서의 설계 결정과 근거
- `howto` — 빌드·디버깅·재현 절차 (예: 특정 config로 커널 빌드/부팅)
- `symbol-note` — 특정 심볼에 대한 응축 설명(서명, 역할, 호출 컨텍스트)

### 5.2 SQLite 스키마 (파생, 재생성 가능)

```sql
CREATE TABLE notes (
  id TEXT PRIMARY KEY, title TEXT, type TEXT, subsystem TEXT,
  tags TEXT, confidence TEXT, updated TEXT, path TEXT
);
CREATE VIRTUAL TABLE notes_fts USING fts5(
  id UNINDEXED, title, body, tags, content=''
);                              -- 전문 검색
CREATE TABLE kv (               -- 빠른 사실 (KV store)
  key TEXT PRIMARY KEY, value TEXT, scope TEXT, updated TEXT
);                              -- 예: build.x86.defconfig = "make ..."
CREATE TABLE symbols (          -- ctags에서 채움
  name TEXT, kind TEXT, file TEXT, line INT, signature TEXT, subsystem TEXT
);
CREATE INDEX idx_sym_name ON symbols(name);
CREATE TABLE links (            -- 노트↔노트, 노트↔심볼 그래프
  src TEXT, dst TEXT, rel TEXT  -- rel: references | relates | supersedes
);
```

---

## 6. 접근 계층 (CLI → 나중에 MCP)

처음엔 **단일 CLI**(`aw`)로 시작한다. 에이전트가 `Bash`로 호출 가능하고,
권한 모델/훅과 잘 맞으며, 인프라가 가볍다. 안정화되면 동일 로직을 **MCP 서버**로
감싸 도구 호출을 1급 시민으로 만든다.

모든 명령은 **토큰 상한**과 **기계가독 출력(기본 간결, `--json` 옵션)**을 가진다.

지식(위키/KV):
- `aw search "<쿼리>" [--type gotcha] [--subsystem mm] [-n 8]`
  → FTS5 랭킹 상위 N개: `id · title · 1줄 스니펫`. (전체 노트 아님 → 토큰 절약)
- `aw get <id>` → 노트 1개 전체.
- `aw note new|append <id>` → 노트 생성/추가 (frontmatter 검증 포함).
- `aw kv get|set <key> [value]` → 빠른 사실.
- `aw related <id|symbol>` → links 그래프로 인접 지식.

코드 인덱스(커널 친화):
- `aw where <symbol>` → ctags 조회: 정의 위치(파일·라인·서명). grep 대체.
- `aw show <symbol>` → **그 함수/구조체 본문만 잘라서** 출력. 거대한 파일 통째 read 금지.
- `aw xref <symbol> [--callers|--callees]` → cscope 크로스레퍼런스(상한 둠).
- `aw reindex [--code|--notes]` → tags/cscope.out/index.db 재생성.

연결:
- `aw recall <symbol>` → **핵심 명령.** 한 번에 (1) 심볼 정의 위치 + (2) 그 심볼에
  닻 내린 위키 노트 + (3) 관련 gotcha를 묶어 반환. "이 함수 만질 건데 아는 거 다 줘".

---

## 7. 세션 통합 (Claude Code 훅)

- **SessionStart 훅** → `aw digest` 출력을 주입. 다이제스트는 **엄격한 토큰 예산**
  (예: ≤ 1.5k 토큰) 안에서:
  - 핀(pinned) 사실 몇 개 (현재 작업 서브시스템, 빌드 명령 등 KV)
  - 최근 갱신/고확신 노트 **제목+id 목록만** (본문 아님 — 필요하면 에이전트가 `get`)
  - 인덱스 통계 (노트 N개, 심볼 M개, "검색은 `aw search`로" 안내)
  → 즉, **CLAUDE.md 안티패턴(전부 주입)을 피하고** "목차 + 검색 진입점"만 준다.
- **/remember (수동) 또는 Stop 훅** → 이번 세션에서 새로 알아낸 durable 지식을
  노트로 증류하도록 유도. 검증된 사실만, 심볼 닻과 confidence를 달아서.
- (커널 작업 시작 시) 한 번 `aw reindex --code`로 tags/cscope 생성.

CLAUDE.md에는 **규칙만** 둔다: "코드 탐색 전 `aw recall`/`aw search`를 먼저 써라.
거대한 파일을 통째로 읽지 말고 `aw show <symbol>`을 써라. 새 사실은 노트로 남겨라."

---

## 8. 리눅스 커널 특화 (컨텍스트 낭비 정면 대응)

커널이 컨텍스트를 잡아먹는 구체적 원인과 대응:

| 낭비 원인 | 대응 |
|---|---|
| 파일이 수천 줄 → whole-file read | `aw show <symbol>`로 **함수 단위 슬라이싱**만 읽기 |
| `grep` 한 번에 수백 매치 | `aw where`/`aw xref`로 ctags/cscope **정밀 회수**(상한) |
| 라인 번호가 버전마다 바뀜 → 지식 썩음 | **심볼 닻**(P3), 라인은 보조. reindex로 라인만 갱신 |
| 같은 탐색을 매 세션 반복 | `symbol-note`/`gotcha`에 응축 저장 → `aw recall`로 즉시 재사용 |
| config/arch별로 코드가 갈림 | frontmatter `config_deps`, `arch`로 명시 → 잘못된 일반화 방지 |
| 서브시스템 경계가 흐림 | `subsystem` 태그로 검색 범위 좁힘(`--subsystem mm`) |
| clangd가 커널 규모에서 붕괴 | 기본은 ctags/cscope(공식 `make cscope`). clangd는 좁은 하위트리에 선택적 |

추가로, 커널은 거대하므로 **인덱스를 커널 트리 안이 아니라 별도 위치**(예:
`~/.agentwiki/<kernel-sha>/`)에 두고, 위키 노트는 우리 프로젝트(agentwiki) git에
둔다 — 커널 트리를 더럽히지 않고, 위키는 커널 버전과 독립적으로 누적된다.

---

## 9. 구현 로드맵

**Phase 0 — 골격 (MVP, 가장 가치 높음)**
- `notes/` 디렉터리 + frontmatter 스펙, 마크다운 파서.
- `index.db` 스키마 + `aw reindex --notes` (마크다운 → SQLite).
- `aw search` (FTS5), `aw get`, `aw note new|append`, `aw kv`.
- 검증: 노트 몇 개 넣고 검색이 토큰 적게 정확히 회수하는지.

**Phase 1 — 코드 인덱스 (커널 효율의 핵심)**
- `aw reindex --code`: `ctags -R` + `cscope -b` 래핑(또는 커널 `make cscope`).
- `aw where`, `aw show`(함수 슬라이싱), `aw xref`.
- `symbols` 테이블 채우기 + frontmatter `code_refs` ↔ `symbols` 연결.
- `aw recall <symbol>` (코드+지식 통합 회수).

**Phase 2 — 세션 통합**
- `aw digest` + SessionStart 훅(토큰 예산 적용).
- `/remember` 워크플로 / Stop 훅으로 지식 증류 유도.
- CLAUDE.md에 사용 규칙 추가.

**Phase 3 — 다듬기 (선택)**
- 동일 로직 MCP 서버로 래핑(도구 호출 1급화).
- `aw stale`: 닻 심볼이 사라졌거나 confidence 낮은/오래된 노트 점검.
- (옵션) 임베딩 보조 검색을 "유사 노트 찾기"에만 한정 추가.

---

## 10. 트레이드오프 & 리스크

- **자동 추출을 안 함(처음엔).** mem0식 자동 메모리는 환각·노이즈 위험이 커서,
  초기엔 **검증된 사실만 명시적으로 기록**하는 큐레이션 방식을 택한다. 신뢰성 우선.
  나중에 `/remember`를 반자동(에이전트가 제안 → 사람/에이전트가 승인)으로 확장 가능.
- **이중 저장(마크다운+DB) 동기화.** DB는 항상 파생이라는 규칙(P2)으로 단순화.
  훅/명령에서 노트 변경 시 증분 reindex, 의심되면 전체 reindex.
- **ctags/cscope의 의미 한계.** 매크로 떡칠된 커널 코드(예: 매크로로 생성되는 심볼)는
  놓칠 수 있음 → 그런 부분은 `symbol-note`로 사람이/에이전트가 보강.
- **지식 부패.** 커널이 바뀌면 노트가 틀려질 수 있음 → `confidence`/`updated`/`source`
  메타와 `aw stale` 점검으로 관리. 심볼 닻 덕분에 라인 드리프트엔 강함.

---

## 부록. 참고한 기존 프로젝트

- [mem0 (Universal memory layer)](https://github.com/mem0ai/mem0) · [OpenMemory MCP](https://mem0.ai/openmemory)
- [Letta / MemGPT 비교](https://vectorize.io/articles/mem0-vs-letta)
- [Claude Code 메모리 공식 문서 (CLAUDE.md / auto memory)](https://code.claude.com/docs/en/memory) · [Memory tool](https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool)
- [claude-mem (세션 컨텍스트 압축)](https://github.com/thedotmack/claude-mem)
- [Claude Memory Bank](https://nsclass.github.io/2026/03/15/claude-memory-bank)
- [Serena (LSP+MCP 시맨틱 코드 검색)](https://github.com/oraios/serena)
- [LSP/clangd의 대규모 코드베이스 한계](https://dev.to/harry_tanama_51571ebf90b6/lsp-server-and-clangd-face-challenges-dealing-with-very-large-codebases-56gc)
- [리눅스 커널 코드 브라우징 (cscope/ctags)](https://kernelnewbies.org/FAQ/CodeBrowsing)
- [AI Agent Memory Frameworks 2026 비교](https://atlan.com/know/best-ai-agent-memory-frameworks-2026/)
