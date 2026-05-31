//! 위키 스토어 — 노트 디렉터리 + 파생 인덱스의 결합 (M1 코어).
//!
//! - 마크다운 노트(진실)를 `notes/<subsystem>/<id>.md`에서 읽고(D10),
//! - 수동 `reindex`(D12)로 SQLite 인덱스를 재생성하고,
//! - 반자동 캡처(D6): `propose` → 사용자/에이전트 승인 → `commit`(append, D13),
//! - 세션용 `digest`(D11)와 비밀 스캔(D15)을 제공한다.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::embed::{Embedder, NoopEmbedder};
use crate::error::{AwError, Result};
use crate::index::{Hit, Index};
use crate::note::{Entry, Frontmatter, Note, NoteType};
use crate::secrets;

/// 노트 디렉터리 + 인덱스.
pub struct Store {
    root: PathBuf,
    index: Index,
    embedder: Box<dyn Embedder>,
}

/// 반자동 캡처용 초안(아직 디스크에 쓰지 않음, D6).
#[derive(Debug, Clone)]
pub struct Proposal {
    pub id: String,
    pub path: PathBuf,
    /// 새 노트면 전체 마크다운, 기존 노트면 추가될 엔트리 포함 전체 미리보기.
    pub preview: String,
    /// 이 커밋이 기존 노트에 append 하는지 여부(D13).
    pub is_append: bool,
    /// 비밀 스캔 경고(차단하지 않음, D15).
    pub secret_findings: Vec<secrets::Finding>,
}

impl Store {
    /// 노트 루트와 인덱스 경로로 스토어를 연다.
    pub fn open(root: impl AsRef<Path>, index_path: &str) -> Result<Store> {
        let index = Index::open(index_path)?;
        Ok(Store {
            root: root.as_ref().to_path_buf(),
            index,
            embedder: Box::new(NoopEmbedder),
        })
    }

    /// 테스트/임시용: 루트 + 인메모리 인덱스.
    pub fn open_in_memory(root: impl AsRef<Path>) -> Result<Store> {
        Ok(Store {
            root: root.as_ref().to_path_buf(),
            index: Index::open_in_memory()?,
            embedder: Box::new(NoopEmbedder),
        })
    }

    pub fn with_embedder(mut self, e: Box<dyn Embedder>) -> Self {
        self.embedder = e;
        self
    }

    pub fn index(&self) -> &Index {
        &self.index
    }

    fn notes_dir(&self) -> PathBuf {
        self.root.join("notes")
    }

    /// 디스크의 모든 노트를 스캔해 인덱스를 재생성한다(D12, 수동 reindex).
    /// 프론트매터가 없는 마크다운(예: README)은 노트가 아니므로 건너뛴다.
    /// 반환: 인덱싱된 노트 수.
    pub fn reindex(&self) -> Result<usize> {
        let (count, _skipped) = self.reindex_verbose()?;
        Ok(count)
    }

