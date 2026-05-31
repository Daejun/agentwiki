---
id: mm-example
title: do_mmap의 mmap_lock 규약 (예시 노트)
type: gotcha
subsystem: mm
tags: [locking, mmap]
code_refs:
  - sym: do_mmap
    file: mm/mmap.c
    sig_hash: "0000"
confidence: high
---
## 2026-05-31T00:00:00Z · host=example · confidence=high
이것은 노트 형식을 보여주는 예시다(M2에서 심볼 닻 검증으로 sig_hash를 채운다).
본문은 한국어 위주(D8)로 작성한다. 핵심 사실·함정·근거를 간결히 적는다.

## 2026-06-01T00:00:00Z · host=example · confidence=med
append-only 엔트리 로그(D13): 기존 줄을 고치지 않고 새 엔트리를 덧붙인다.
이렇게 하면 멀티머신 git 머지 충돌이 사실상 사라진다.
