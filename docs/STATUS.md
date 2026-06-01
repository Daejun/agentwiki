# agentwiki — 작업 인계 지침 (STATUS)

> **새 세션은 이 문서부터 읽으세요.** 클론 후 바로 이어서 작업할 수 있도록 현재
> 진행 상황·환경 설정·남은 작업·관례를 정리합니다.
> 최종 갱신: 2026-06-01 (커밋 `99f9d11` 기준)

---

## 0. 30초 요약

- **무엇**: 코딩 에이전트(Claude Code 등)가 세션 간 지식을 잃지 않도록 하는 로컬
  지식 저장소. 특히 **리눅스 커널 작업의 컨텍스트 낭비 최소화**가 핵심 목표.
- **어떻게**: 마크다운(진실) + SQLite(파생 인덱스) 하이브리드, MCP 서버로 노출,
  ctags/cscope로 코드 인덱싱, FTS5+로컬임베딩 하이브리드 검색.
- **상태**: M1·M2 완료, M3·M4 부분 완료. 34 단위테스트 + 41 스모크체크 통과,
  clippy 0 경고. **동작하는 프로토타입**.
- **설계 근거**: [`docs/decisions.md`](decisions.md)(확정 24개 결정), 조사는
  [`docs/design.md`](design.md). 사용법은 [`README.md`](../README.md).

---

## 1. 클론 후 첫 단계 (재현)

```sh
# 1) 코드 인덱스 도구 설치 (M2 기능에 필요. 없어도 M1은 동작)
sudo apt-get update && sudo apt-get install -y universal-ctags cscope

# 2) 빌드 + 전체 검증 (이게 통과하면 환경 정상)
cargo build --workspace
cargo test --workspace          # 34 unit tests
./scripts/smoke.sh              # 41 checks, 종료코드 0 = 전부 통과

# 3) 사람이 눈으로 보는 데모
./scripts/demo.sh
```

빌드 환경: **Rust 1.94+, crates.io 접근 필요**. SQLite는 `rusqlite`의 `bundled`
기능으로 소스 컴파일되므로 별도 설치 불필요(FTS5 포함). ctags/cscope는 M2 전용.

---

## 2. 현재 구조

```
crates/
  aw-core/   라이브러리 (모든 로직). 34 단위테스트.
    note.rs    노트 모델 + append-only 엔트리 로그 파서
    index.rs   SQLite: FTS5(trigram)+벡터+KV+symbols+links, 하이브리드 검색
    embed.rs   HashEmbedder(의존성0 로컬임베딩) + Embedder 트레이트
    store.rs   노트디렉터리+인덱스 결합: reindex/search/propose/commit/recall/compact/stale
    code.rs    ctags/cscope 어댑터: where/show(슬라이싱)/xref + 심볼닻 검증
    secrets.rs 비밀 스캔(경고만)
    error.rs
  aw-mcp/    MCP 서버 (JSON-RPC over stdio, 외부SDK 없음)
    tools.rs   도구 그룹 wiki/code/recall/kv/admin (op 분기)
  aw-cli/    디버깅용 `aw` CLI
scripts/
  smoke.sh             전체 자동 검증 (빌드+테스트+CLI+MCP+코드인덱스+회귀)
  demo.sh              사람용 시연
  session-start-hook.sh  Claude Code SessionStart 훅 (최소 다이제스트 주입)
.github/workflows/   ci.yml(빌드/clippy/test/smoke), release.yml(musl 바이너리)
docs/                decisions.md(ADR), design.md(조사), STATUS.md(이 문서)
notes/               예시 노트 + 레이아웃 스펙
```

---

## 3. 완료된 것 (마일스톤)

### M1 — 위키/메모리 코어 ✅
- 마크다운 노트(진실) + append-only 엔트리 로그(D13)
- SQLite 파생 인덱스: FTS5(trigram, 한국어 D8) + 2글자 한국어 LIKE 폴백
- 3스코프 KV(global/project/machine, D22), 우선순위 machine→project→global
- 반자동 캡처 propose→commit(D6), 비밀 스캔 경고만(D15)
- 서브시스템-해시 ID(D21), 최소 다이제스트(D11)

### M2 — 코드 인덱스 (커널 효율 핵심) ✅
- ctags 어댑터: `where`(정의위치), `show`(**함수 본문만 슬라이싱**, end 필드 사용)
- cscope `xref`(호출자/피호출자)
- `recall`(심볼→정의+닻노트+언급+stale 통합 회수)
- 심볼 닻 sig_hash 검증 → `stale` 플래그(D14, 지식 부패 감지)

### M3 — 패키징/통합 🟡 (clangd 제외 완료)
- CI(ci.yml) + 릴리스(release.yml, musl 바이너리 D24)
- SessionStart 훅 스크립트(D11)
- **미완**: clangd 승격(정밀 경로 D7)

### M4 — 확장 🟡 (고품질 임베딩 제외 완료)
- 하이브리드 벡터 검색 활성화: `HashEmbedder`(의존성0, 문자 트라이그램 해시),
  FTS5와 RRF 융합, 최소 코사인 임계값(0.15)으로 무관 노트 차단
- append 로그 compact(D13): 중복/빈 엔트리 제거
- **미완**: fastembed 고품질 다국어 모델(D18, `embeddings` feature 뒤 트레이트만)

### 버그 수정 (테스트 강화로 발견한 실제 결함 4건, 커밋 99f9d11)
1. 본문 속 `## 헤딩`이 엔트리로 오분리 → 타임스탬프 시작 라인만 엔트리로 인식
2. `make_id`가 body 포함 → 같은 주제가 분산. title만으로 파생하도록 수정
3. `brace_match_end`가 문자열/주석 속 중괄호 오인 → 리터럴/주석 스킵
4. digest 정렬이 reindex로 무의미화 → updated를 최신 엔트리 타임스탬프에서 파생