    /// reindex 하되 건너뛴 비노트 파일 경로 목록도 함께 반환한다.
    pub fn reindex_verbose(&self) -> Result<(usize, Vec<String>)> {
        let dir = self.notes_dir();
        if !dir.exists() {
            return Ok((0, Vec::new()));
        }
        let mut count = 0;
        let mut skipped = Vec::new();
        for entry in WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()) {
            let p = entry.path();
            if !p.extension().map(|e| e == "md").unwrap_or(false) {
                continue;
            }
            let content = std::fs::read_to_string(p)?;
            // 프론트매터가 없으면 노트가 아니다(README 등) → 조용히 건너뛴다.
            if !content.trim_start().starts_with("---") {
                skipped.push(p.to_string_lossy().into_owned());
                continue;
            }
            match Note::parse(&content, Some(p.to_string_lossy().into_owned())) {
                Ok(note) => {
                    self.index.upsert_note(&note, self.embedder.as_ref())?;
                    count += 1;
                }
                Err(_) => skipped.push(p.to_string_lossy().into_owned()),
            }
        }
        Ok((count, skipped))
    }

    /// 검색 (D9 하이브리드, 출력 상한은 호출자가 limit으로 제어 D23).
    pub fn search(&self, query: &str, subsystem: Option<&str>, limit: usize) -> Result<Vec<Hit>> {
        self.index
            .search(query, subsystem, limit, self.embedder.as_ref())
    }

    /// id로 노트 전체를 디스크에서 읽어 반환.
    pub fn get(&self, id: &str) -> Result<Note> {
        let path = self.path_for(id)?;
        let content = std::fs::read_to_string(&path)?;
        Note::parse(&content, Some(path.to_string_lossy().into_owned()))
    }

    /// id에 해당하는 파일 경로를 찾는다(서브시스템 디렉터리 순회).
    fn path_for(&self, id: &str) -> Result<PathBuf> {
        let dir = self.notes_dir();
        for entry in WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()) {
            let p = entry.path();
            if p.file_stem().map(|s| s == id).unwrap_or(false)
                && p.extension().map(|e| e == "md").unwrap_or(false)
            {
                return Ok(p.to_path_buf());
            }
        }
        Err(AwError::NotFound(format!("note id '{id}'")))
    }

    /// 새 노트 ID 생성: `<subsystem>-<짧은해시>` (D21).
    /// 해시는 제목+본문+서브시스템에서 파생(같은 입력=같은 id, 멀티머신 충돌 회피).
    pub fn make_id(subsystem: &str, title: &str, body: &str) -> String {
        let mut h = Sha256::new();
        h.update(subsystem.as_bytes());
        h.update(b"\0");
        h.update(title.as_bytes());
        h.update(b"\0");
        h.update(body.as_bytes());
        let digest = h.finalize();
        let hex: String = digest.iter().take(3).map(|b| format!("{b:02x}")).collect();
        format!("{subsystem}-{hex}")
    }

    /// 반자동 캡처 1단계: 초안을 만든다(디스크 미기록, D6).
    /// 같은 id가 이미 있으면 append 모드로 미리보기를 만든다(D13).
    #[allow(clippy::too_many_arguments)]
    pub fn propose(
        &self,
        subsystem: &str,
        title: &str,
        body: &str,
        note_type: NoteType,
        tags: Vec<String>,
        confidence: &str,
        id_override: Option<&str>,
    ) -> Result<Proposal> {
        let id = match id_override {
            Some(i) => i.to_string(),
            None => Self::make_id(subsystem, title, body),
        };
        let entry = Entry {
            header: timestamp_header(confidence),
            body: body.trim().to_string(),
        };

        // 기존 노트가 있으면 append, 없으면 새 노트.
        let existing = self.path_for(&id).ok();
        let (note, is_append, path) = if let Some(path) = existing {
            let content = std::fs::read_to_string(&path)?;
            let mut note = Note::parse(&content, Some(path.to_string_lossy().into_owned()))?;
            note.entries.push(entry);
            (note, true, path)
        } else {
            let front = Frontmatter {
                id: id.clone(),
                title: title.to_string(),
                r#type: note_type,
                subsystem: subsystem.to_string(),
                tags,
                code_refs: Vec::new(),
                confidence: confidence.to_string(),
            };
            let path = self
                .notes_dir()
                .join(subsystem)
                .join(format!("{id}.md"));
            (
                Note {
                    front,
                    entries: vec![entry],
                    path: Some(path.to_string_lossy().into_owned()),
                },
                false,
                path,
            )
        };

        let preview = note.to_markdown()?;
        let secret_findings = secrets::scan(&preview);
        Ok(Proposal {
            id,
            path,
            preview,
            is_append,
            secret_findings,
        })
    }

    /// 반자동 캡처 2단계: 승인된 초안을 디스크에 기록하고 인덱스를 갱신한다(D6).
    pub fn commit(&self, proposal: &Proposal) -> Result<()> {
        if let Some(parent) = proposal.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&proposal.path, &proposal.preview)?;
        let note = Note::parse(
            &proposal.preview,
            Some(proposal.path.to_string_lossy().into_owned()),
        )?;
        self.index.upsert_note(&note, self.embedder.as_ref())?;
        Ok(())
    }

    /// 세션 시작용 최소 다이제스트 (D11): 핀 KV + 최근/고확신 노트 제목·id 목록만.
    /// 본문은 넣지 않는다(토큰 절약) — 에이전트가 필요 시 get/recall 로 회수.
    pub fn digest(&self, max_notes: usize) -> Result<Digest_> {
        let conn = self.index.connection();
        let mut stmt = conn.prepare(
            "SELECT id, title, subsystem, confidence FROM notes \
             ORDER BY (confidence='high') DESC, updated DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([max_notes as i64], |r| {
            Ok(DigestNote {
                id: r.get(0)?,
                title: r.get(1)?,
                subsystem: r.get(2)?,
                confidence: r.get(3)?,
            })
        })?;
        let notes = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        let total = self.index.note_count()?;
        Ok(Digest_ { total, notes })
    }
}

