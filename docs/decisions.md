# agentwiki — 확정 설계 결정 (ADR)

> 인터뷰로 **확정된** 24개 결정의 정본. 초기 조사·탐색안은 [`design.md`](./design.md) 참조.
> 상태: **전체 확정.** 갱신: 2026-05-31

---

## 0. 결정 요약

| # | 항목 | 결정 |
|---|---|---|
| D1 | 적용 범위 | **멀티 머신 / 멀티 리포 공유** |
| D2 | 구현 언어 | **Rust** |
| D3 | 접근 계층 | **처음부터 MCP 서버** |
| D4 | 저장 형태 | **마크다운(진실) + SQLite(파생 인덱스) 하이브리드** |
| D5 | 동기화 | **Git 저장소로 동기화** (파생물은 로컬 재생성) |
| D6 | 지식 캡처 | **반자동 (제안 → 승인)** |
| D7 | 코드 인덱스 | **ctags/cscope + clangd 하이브리드** |
| D8 | 노트 언어 | **한국어 위주** (FTS5 trigram 토크나이저) |
| D9 | 의미 검색 | **FTS5 + 로컬 임베딩 하이브리드** |
| D10 | 노트 레이아웃 | **서브시스템별 디렉터리** |
| D11 | 세션 주입 | **최소 다이제스트만** (≤~1.5k 토큰) |
| D12 | 인덱스 갱신 | **수동 reindex** |
| D13 | 충돌 처리 | **Append-only 엔트리 로그** |
| D14 | 지식 부패 | **심볼 닻 검증 + stale 플래그** |
| D15 | 비밀/보안 | **비밀 스캔 — 경고만 (차단 안 함)** |
| D16 | 커널 버전 | **단일 트리 위주** (확장 여지) |
| D17 | MCP 도구 입자 | **중간 (3~5 그룹 + `op` 인자)** |
| D18 | 임베딩 모델 | **fastembed-rs + 다국어 소형모델** |
| D19 | 첫 마일스톤 | **위키/메모리 먼저** |
| D20 | 레포 구조 | **도구 레포 / 데이터 레포 분리** |
| D21 | 노트 ID | **서브시스템-해시** |
| D22 | KV 스코프 | **global / project / machine 3스코프** |
| D23 | 출력 상한 | **보수적 기본 + 확장 인자** |
| D24 | 바이너리 배포 | **릴리스 바이너리(musl) + cargo install** |

---

## 1. 아키텍처 (확정)

```
┌──────────────────────────────────────────────────────────────────┐
│  도구 레포 (aw-tool, Rust) ── D2/D20                               │
│   crates/ aw-core · aw-mcp · aw-cli(디버깅용)                      │
│   배포: GitHub Releases musl 바이너리 + cargo install ── D24       │
└───────────────┬──────────────────────────────────────────────────┘
                │ MCP stdio ── D3
   ┌────────────┴───────────────┐
   │  Claude Code 세션           │
   │   SessionStart → 최소 다이제스트 주입(≤1.5k tok) ── D11         │
   │   작업 중 → MCP 도구(3~5 그룹+op) ── D17                        │
   │   적당 시점 → 반자동 캡처: propose → 승인 → commit ── D6        │
   └────────────┬───────────────┘
                │ 읽기/쓰기
┌───────────────▼──────────────────────────────────────────────────┐
│  데이터 레포 (agentwiki-data) ── D20, git 동기화 ── D5/D1         │
│   notes/<subsystem>/<id>.md   (마크다운 = 진실) ── D4/D10          │
│   notes 는 append-only 엔트리 로그로 누적 ── D13                   │
│   .gitignore: .aw/(파생·로컬 전부)                                 │
└───────────────┬──────────────────────────────────────────────────┘
                │ 수동 reindex(머신마다 로컬 재생성) ── D12, 비동기화 │
┌───────────────▼──────────────────────────────────────────────────┐
│  .aw/index.db (SQLite, 파생) ── D4                                 │
│   notes_fts(trigram, 한국어) ── D8                                 │
│   notes_vec(fastembed-rs 로컬 임베딩) ── D9/D18                    │
│   kv(scope: global/project/machine) ── D22                        │
│   symbols(ctags) · links                                          │
├───────────────────────────────────────────────────────────────────┤
│  코드 인덱스(커널 트리 밖, 비동기화) ── D7/D16                    │
│   tags(ctags) · cscope.out · compile_commands.json → clangd      │
└───────────────────────────────────────────────────────────────────┘
```