---

## 4. 남은 작업 (우선순위 순)

### P1 — 실전 검증 (가장 가치 높음, 아직 안 함)
- [ ] **실제 리눅스 커널 트리 대상 검증**. 지금까지 작은 합성 C 파일로만 테스트함.
      `code-reindex /path/to/linux`의 인덱싱 시간·메모리·심볼 수, `show`/`xref`의
      정확도를 실측. 커널 규모(수천만 줄)에서 ctags/cscope 동작 확인.
- [ ] **대용량 노트/인덱스 성능**: 노트 수천 개일 때 search/digest/reindex 시간 측정.
      필요시 reindex 증분화(현재 전체 재생성).

### P2 — 멀티머신 동기화 마무리 (D5/D13, 설계는 했으나 미구현)
- [ ] git 머지 시나리오 실제 테스트: 두 머신이 같은 노트에 append → 머지 충돌
      여부 확인. append-only라 충돌이 적어야 하지만 **frontmatter 동시 수정**
      (예: tags) 시 충돌 가능. LWW 머지 드라이버 또는 가이드 필요.
- [ ] 데이터 레포 분리(D20): 현재 notes/가 도구 레포에 같이 있음. 실사용 시
      `agentwiki-data` 별도 레포로 분리하는 절차/스크립트.

### P3 — clangd 승격 (D7, M3 잔여)
- [ ] `compile_commands.json` 연동(커널 `scripts/clang-tools/gen_compile_commands.py`).
- [ ] 정밀 질의(매크로 전개, 타입 추론) 시 ctags 대신 clangd LSP 사용 경로.
      주의: clangd는 커널 규모에서 메모리/시간 한계 — 좁은 하위트리에만 적용(D16).

### P4 — 고품질 임베딩 (D18, M4 잔여)
- [ ] `embeddings` feature에 fastembed-rs 실제 연결(multilingual-e5-small 등).
      현재 `embed.rs`의 `FastEmbedder`는 스텁(0벡터). 트레이트 계약은 고정됨.
      주의: ONNX 런타임 다운로드가 ephemeral 환경에서 불안정 → feature-gate 유지.
- [ ] notes_vec 차원이 임베더와 일치하는지 마이그레이션 처리(현재 BLOB 저장).

### P5 — 운영 편의
- [ ] `Cargo.lock` 추적 여부 결정(현재 .gitignore에 있음. 바이너리라 보통 커밋).
- [ ] MCP 도구에 사용 예시/에러 메시지 보강, 도구 출력 토큰 상한 실측 튜닝(D23).
- [ ] 멀티 커널 버전 키잉(D16): 현재 단일 트리 전제. 인덱스 경로 `<repo-id>/<sha>/`.

---

## 5. 작업 관례 (중요)

- **개발 브랜치**: `claude/llm-context-storage-plan-w9TaF`. 여기에 커밋·푸시.
  (다른 세션은 자신의 지정 브랜치 규칙을 따르되, 이어가려면 이 브랜치 기준)
- **모든 변경 후**: `cargo test --workspace` + `cargo clippy --workspace`(0 경고
  유지) + `./scripts/smoke.sh`(전부 통과) 를 돌리고 커밋.
- **버그를 고치면 회귀 테스트를 먼저 추가**(실패 확인)한 뒤 수정. smoke.sh에도
  CLI 레벨 회귀 체크 추가.
- **정직 원칙**: 미완/스텁/건너뛴 것은 문서에 명시. decisions.md §7.5에 구현 현황,
  이 문서 §4에 남은 작업을 항상 갱신.
- **파생물 비커밋**: `.aw/`, `*.db`, `tags`, `cscope.out`, `compile_commands.json`
  는 .gitignore됨(D5). 마크다운 노트만 동기화.
- **커밋 메시지**: 무엇을·왜. 버그 수정은 근본 원인과 재현 조건 명시.

---

## 6. 빠른 손익 지도 (어디를 건드리면 무엇이 깨지나)

| 바꾸려는 것 | 보는 파일 | 주의 |
|---|---|---|
| 노트 형식/파싱 | `note.rs` | `looks_like_entry_header`(엔트리 vs 헤딩 구분), roundtrip 테스트 |
| 검색 랭킹 | `index.rs` `search`/`rrf_fuse`/`vec_search` | 임계값 0.15, LIKE 폴백 조건 |
| ID 생성 규칙 | `store.rs` `make_id` | title만 사용(body 넣으면 버그2 재발) |
| 함수 슬라이싱 | `code.rs` `show_sym`/`brace_match_end` | end 필드 우선, 폴백은 리터럴 스킵 |
| 다이제스트 정렬 | `index.rs` upsert(updated), `store.rs` digest | updated=최신 엔트리 ts |
| MCP 도구 추가 | `aw-mcp/tools.rs` | `call_tool` 분기 + `tool_specs` 둘 다 갱신 |

---

## 7. 한 줄 명령 모음

```sh
cargo test --workspace && cargo clippy --workspace && ./scripts/smoke.sh   # 전체 검증
./target/debug/aw --root DATA note SUB "제목" "본문" gotcha high           # 노트 작성
./target/debug/aw --root DATA search "질의" [SUBSYS]                       # 검색
./target/debug/aw --root DATA code-reindex /path/to/src                    # 코드 인덱싱
./target/debug/aw --root DATA recall do_mmap                               # 통합 회수
AW_ROOT=DATA ./target/debug/aw-mcp                                         # MCP 서버 기동(stdio)
```
