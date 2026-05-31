//! aw-core — agentwiki 코어 라이브러리 (M1: 위키/메모리).
//!
//! 설계 결정은 `docs/decisions.md` 참조. 핵심:
//! - 마크다운(진실) + SQLite(파생 인덱스) 하이브리드 (D4)
//! - append-only 엔트리 로그 노트 (D13), 서브시스템-해시 ID (D21)
//! - FTS5(trigram, 한국어 D8) + 로컬 임베딩(D9/D18) 하이브리드 검색
//! - 반자동 캡처 propose→commit (D6), 비밀 스캔 경고만 (D15)
//! - 3스코프 KV (D22), 수동 reindex (D12), 최소 다이제스트 (D11)

pub mod embed;
pub mod error;
pub mod index;
pub mod note;
pub mod secrets;
pub mod store;

pub use embed::{Embedder, NoopEmbedder};
pub use error::{AwError, Result};
pub use index::{Hit, Index};
pub use note::{CodeRef, Entry, Frontmatter, Note, NoteType};
pub use store::{Digest_, DigestNote, Proposal, Store};