**불변 원칙**
- **마크다운만 git 공유(D5).** SQLite·임베딩·tags·cscope.out·compile_commands는
  전부 파생물 → 비동기화, 각 머신에서 **수동 `reindex`로 재생성(D12)**.
- **검색 우선, 주입 최소(D11).** 세션 시작엔 목차+진입점만, 본문은 도구로 회수.
- **코드는 심볼에 닻(D14/D21).** 라인 번호는 보조 메타.

---

## 2. 레포 구조 (D20)

**도구 레포** `aw-tool` (Rust):
```
crates/
  aw-core/    # 마크다운+frontmatter 파서, SQLite, FTS5/벡터, ctags/cscope/clangd 어댑터,
              #   fastembed-rs 임베딩, append-only 로그 컴팩션, 심볼-닻 검증
  aw-mcp/     # MCP 서버 (도구 그룹 wiki/code/recall/kv/admin), aw-core 의존
  aw-cli/     # 동일 코어 위 얇은 CLI (디버깅·스크립트용)
.github/workflows/release.yml   # musl 정적 바이너리 릴리스 ── D24
```

**데이터 레포** `agentwiki-data` (머신 간 git 공유 ── D1/D5):
```
notes/
  mm/ sched/ net/ fs/ ...        # 서브시스템별 ── D10
    <subsystem>-<hash>.md        # 노트 ID = 파일명 ── D21
notes-local/                     # 민감/머신 한정(.gitignore) ── D15
kv/
  global.toml  project.toml      # 공유 KV (machine 스코프는 로컬) ── D22
.gitignore                       # .aw/ (index.db, tags, cscope.out 등 파생물)
.aw/                             # 로컬 파생물 — 비동기화, 수동 reindex로 생성 ── D12
```

> 단일 트리 위주(D16). 코드 인덱스는 `~/.cache/aw/<repo-id>/`에 하나.
> 멀티 버전이 필요해지면 `<repo-id>/<kernel-sha>/`로 키잉 확장(자리만 예약).

---

## 3. 데이터 모델

### 노트 (마크다운 = 진실, append-only ── D13)
한 노트 파일은 시간순 **엔트리 로그**다. 같은 주제에 사실을 덧붙일 때 기존 줄을
고치지 않고 새 엔트리를 append → git 머지 충돌이 사실상 사라진다. 읽을 때 코어가
신뢰도/시간으로 합성하고, `admin compact`로 중복을 주기적으로 정리한다.

```yaml
---
id: mm-a1b2c3                    # 서브시스템-해시 ── D21 (충돌 0, append-only 친화)
title: do_mmap의 mmap_lock 규약
type: gotcha                    # concept|gotcha|decision|howto|symbol-note
subsystem: mm                   # 디렉터리와 일치 ── D10
tags: [locking, mmap]
code_refs:                      # 심볼 닻 + 검증 해시 ── D14
  - sym: do_mmap
    file: mm/mmap.c
    sig_hash: "ab12…"           # 시그니처 변경/소멸 시 stale 플래그
confidence: high
---
## 2026-05-31T..Z · host=devbox · confidence=high
do_mmap 진입 시 호출자가 mmap_write_lock 보유 가정 … (엔트리 1)

## 2026-06-02T..Z · host=laptop · confidence=med
예외: nommu 빌드에서는 … (엔트리 2, append)
```

### SQLite (.aw/index.db, 파생 ── D4)
```sql
CREATE VIRTUAL TABLE notes_fts USING fts5(
  id UNINDEXED, title, body, tags, tokenize = "trigram"          -- 한국어 ── D8
);
CREATE VIRTUAL TABLE notes_vec USING vec0(                       -- 로컬 임베딩 ── D9
  id TEXT, embedding FLOAT[384]                                  -- multilingual-e5-small류 ── D18
);
CREATE TABLE kv (                                                -- ── D22
  key TEXT, value TEXT,
  scope TEXT,                    -- 'global' | 'project:<id>' | 'machine:<host>'
  updated TEXT,
  PRIMARY KEY (key, scope)
);
CREATE TABLE symbols (name, kind, file, line, signature, subsystem);  -- ctags
CREATE TABLE links (src, dst, rel);                                   -- 노트↔노트/심볼
```

