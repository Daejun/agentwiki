//! SQLite 파생 인덱스 (D4/D8/D9/D22).
//!
//! `index.db`는 항상 마크다운에서 재생성 가능한 **파생물**이다. 따라서 git으로
//! 동기화하지 않고(D5) 각 머신에서 수동 `reindex`(D12)로 다시 만든다.
//!
//! 구성:
//! - `notes` / `notes_fts`(trigram, 한국어 D8) — 전문 검색
//! - `notes_vec`(임베딩 BLOB, D9/D18) — 벡터 검색(임베딩 활성 시)
//! - `kv`(scope: global/project/machine, D22)
//! - `symbols`(ctags, M2) / `links`

use rusqlite::{params, Connection, OptionalExtension};

use crate::embed::{cosine, Embedder};
use crate::error::Result;
use crate::note::Note;

/// 인덱스 핸들.
pub struct Index {
    conn: Connection,
}

/// 검색 결과 1건.
#[derive(Debug, Clone)]
pub struct Hit {
    pub id: String,
    pub title: String,
    pub subsystem: String,
    pub snippet: String,
    /// 융합 점수(높을수록 관련).
    pub score: f32,
}

impl Index {
    /// 파일 기반 인덱스를 연다(없으면 생성).
    pub fn open(path: &str) -> Result<Index> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }

    /// 인메모리 인덱스(테스트용).
    pub fn open_in_memory() -> Result<Index> {
        let conn = Connection::open_in_memory()?;
        Self::init(conn)
    }

    /// 내부 연결 접근(스토어의 다이제스트 쿼리 등에서 사용).
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    fn init(conn: Connection) -> Result<Index> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS notes (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                type TEXT,
                subsystem TEXT,
                tags TEXT,
                confidence TEXT,
                path TEXT,
                updated TEXT
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(
                id UNINDEXED, title, body, tags, tokenize = 'trigram'
            );
            CREATE TABLE IF NOT EXISTS notes_vec (
                id TEXT PRIMARY KEY,
                dim INTEGER NOT NULL,
                embedding BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS kv (
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                scope TEXT NOT NULL,
                updated TEXT,
                PRIMARY KEY (key, scope)
            );
            CREATE TABLE IF NOT EXISTS symbols (
                name TEXT, kind TEXT, file TEXT, line INTEGER,
                end_line INTEGER, signature TEXT, sig_hash TEXT, subsystem TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_sym_name ON symbols(name);
            CREATE TABLE IF NOT EXISTS links (
                src TEXT, dst TEXT, rel TEXT
            );
            "#,
        )?;
        Ok(Index { conn })
    }

    /// 노트 하나를 인덱스에 upsert 한다(메타/FTS/임베딩).
    pub fn upsert_note(&self, note: &Note, embedder: &dyn Embedder) -> Result<()> {
        let f = &note.front;
        let tags = f.tags.join(" ");
        let body = note.body_text();
        let type_s = serde_json::to_string(&f.r#type)
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();

        self.conn.execute(
            "INSERT INTO notes(id,title,type,subsystem,tags,confidence,path,updated)
             VALUES(?1,?2,?3,?4,?5,?6,?7,datetime('now'))
             ON CONFLICT(id) DO UPDATE SET
               title=?2,type=?3,subsystem=?4,tags=?5,confidence=?6,path=?7,updated=datetime('now')",
            params![f.id, f.title, type_s, f.subsystem, tags, f.confidence, note.path],
        )?;

        // FTS는 외부콘텐츠가 아니므로 delete 후 insert 로 갱신.
        self.conn
            .execute("DELETE FROM notes_fts WHERE id=?1", params![f.id])?;
        self.conn.execute(
            "INSERT INTO notes_fts(id,title,body,tags) VALUES(?1,?2,?3,?4)",
            params![f.id, f.title, body, tags],
        )?;

        if embedder.enabled() {
            let text = format!("{}\n{}", f.title, body);
            let vecs = embedder.embed(&[text]);
            if let Some(v) = vecs.into_iter().next() {
                let blob = f32_to_blob(&v);
                self.conn.execute(
                    "INSERT INTO notes_vec(id,dim,embedding) VALUES(?1,?2,?3)
                     ON CONFLICT(id) DO UPDATE SET dim=?2, embedding=?3",
                    params![f.id, v.len() as i64, blob],
                )?;
            }
        }
        Ok(())
    }

    /// 인덱스에서 노트를 제거한다.
    pub fn remove_note(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM notes WHERE id=?1", params![id])?;
        self.conn
            .execute("DELETE FROM notes_fts WHERE id=?1", params![id])?;
        self.conn
            .execute("DELETE FROM notes_vec WHERE id=?1", params![id])?;
        Ok(())
    }

    /// 노트 메타 행 수.
    pub fn note_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM notes", [], |r| r.get(0))?)
    }

    /// 하이브리드 검색 (D9): FTS5 + 벡터 결과를 RRF로 융합.
    ///
    /// `subsystem`이 주어지면 해당 서브시스템으로 한정한다(D10 검색 범위 좁히기).
    pub fn search(
        &self,
        query: &str,
        subsystem: Option<&str>,
        limit: usize,
        embedder: &dyn Embedder,
    ) -> Result<Vec<Hit>> {
        let mut fts = self.fts_search(query, subsystem, limit * 4)?;
        // 한국어(D8): trigram은 3글자 미만 토큰을 매치하지 못한다. 짧은 토큰이
        // 있으면 LIKE 부분문자열 폴백으로 보강한다.
        if has_short_token(query) {
            let like = self.like_search(query, subsystem, limit * 4)?;
            for id in like {
                if !fts.contains(&id) {
                    fts.push(id);
                }
            }
        }
        let vec = if embedder.enabled() {
            self.vec_search(query, subsystem, limit * 4, embedder)?
        } else {
            Vec::new()
        };
        let fused = rrf_fuse(&fts, &vec, limit);
        self.hydrate(fused)
    }

    /// FTS5 검색. 랭크 순으로 id 목록 반환.
    fn fts_search(
        &self,
        query: &str,
        subsystem: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>> {
        let match_q = sanitize_fts_query(query);
        if match_q.is_empty() {
            return Ok(Vec::new());
        }
        let sql = "SELECT f.id FROM notes_fts f \
                   JOIN notes n ON n.id = f.id \
                   WHERE notes_fts MATCH ?1 \
                   AND (?2 IS NULL OR n.subsystem = ?2) \
                   ORDER BY bm25(notes_fts) ASC LIMIT ?3";
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params![match_q, subsystem, limit as i64], |r| {
            r.get::<_, String>(0)
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// 짧은 질의(트라이그램 미만)용 LIKE 부분문자열 폴백. notes 메타+FTS 본문에서
    /// 각 토큰을 부분문자열로 찾는다(AND). 한국어 2글자어 대응(D8).
    fn like_search(
        &self,
        query: &str,
        subsystem: Option<&str>,
        limit: usize,
    ) -> Result<Vec<String>> {
        let toks: Vec<String> = query
            .split_whitespace()
            .filter(|t| t.chars().count() < 3 && t.chars().any(|c| c.is_alphanumeric()))
            .map(|t| t.to_string())
            .collect();
        if toks.is_empty() {
            return Ok(Vec::new());
        }
        let mut out: Vec<String> = Vec::new();
        for tok in toks {
            let pat = format!("%{tok}%");
            let sql = "SELECT f.id FROM notes_fts f JOIN notes n ON n.id=f.id \
                       WHERE (f.title LIKE ?1 OR f.body LIKE ?1 OR f.tags LIKE ?1) \
                       AND (?2 IS NULL OR n.subsystem = ?2) LIMIT ?3";
            let mut stmt = self.conn.prepare(sql)?;
            let rows = stmt.query_map(params![pat, subsystem, limit as i64], |r| {
                r.get::<_, String>(0)
            })?;
            let ids: Vec<String> = rows.collect::<std::result::Result<Vec<_>, _>>()?;
            if out.is_empty() {
                out = ids;
            } else {
                out.retain(|id| ids.contains(id)); // AND 결합
            }
        }
        Ok(out)
    }

    /// 벡터 검색. 임베딩 활성일 때만 호출됨. 코사인 유사도 정렬.
    fn vec_search(
        &self,
        query: &str,
        subsystem: Option<&str>,
        limit: usize,
        embedder: &dyn Embedder,
    ) -> Result<Vec<String>> {
        let qv = match embedder.embed(&[query.to_string()]).into_iter().next() {
            Some(v) if !v.is_empty() => v,
            _ => return Ok(Vec::new()),
        };
        let sql = "SELECT v.id, v.embedding FROM notes_vec v \
                   JOIN notes n ON n.id = v.id \
                   WHERE (?1 IS NULL OR n.subsystem = ?1)";
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(params![subsystem], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?;
        let mut scored: Vec<(String, f32)> = Vec::new();
        for row in rows {
            let (id, blob) = row?;
            let v = blob_to_f32(&blob);
            scored.push((id, cosine(&qv, &v)));
        }
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        Ok(scored.into_iter().map(|(id, _)| id).collect())
    }

    /// id 목록 → Hit(제목/서브시스템/스니펫) 로 채운다.
    fn hydrate(&self, scored: Vec<(String, f32)>) -> Result<Vec<Hit>> {
        let mut hits = Vec::new();
        for (id, score) in scored {
            let row = self
                .conn
                .query_row(
                    "SELECT title, subsystem FROM notes WHERE id=?1",
                    params![id],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
                )
                .optional()?;
            let Some((title, subsystem)) = row else {
                continue;
            };
            let snippet: Option<String> = self
                .conn
                .query_row(
                    "SELECT snippet(notes_fts, 2, '[', ']', ' … ', 8) \
                     FROM notes_fts WHERE id=?1",
                    params![id],
                    |r| r.get(0),
                )
                .optional()?;
            hits.push(Hit {
                id,
                title,
                subsystem,
                snippet: snippet.unwrap_or_default(),
                score,
            });
        }
        Ok(hits)
    }

    // ---- 코드 심볼 (M2) ----

    /// ctags 결과로 symbols 테이블을 통째로 교체한다(수동 reindex, D12).
    pub fn replace_symbols(
        &self,
        defs: &[crate::code::SymbolDef],
        _src_root: &str,
    ) -> Result<()> {
        self.conn.execute("DELETE FROM symbols", [])?;
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO symbols(name,kind,file,line,end_line,signature,sig_hash) \
                 VALUES(?1,?2,?3,?4,?5,?6,?7)",
            )?;
            for d in defs {
                stmt.execute(params![
                    d.name,
                    d.kind,
                    d.file,
                    d.line,
                    d.end_line,
                    d.signature,
                    d.sig_hash()
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 심볼 이름으로 정의를 조회한다. 함수 등 정의성 kind 를 우선한다.
    pub fn lookup_symbols(
        &self,
        name: &str,
        limit: usize,
    ) -> Result<Vec<crate::code::SymbolDef>> {
        let mut stmt = self.conn.prepare(
            "SELECT name,kind,file,line,end_line,signature FROM symbols \
             WHERE name=?1 \
             ORDER BY (kind IN ('function','struct','macro','prototype')) DESC, line ASC \
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![name, limit as i64], |r| {
            Ok(crate::code::SymbolDef {
                name: r.get(0)?,
                kind: r.get(1)?,
                file: r.get(2)?,
                line: r.get(3)?,
                end_line: r.get(4)?,
                signature: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// 심볼 테이블 행 수.
    pub fn symbol_count(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |r| r.get(0))?)
    }

    // ---- KV (D22) ----

    /// KV 설정. scope: "global" | "project:<id>" | "machine:<host>".
    pub fn kv_set(&self, key: &str, value: &str, scope: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO kv(key,value,scope,updated) VALUES(?1,?2,?3,datetime('now'))
             ON CONFLICT(key,scope) DO UPDATE SET value=?2, updated=datetime('now')",
            params![key, value, scope],
        )?;
        Ok(())
    }

    /// KV 조회. scope 우선순위: machine → project → global (구체 우선, D22).
    /// `prefer`에 현재 machine/project 스코프 문자열을 우선순위 순으로 넘긴다.
    pub fn kv_get(&self, key: &str, prefer: &[String]) -> Result<Option<String>> {
        for scope in prefer {
            if let Some(v) = self.kv_get_scoped(key, scope)? {
                return Ok(Some(v));
            }
        }
        self.kv_get_scoped(key, "global")
    }

    fn kv_get_scoped(&self, key: &str, scope: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM kv WHERE key=?1 AND scope=?2",
                params![key, scope],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }
}

/// FTS5 MATCH 안전화: 토큰을 따옴표로 감싸 구문오류를 막는다.
fn sanitize_fts_query(q: &str) -> String {
    let toks: Vec<String> = q
        .split_whitespace()
        .filter(|t| t.chars().any(|c| c.is_alphanumeric()))
        .map(|t| {
            let cleaned: String = t.chars().filter(|c| *c != '"').collect();
            format!("\"{cleaned}\"")
        })
        .collect();
    toks.join(" OR ")
}

/// 질의에 trigram 미만(3글자 미만) 토큰이 있는지.
fn has_short_token(q: &str) -> bool {
    q.split_whitespace()
        .any(|t| t.chars().any(|c| c.is_alphanumeric()) && t.chars().count() < 3)
}

/// Reciprocal Rank Fusion. 두 랭킹 목록을 합쳐 상위 limit 반환.
fn rrf_fuse(a: &[String], b: &[String], limit: usize) -> Vec<(String, f32)> {
    use std::collections::HashMap;
    const K: f32 = 60.0;
    let mut score: HashMap<&String, f32> = HashMap::new();
    for (rank, id) in a.iter().enumerate() {
        *score.entry(id).or_insert(0.0) += 1.0 / (K + rank as f32 + 1.0);
    }
    for (rank, id) in b.iter().enumerate() {
        *score.entry(id).or_insert(0.0) += 1.0 / (K + rank as f32 + 1.0);
    }
    let mut v: Vec<(String, f32)> = score.into_iter().map(|(k, s)| (k.clone(), s)).collect();
    v.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
    v.truncate(limit);
    v
}

fn f32_to_blob(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }
    out
}

fn blob_to_f32(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::NoopEmbedder;
    use crate::note::Note;

    fn sample(id: &str, sub: &str, title: &str, body: &str) -> Note {
        let md = format!(
            "---\nid: {id}\ntitle: {title}\ntype: gotcha\nsubsystem: {sub}\ntags: [x]\nconfidence: high\n---\n## 2026-01-01T00:00:00Z\n{body}\n"
        );
        Note::parse(&md, Some(format!("{sub}/{id}.md"))).unwrap()
    }

    #[test]
    fn upsert_and_fts_search() {
        let idx = Index::open_in_memory().unwrap();
        let emb = NoopEmbedder;
        idx.upsert_note(
            &sample("mm-1", "mm", "mmap lock", "do_mmap takes mmap_write_lock"),
            &emb,
        )
        .unwrap();
        idx.upsert_note(
            &sample("net-1", "net", "socket buffer", "skb allocation in softirq"),
            &emb,
        )
        .unwrap();
        assert_eq!(idx.note_count().unwrap(), 2);

        let hits = idx.search("mmap_write_lock", None, 5, &emb).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].id, "mm-1");
    }

    #[test]
    fn subsystem_filter() {
        let idx = Index::open_in_memory().unwrap();
        let emb = NoopEmbedder;
        idx.upsert_note(&sample("mm-1", "mm", "alloc", "page allocation"), &emb)
            .unwrap();
        idx.upsert_note(&sample("net-1", "net", "alloc", "skb allocation"), &emb)
            .unwrap();
        let hits = idx.search("allocation", Some("net"), 5, &emb).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "net-1");
    }

    #[test]
    fn short_korean_query_falls_back_to_like() {
        let idx = Index::open_in_memory().unwrap();
        let emb = NoopEmbedder;
        idx.upsert_note(
            &sample("mm-1", "mm", "mmap_lock 규약", "호출자가 락을 보유"),
            &emb,
        )
        .unwrap();
        // "규약"은 2글자 → trigram 미스. LIKE 폴백으로 잡혀야 한다.
        let hits = idx.search("규약", None, 5, &emb).unwrap();
        assert_eq!(hits.len(), 1, "2글자 한국어 질의가 폴백으로 매치되어야 함");
        assert_eq!(hits[0].id, "mm-1");
    }

    #[test]
    fn upsert_is_idempotent() {
        let idx = Index::open_in_memory().unwrap();
        let emb = NoopEmbedder;
        let n = sample("mm-1", "mm", "t", "body text here");
        idx.upsert_note(&n, &emb).unwrap();
        idx.upsert_note(&n, &emb).unwrap();
        assert_eq!(idx.note_count().unwrap(), 1);
        let hits = idx.search("body", None, 5, &emb).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn kv_scope_precedence() {
        let idx = Index::open_in_memory().unwrap();
        idx.kv_set("src", "/global/src", "global").unwrap();
        idx.kv_set("src", "/machine/src", "machine:devbox").unwrap();
        let prefer = vec!["machine:devbox".to_string(), "project:k".to_string()];
        assert_eq!(
            idx.kv_get("src", &prefer).unwrap().as_deref(),
            Some("/machine/src")
        );
        // machine 미존재 키는 global 로 폴백.
        idx.kv_set("rule", "tabs", "global").unwrap();
        assert_eq!(idx.kv_get("rule", &prefer).unwrap().as_deref(), Some("tabs"));
    }
}
