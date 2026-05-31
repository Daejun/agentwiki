# agentwiki — 확정 설계 결정 (ADR)

> 이 문서는 인터뷰를 통해 **확정된** 설계 결정을 기록한다.
> 초기 조사·탐색안은 [`design.md`](./design.md) 참조.
> 결정일: 2026-05-31

---

## 0. 한눈에 보기

| # | 항목 | 결정 |
|---|---|---|
| D1 | 적용 범위 | **멀티 머신 / 멀티 리포 공유** |
| D2 | 구현 언어 | **Rust** |
| D3 | 접근 계층 | **처음부터 MCP 서버** |
| D4 | 저장 형태 | **마크다운(진실) + SQLite(파생 인덱스) 하이브리드** |
| D5 | 동기화 | **Git 저장소로 동기화** (SQLite는 로컬 재생성) |
| D6 | 지식 캡처 | **반자동 (제안 → 승인)** |
| D7 | 코드 인덱스 | **ctags/cscope + clangd 하이브리드** |
| D8 | 노트 언어 | **한국어 위주** (FTS5 CJK 토크나이저) |
| D9 | 의미 검색 | **지금은 FTS5, 임베딩은 나중 확장 여지** |
| D10 | 노트 레이아웃 | **서브시스템별 디렉터리** |
| D11 | 세션 주입 | **최소 다이제스트만** |
| D12 | 인덱스 갱신 | **온디맨드(lazy)** |
| D13 | 충돌 처리 | **파일당 1노트 + LWW** |
| D14 | 지식 부패 | **심볼 닻 검증 자동 플래그** |
| D15 | 비밀/보안 | **비밀 스캔 + 차단** |
| D16 | clangd 준비 | **커널 내장 스크립트** (gen_compile_commands.py) |
| D17 | MCP 도구 입자 | **중간 (3~5개 그룹)** |
| D18 | 커널 버전 | **단일 트리 위주** |
| D19 | 노트 ID | **서브시스템-슬러그** |
| D20 | 바이너리 배포 | **cargo install / 릴리스 바이너리** |
| D21 | 레포 구조 | **도구 레포 / 데이터 레포 분리** |
| D22 | KV 스코프 | **글로벌 / 프로젝트 / 머신** |
| D23 | 출력 상한 | **보수적 기본 + 확장 인자** |
| D24 | 첫 마일스톤 | **위키/메모리 먼저** |

---

## 1. 시스템 구성 (확정)

```
┌──────────────────────────────────────────────────────────────┐
│  도구 레포 (aw-tool)  ── D21                                   │
│   Rust 크레이트: MCP 서버 + 내부 코어 라이브러리               │
│   cargo install / GitHub Releases 바이너리로 각 머신 배포(D20) │
└───────────────┬──────────────────────────────────────────────┘
                │ MCP (stdio) — D3
   ┌────────────┴──────────────┐
   │  Claude Code 세션          │
   │  - SessionStart: 최소 다이제스트 주입 (D11)                 │
   │  - 작업 중: MCP 도구 호출 (3~5 그룹, D17)                   │
   │  - 종료/적당 시점: 반자동 캡처 제안→승인 (D6)              │
   └────────────┬──────────────┘
                │ 읽기/쓰기
┌───────────────▼──────────────────────────────────────────────┐
│  데이터 레포 (agentwiki-data)  ── D21, git 동기화(D5)         │
│   notes/<subsystem>/<id>.md   (마크다운 = 진실, D4/D10)       │
│   .gitignore: index.db, notes-local/* (파생/로컬은 비동기화)  │
└───────────────┬──────────────────────────────────────────────┘
                │ lazy 재생성 (D12) — 머신마다 로컬
┌───────────────▼──────────────────────────────────────────────┐
│  index.db (SQLite, 파생, 비동기화)                            │
│   notes/notes_fts(CJK 토크나이저, D8) · kv(scope, D22)        │
│   symbols(ctags) · links · [vec 자리 예약, D9]                │
├───────────────────────────────────────────────────────────────┤
│  코드 인덱스 (커널 트리 밖, 비동기화) ── D7, D18              │
│   tags (ctags) · cscope.out · compile_commands.json(D16)→clangd│
└───────────────────────────────────────────────────────────────┘
```