/// 다이제스트의 노트 한 줄(제목+id만, 본문 없음 D11).
#[derive(Debug, Clone)]
pub struct DigestNote {
    pub id: String,
    pub title: String,
    pub subsystem: String,
    pub confidence: String,
}

/// 세션 시작 다이제스트.
#[derive(Debug, Clone)]
pub struct Digest_ {
    pub total: i64,
    pub notes: Vec<DigestNote>,
}

fn timestamp_header(confidence: &str) -> String {
    let now = time::OffsetDateTime::now_utc();
    let ts = now
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "unknown".into());
    let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into());
    format!("{ts} · host={host} · confidence={confidence}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir() -> PathBuf {
        let p = std::env::temp_dir().join(format!("awtest-{}", std::process::id()))
            .join(format!("{:?}", std::time::SystemTime::now()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn propose_commit_then_search() {
        let root = tmpdir();
        let store = Store::open_in_memory(&root).unwrap();
        let p = store
            .propose(
                "mm",
                "mmap_lock 규약",
                "do_mmap 진입 시 mmap_write_lock 필요",
                NoteType::Gotcha,
                vec!["locking".into()],
                "high",
                None,
            )
            .unwrap();
        assert!(!p.is_append);
        assert!(p.preview.contains("do_mmap"));
        store.commit(&p).unwrap();

        let hits = store.search("mmap_write_lock", None, 5).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, p.id);
    }

    #[test]
    fn second_propose_same_id_appends() {
        let root = tmpdir();
        let store = Store::open_in_memory(&root).unwrap();
        let p1 = store
            .propose("mm", "t", "first fact", NoteType::Gotcha, vec![], "high", None)
            .unwrap();
        store.commit(&p1).unwrap();

        // 같은 id로 append (D13 append-only).
        let p2 = store
            .propose(
                "mm",
                "t",
                "second fact appended",
                NoteType::Gotcha,
                vec![],
                "med",
                Some(&p1.id),
            )
            .unwrap();
        assert!(p2.is_append);
        store.commit(&p2).unwrap();

        let note = store.get(&p1.id).unwrap();
        assert_eq!(note.entries.len(), 2);
        assert!(note.body_text().contains("first fact"));
        assert!(note.body_text().contains("second fact"));
    }

    #[test]
    fn reindex_picks_up_files() {
        let root = tmpdir();
        let store = Store::open_in_memory(&root).unwrap();
        let p = store
            .propose("net", "skb", "skb in softirq", NoteType::Concept, vec![], "med", None)
            .unwrap();
        store.commit(&p).unwrap();

        // 새 스토어로 열어 reindex 만으로 검색되는지(파생 인덱스 재생성 D12).
        let store2 = Store::open_in_memory(&root).unwrap();
        let n = store2.reindex().unwrap();
        assert_eq!(n, 1);
        let hits = store2.search("softirq", None, 5).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn reindex_skips_non_note_markdown() {
        let root = tmpdir();
        let store = Store::open_in_memory(&root).unwrap();
        let p = store
            .propose("mm", "t", "real note body", NoteType::Concept, vec![], "med", None)
            .unwrap();
        store.commit(&p).unwrap();
        // 노트가 아닌 README 를 같은 트리에 둔다.
        std::fs::write(
            root.join("notes").join("README.md"),
            "# 설명\n프론트매터 없는 일반 마크다운\n",
        )
        .unwrap();

        let store2 = Store::open_in_memory(&root).unwrap();
        let (count, skipped) = store2.reindex_verbose().unwrap();
        assert_eq!(count, 1, "노트 1개만 인덱싱");
        assert_eq!(skipped.len(), 1, "README 1개는 건너뜀");
    }

    #[test]
    fn secret_warning_surfaced_not_blocked() {
        let root = tmpdir();
        let store = Store::open_in_memory(&root).unwrap();
        let p = store
            .propose(
                "misc",
                "creds",
                "token: AKIAIOSFODNN7EXAMPLE leaked",
                NoteType::Howto,
                vec![],
                "low",
                None,
            )
            .unwrap();
        assert!(!p.secret_findings.is_empty()); // 경고는 뜨지만
        store.commit(&p).unwrap(); // 차단하지 않는다(D15).
    }
}