**하이브리드 검색(D9/D18):** 질의를 FTS5(trigram) + 벡터(fastembed-rs로 로컬 추론한
임베딩) 양쪽으로 돌려 **RRF(reciprocal rank fusion)로 랭크 결합**. 외부 API·데몬 불필요,
순수 Rust(ONNX). 모델은 다국어 소형(한국어 지원) — `multilingual-e5-small` 등.

**KV 조회 우선순위(D22):** `machine:<host>` → `project:<id>` → `global` (구체 우선).
예: 커널 소스 경로=machine, 빌드 명령=project, 코딩 규칙=global.

---

## 4. MCP 도구 표면 — 중간 입자 3~5 그룹 (D17/D23)

각 그룹은 `op` 인자로 동작 분기. 모든 출력은 **보수적 기본 상한 + 확장 인자(D23)**.

1. **`wiki`** — 지식 위키
   - `op: search|get|related` (읽기) · `op: propose|commit` (반자동 쓰기 ── D6)
   - 기본 상한: `search` 8건·각 2줄. `propose`는 초안 생성 → 승인 후 `commit`이
     append 엔트리로 기록(D13). `limit`/`expand`로 명시 확장.
2. **`code`** — 코드 인덱스(커널 효율 ── D7)
   - `op: where|show|xref`
   - `where`(ctags) 정의 위치 · `show` **함수 단위 슬라이스만**(whole-file read 금지,
     기본 함수 1개) · `xref`(cscope) 호출자/피호출자(기본 20건). 정밀 필요 시 clangd 승격.
3. **`recall`** — 통합 회수(핵심)
   - 심볼 1개 → (정의 위치 + 닻 내린 노트 + 관련 gotcha) 묶어 반환.
4. **`kv`** — 빠른 사실
   - `op: get|set`, `scope` 인자(global/project/machine ── D22).
5. **`admin`** — 운영
   - `op: reindex|stale|compact|digest|scan-secrets`
   - `reindex`(수동 ── D12) · `stale`(심볼 닻 검증 실패 목록 ── D14) ·
     `compact`(append 로그 정리 ── D13) · `digest`(세션 다이제스트 ── D11) ·
     `scan-secrets`(비밀 스캔, **경고만** ── D15).

---

## 5. 운영 규칙

- **세션 시작(D11):** `admin digest` → 핀 KV + 최근/고확신 노트 *제목+id 목록만*
  (≤~1.5k 토큰). 본문은 `wiki get`/`recall`로 회수.
- **인덱스 갱신(D12):** 명시적 `admin reindex` 호출 시에만 (노트 FTS·벡터, 코드 tags/cscope).
- **충돌(D13):** 노트=append-only 엔트리 로그 → git 머지 충돌 최소화. 주기적 `compact`.
- **지식 부패(D14):** `code_refs.sig_hash`를 코드 인덱스와 대조 → 심볼 소멸/시그니처
  변경 시 `stale` 플래그. 심볼 닻 덕분에 라인 드리프트엔 강함.
- **비밀(D15):** `scan-secrets`로 비밀/키/토큰 패턴 탐지 → **경고만**(차단 안 함).
  민감 노트는 `notes-local/`(비동기화)로 두는 것을 권장.
- **clangd(D7):** 커널 `scripts/clang-tools/gen_compile_commands.py`로
  compile_commands.json 생성(빌드 후). 없으면 ctags/cscope로 폴백.

---

## 6. 리눅스 커널 컨텍스트 낭비 대응 (요청 핵심)

| 낭비 원인 | 대응 | 관련 결정 |
|---|---|---|
| 수천 줄 파일 통째 read | `code show`로 **함수 단위 슬라이싱**만 | D7/D23 |
| `grep` 수백 매치 | `code where`/`xref`로 ctags/cscope **정밀 회수**(상한) | D7/D23 |
| 라인 번호가 버전마다 바뀜 | **심볼 닻** + sig_hash, 라인은 보조 | D14/D21 |
| 같은 탐색 매 세션 반복 | gotcha/symbol-note에 응축 → `recall`로 즉시 재사용 | D6/recall |
| 세션 시작 토큰 폭증 | 최소 다이제스트(목차+진입점)만 | D11 |
| clangd가 커널 규모서 붕괴 | 기본 ctags/cscope, 좁은 트리에만 clangd 승격 | D7/D16 |
| 한국어 노트 검색 정확도 | FTS5 trigram + 다국어 임베딩 하이브리드 | D8/D9/D18 |

