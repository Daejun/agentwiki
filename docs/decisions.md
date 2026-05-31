# agentwiki — 설계 결정 기록 (ADR, 진행 중)

> 인터뷰로 **확정된** 결정만 기록한다. 초기 조사·탐색안은 [`design.md`](./design.md) 참조.
> 상태: **1~3라운드 확정(D1~D12). 나머지 항목은 아래 "미정" 절에서 인터뷰 진행 중.**
> 갱신: 2026-05-31

---

## 0. 확정 결정 (D1~D12)

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
| D9 | 의미 검색 | **FTS5 + 로컬 임베딩 하이브리드 (처음부터 포함)** |
| D10 | 노트 레이아웃 | **서브시스템별 디렉터리** |
| D11 | 세션 주입 | **최소 다이제스트만** |
| D12 | 인덱스 갱신 | **수동 reindex** |

---

## 1. 확정 결정이 함의하는 구조

```
Rust 크레이트 → MCP 서버(stdio, D3) ── Claude Code 세션
                                         - SessionStart: 최소 다이제스트(D11)
                                         - 작업 중: MCP 도구로 검색/회수/기록(D6)
        │ 읽기/쓰기
데이터(git 동기화, D5) : notes/<subsystem>/<slug>.md  (마크다운=진실, D4/D10)
        │ 수동 reindex(D12) — 머신마다 로컬, 비동기화
index.db (SQLite, 파생) : notes_fts(CJK, D8) + notes_vec(로컬 임베딩, D9)
                          + kv + symbols + links
코드 인덱스(파생, 비동기화) : tags + cscope.out (+ clangd, D7)
```

불변 원칙:
- **마크다운만 git으로 공유(D5).** SQLite·임베딩·tags·cscope.out은 전부 파생물 →
  비동기화, 각 머신에서 **수동 `reindex`로 재생성(D12)**.
- **검색 우선, 주입 최소(D11).** 세션 시작엔 목차+진입점만, 본문은 도구로 회수.
- **하이브리드 검색(D9).** 기본 FTS5(trigram, 한국어) + 로컬 임베딩 벡터 검색을
  랭크 결합. 임베딩 모델은 로컬 추론(예: onnx/fastembed류, 모델은 미정 항목).

### 데이터 모델 (확정분 반영)

노트 frontmatter (D8/D10):
```yaml
---
id: mm-do_mmap-locking          # 서브시스템 접두 슬러그(상세 규칙은 미정)
title: do_mmap의 mmap_lock 규약
type: gotcha                    # concept|gotcha|decision|howto|symbol-note
subsystem: mm                   # 디렉터리와 일치 (D10)
tags: [locking, mmap]
code_refs: [{sym: do_mmap, file: mm/mmap.c}]   # 심볼 닻
confidence: high
updated: 2026-05-31T..Z
---
본문 (한국어 위주, D8)
```

SQLite (D4/D8/D9):
```sql
CREATE VIRTUAL TABLE notes_fts USING fts5(
  id UNINDEXED, title, body, tags, tokenize = "trigram"   -- 한국어(D8)
);
CREATE VIRTUAL TABLE notes_vec USING vec0(id TEXT, embedding FLOAT[N]);  -- 로컬 임베딩(D9)
CREATE TABLE kv (key TEXT, value TEXT, scope TEXT, updated TEXT);
CREATE TABLE symbols (name, kind, file, line, signature, subsystem);
CREATE TABLE links (src, dst, rel);
```

---

## 2. 미정 — 다음 인터뷰 라운드에서 결정할 항목

> ⚠️ 아래는 **아직 사용자 확정 전**이다. (이전 답변으로 착각해 채웠던 내용은 제거함.)

- 충돌 처리 (멀티 머신 git 머지: LWW / 머지 마커 / append-only)
- 지식 부패 대응 (심볼 닻 검증·stale 플래그 방식)
- 비밀/보안 (커밋 전 비밀 스캔·차단 여부, 민감 노트 분리)
- clangd용 compile_commands.json 준비 방식 (커널 내장 스크립트 등)
- MCP 도구 입자(소수 coarse / 다수 fine / 중간 그룹)
- 커널 다중 버전 지원 여부 (단일 트리 / 여러 버전 키잉)
- 노트 ID 체계 (슬러그 규칙·충돌 회피)
- Rust 바이너리 배포 방식 (cargo install / 릴리스 바이너리)
- 레포 구조 (도구와 데이터 한 레포 / 분리)
- KV 스코프 (글로벌 / 프로젝트 / 머신)
- 도구 출력 토큰 상한 정책
- 임베딩 모델 선택 (로컬 onnx/fastembed 등)
- 첫 구현 마일스톤 우선순위
