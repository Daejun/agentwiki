# agentwiki

코딩 에이전트(Claude Code 등)가 **세션이 바뀌어도 지식을 잃지 않고**, 세션 간
지식을 효율적으로 공유하며, 특히 **리눅스 커널처럼 거대한 코드베이스에서 컨텍스트
낭비를 최소화**하도록 만든 로컬 지식 저장소(위키 + KV + 경량 인덱스)입니다.

- 설계 배경·기존 프로젝트 조사: [`docs/design.md`](docs/design.md)
- 확정 설계 결정 24개(ADR): [`docs/decisions.md`](docs/decisions.md)

## 핵심 아이디어

- **마크다운이 진실의 원천, SQLite는 파생 인덱스** (D4). 마크다운만 git으로
  동기화하고(D5) 인덱스는 각 머신에서 재생성한다(D12).
- **노트는 append-only 엔트리 로그** (D13) → 멀티머신 git 머지 충돌 최소화.
- **검색 우선, 주입 최소** (D11). 세션 시작엔 목차만, 본문은 필요할 때 회수.
- **코드는 심볼에 닻** (D14/D21) → 라인 드리프트에 강함(M2).
- **FTS5(trigram, 한국어 D8) + 로컬 임베딩 하이브리드** (D9/D18) 검색.

## 구성 (Cargo 워크스페이스)

| 크레이트 | 역할 |
|---|---|
| `aw-core` | 노트 파서, SQLite 인덱스, 하이브리드 검색, KV, 비밀 스캔 |
| `aw-mcp` | MCP 서버(stdio, JSON-RPC). 도구 그룹 `wiki`/`kv`/`admin` (D17) |
| `aw-cli` | 디버깅/스크립트용 얇은 CLI (`aw`) |

## 빌드 / 테스트

```sh
cargo build --workspace
cargo test --workspace
```

> 임베딩(fastembed-rs)은 모델 다운로드가 필요하므로 `aw-core`의 `embeddings`
> feature로 게이트됩니다. 기본 빌드는 FTS5 단독으로 오프라인에서 동작합니다.

## CLI 사용 예

```sh
AW=./target/debug/aw
$AW --root ./data note mm "mmap_lock 규약" "do_mmap 진입 시 mmap_write_lock 보유 필요" gotcha high
$AW --root ./data search 규약           # 2글자 한국어도 폴백 검색
$AW --root ./data kv-set src /home/me/linux machine:$(hostname)
$AW --root ./data digest                # 세션 시작용 최소 목차
```

## MCP 서버 (Claude Code 연동)

`aw-mcp`는 stdio JSON-RPC MCP 서버입니다. 노트 루트는 `AW_ROOT`로 지정합니다.

```jsonc
// .mcp.json (예시)
{
  "mcpServers": {
    "agentwiki": {
      "command": "/path/to/aw-mcp",
      "env": { "AW_ROOT": "/path/to/agentwiki-data" }
    }
  }
}
```

노출 도구(중간 입자 + `op` 분기, D17):

- `wiki` — `op=search|get|propose|commit`. `propose`는 초안만 보여주고 저장하지
  않으며, 승인 후 `commit`이 append-only로 기록(반자동 캡처, D6).
- `kv` — `op=get|set`, `scope=global|project:<id>|machine:<host>` (D22).
- `admin` — `op=reindex|digest|scan-secrets`.

## 로드맵

- **M1 (현재)**: 위키/메모리 코어 — 노트·인덱스·검색·KV·반자동 캡처·다이제스트.
- **M2**: ctags/cscope 코드 인덱스 + `code`(where/show/xref) + `recall` (커널 효율).
- **M3**: clangd 승격, musl 릴리스 바이너리 + cargo install, git 동기화 워크플로.
- **M4**: fastembed-rs 임베딩 활성화, append 로그 compact, 멀티 버전 키잉.