---

## 7. 구현 로드맵 (첫 마일스톤 = 위키/메모리 ── D19)

**M1 — 위키/메모리 코어 (먼저)**
- `aw-core`: 마크다운+frontmatter 파서, append-only 로그(D13), `index.db` 스키마,
  FTS5(trigram ── D8), fastembed-rs 임베딩 + 하이브리드 RRF 검색(D9/D18).
- `aw-mcp`: `wiki`(search/get/related/propose/commit) + `kv`(3스코프 ── D22)
  + `admin`(reindex/digest/compact/scan-secrets).
- 반자동 캡처(D6), 비밀 스캔 경고(D15).
- 검증: 한국어 노트 적재 → 토큰 적게 정확 회수, 세션 다이제스트 동작.

**M2 — 코드 인덱스 (커널 효율)**
- ctags/cscope 어댑터, `symbols` 적재, `code`(where/show/xref) + `recall`.
- 심볼 닻 검증/`stale`(D14). 인덱스 캐시 위치(D16). 수동 reindex(D12).

**M3 — clangd 승격 & 배포**
- compile_commands.json 연동, 정밀 질의 시 clangd 승격(D7).
- musl 릴리스 바이너리 + cargo install(D24), 데이터 레포 git 동기화(D5/D20).

**M4 — 확장(선택)**
- append 로그 자동 compact 튜닝, 멀티 버전 키잉(D16 확장), 임베딩 모델 교체 실험.

---

## 7.5. 구현 현황 (2026-05-31)

프로토타입이 동작하며 테스트 스크립트로 검증된다. `scripts/smoke.sh` = 34 체크 통과.

| 마일스톤 | 상태 | 비고 |
|---|---|---|
| M1 위키/메모리 | ✅ 완료 | 노트·FTS5(trigram)+LIKE 폴백·KV 3스코프·반자동 캡처·다이제스트·비밀 스캔 |
| M2 코드 인덱스 | ✅ 완료 | ctags(where/show 슬라이싱)·cscope(xref)·recall·심볼 닻 stale 검증(D14) |
| M3 패키징/통합 | 🟡 부분 | CI/release 워크플로·SessionStart 훅 완료. clangd 승격은 미구현 |
| M4 확장 | 🟡 부분 | 로컬 임베딩 하이브리드 활성화·append compact 완료. fastembed 고품질 모델·멀티버전은 향후 |

구현된 크레이트: `aw-core`(라이브러리, 25 단위테스트), `aw-mcp`(MCP 서버:
wiki/code/recall/kv/admin), `aw-cli`(`aw` 디버깅 CLI). 검증 스크립트:
`scripts/smoke.sh`(자동, 38체크), `scripts/demo.sh`(시연).

**M4 구현 내용:**
- **하이브리드 벡터 검색 활성화**(D9): 의존성 0의 결정론적 로컬 임베더
  (`HashEmbedder` — 문자 트라이그램 feature hashing). FTS5와 RRF 융합되며
  최소 코사인 임계값(0.15)으로 무관 노트를 거른다. 한국어 문자 n-gram에 적합.
- **append 로그 compact**(D13): 동일 본문 중복/빈 엔트리 제거, 가장 오래된 출처 보존.

**의도적으로 미완(정직 고지):**
- 고품질 임베딩(D18: fastembed 다국어 모델)은 `embeddings` feature 뒤 트레이트로
  유지 — 네이티브 ONNX 런타임 다운로드가 ephemeral 환경에서 불안정하므로 기본은
  `HashEmbedder`. 학습 임베딩만큼 의미가 풍부하진 않으나 하이브리드 경로는 실동작.
- clangd 승격(D7 정밀 경로)·멀티 커널 버전 키잉(D16)은 향후.

## 8. 미해결/추후 결정 (열어둠)
- 임베딩 모델 구체 선정·차원(D18: multilingual-e5-small 가정, 벤치 후 확정).
- 멀티 커널 버전 동시 지원의 구체 키잉(D16은 단일 트리 전제).
- aw-cli 공식 지원 범위(현재 디버깅용) — 사용 패턴 보고 결정.
- append 로그 compact 자동화 정책(수동 `admin compact` 우선).