핵심 원칙(불변):
- **마크다운만 git으로 공유(D5/D21).** SQLite·tags·cscope.out·compile_commands.json은
  전부 **파생물이라 비동기화**, 각 머신에서 lazy 재생성(D12).
- **검색 우선, 주입 최소(D11).** 세션 시작엔 목차+진입점만, 본문은 도구로 회수.
- **코드는 심볼에 닻(D14/D19).** 라인 번호는 보조.

---

## 2. 레포 구조 (D21)

**도구 레포** (`aw-tool`, Rust):
```
crates/
  aw-core/      # 마크다운 파서, SQLite, 인덱서, 검색, ctags/cscope/clangd 어댑터
  aw-mcp/       # MCP 서버 (도구 3~5 그룹), aw-core에 의존
  aw-cli/       # (선택) 동일 코어 위 얇은 CLI — 디버깅/스크립트용
.github/workflows/release.yml   # 정적 바이너리 릴리스 (D20)
```

**데이터 레포** (`agentwiki-data`, 머신 간 git 공유):
```
notes/
  mm/ sched/ net/ fs/ ...        # 서브시스템별 (D10)
    <subsystem>-<slug>.md        # 노트 ID = 파일명 (D19)
notes-local/                     # 로컬 전용(민감/머신 한정), .gitignore (D15/D22)
kv/                              # 공유 KV(글로벌/프로젝트 스코프) — 마크다운/TOML
.gitignore                       # index.db, .aw/ 등 파생물 제외
.aw/                             # 로컬 파생: index.db, tags, cscope.out (비동기화)
```

> 단일 트리 위주(D18)이므로 코드 인덱스는 `~/.cache/aw/<repo-id>/`에 하나만 둔다.
> 향후 여러 버전이 필요해지면 `<repo-id>/<kernel-sha>/`로 키잉 확장 가능(자리만 남겨둠).

---

## 3. 데이터 모델 (확정 반영)

### 노트 frontmatter (D8/D10/D14/D19)
```yaml
---
id: mm-do_mmap-locking          # 서브시스템-슬러그 (= 파일명, D19)
title: do_mmap의 mmap_lock 규약
type: gotcha
subsystem: mm                   # 디렉터리와 일치 (D10)
tags: [locking, mmap]
code_refs:                      # 심볼 닻 — 자동 검증 대상 (D14)
  - sym: do_mmap
    file: mm/mmap.c
    sig_hash: "ab12…"           # 검증용 시그니처 해시 (바뀌면 stale 플래그)
confidence: high
source: "세션 확인 / commit <sha>"
updated: 2026-05-31T..Z         # LWW 기준 (D13)
---
본문 (한국어 위주)
```

### SQLite (D4/D8/D9/D22) — 전부 파생, 재생성 가능
```sql
CREATE VIRTUAL TABLE notes_fts USING fts5(
  id UNINDEXED, title, body, tags,
  tokenize = "trigram"          -- CJK(한국어) 대응 (D8)
);
CREATE TABLE kv (
  key TEXT, value TEXT,
  scope TEXT,                   -- 'global' | 'project:<id>' | 'machine:<host>' (D22)
  updated TEXT,
  PRIMARY KEY (key, scope)
);
CREATE TABLE symbols (name, kind, file, line, signature, subsystem);
CREATE TABLE links (src, dst, rel);
-- D9: 임베딩 확장 자리 예약 (지금은 미사용)
-- CREATE VIRTUAL TABLE notes_vec USING vec0(...);
```

KV 조회 우선순위(D22): `machine:<host>` → `project:<id>` → `global` (구체적인 것 우선).
예: 빌드 명령=project, 커널 소스 경로=machine, 코딩 규칙=global.

---

## 4. MCP 도구 표면 — 3~5 그룹 (D17/D23)

도구는 **중간 입자**: 그룹별로 묶되 동작은 `op` 인자로 분기. 모든 출력은
**보수적 기본 상한 + 확장 인자**(D23): 예) `wiki.search` 기본 8건·각 2줄,
`code.show`는 함수 1개, 넘으면 `limit`/`expand`로 명시 확장.

1. **`wiki`** — 지식 위키
   - `op: search|get|related` (읽기), `op: propose|commit` (반자동 쓰기, D6)
   - `search`: FTS5 랭킹 N건(id·title·스니펫). `get`: 노트 1개. `related`: links 그래프.
   - `propose`: 새 노트/추가 **초안 생성 → 사용자/에이전트 승인 후 `commit`** (D6).
2. **`code`** — 코드 인덱스 (커널 효율)
   - `op: where|show|xref`
   - `where`(ctags) 정의 위치, `show` **함수 단위 슬라이스만**(whole-file read 금지),
     `xref`(cscope) 호출자/피호출자. 정밀 필요 시 clangd로 승격(D7).
3. **`recall`** — 통합 회수 (핵심)
   - 심볼 1개로 (정의 위치 + 닻 내린 노트 + 관련 gotcha) 묶어 반환.
4. **`kv`** — 빠른 사실
   - `op: get|set`, `scope` 인자(global/project/machine, D22).
5. **`admin`** — 운영
   - `op: reindex(lazy)|stale|digest|scan-secrets`
   - `stale`: 심볼 닻 검증 실패 노트 목록(D14). `scan-secrets`: 비밀 스캔(D15).
   - `digest`: SessionStart용 최소 다이제스트 생성(D11).

---

## 5. 운영 규칙 (확정)

- **세션 시작(D11):** `admin digest` → 핀 KV 몇 개 + 최근/고확신 노트 *제목+id 목록만*
  (≤ ~1.5k 토큰). 본문은 에이전트가 `wiki get`/`recall`로 회수.
- **인덱스 갱신(D12):** 도구 호출 시 mtime/HEAD 비교 → 필요하면 그때 증분 재생성(lazy).
  명시 `admin reindex`도 제공.
- **충돌(D13):** 노트=파일당 1개. git 충돌 시 frontmatter `updated` 최신 우선(LWW)으로
  자동 해소하는 머지 드라이버 제공, 본문 양쪽 보존이 필요하면 마커 남김.
- **지식 부패(D14):** `code_refs.sig_hash`를 코드 인덱스와 대조 → 심볼 소멸/시그니처
  변경 시 노트에 `stale` 플래그. `admin stale`로 점검·갱신 유도.
- **비밀(D15):** `wiki commit` 전과 (선택)pre-commit 훅에서 비밀/키/토큰 패턴 스캔 →
  탐지 시 **커밋 차단**. 민감 노트는 `notes-local/`(비동기화)로.
- **clangd(D16):** 커널의 `scripts/clang-tools/gen_compile_commands.py`로
  compile_commands.json 생성(빌드 후). 없으면 ctags/cscope로 폴백(D7).

---

## 6. 구현 로드맵 (첫 마일스톤 = 위키/메모리, D24)

**M1 — 위키/메모리 코어 (D24, 가장 먼저)**
- `aw-core`: 마크다운+frontmatter 파서, `index.db` 스키마, FTS5(trigram, D8).
- `aw-mcp`: `wiki`(search/get/related/propose/commit) + `kv`(scope) + `admin`(reindex/digest/scan-secrets).
- 반자동 캡처(propose→commit, D6), 비밀 스캔(D15), LWW 머지 드라이버(D13).
- 검증: 노트 적재 → 토큰 적게 정확 회수, 세션 다이제스트 동작.

**M2 — 코드 인덱스 (커널 효율)**
- ctags/cscope 어댑터, `symbols` 적재, `code`(where/show/xref) + `recall`.
- 심볼 닻 검증/`stale`(D14). lazy 재생성(D12), 인덱스 캐시 위치(D18).

**M3 — clangd 승격 & 배포**
- compile_commands.json 연동(D16), 정밀 질의 시 clangd 승격.
- cargo/릴리스 바이너리(D20), 데이터 레포 git 동기화 워크플로(D5/D21).

**M4 — 확장(선택)**
- 임베딩 하이브리드 검색(D9, vec 테이블 활성화), 멀티 버전 키잉(D18 확장).

---

## 7. 미해결/추후 결정 (열어둠)
- 임베딩 모델 선택(로컬 onnx/fastembed 등) — M4 진입 시.
- 여러 커널 버전 동시 지원의 구체 키잉 — 필요해질 때(D18은 단일 트리 전제).
- MCP 외 CLI를 공식 지원할지(현재 디버깅용으로만) — 사용 패턴 보고 결정.
