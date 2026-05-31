//! 노트 모델 (D4/D10/D13/D14/D21).
//!
//! 노트 = 마크다운 파일이며 **진실의 원천**이다. 본문은 시간순 **append-only
//! 엔트리 로그**(D13)다. 같은 주제에 사실을 덧붙일 때 기존 줄을 고치지 않고
//! 새 엔트리를 append 하므로 git 머지 충돌이 사실상 사라진다.
//!
//! 파일 형식:
//! ```text
//! ---
//! id: mm-a1b2c3
//! title: do_mmap의 mmap_lock 규약
//! type: gotcha
//! subsystem: mm
//! tags: [locking, mmap]
//! code_refs:
//!   - sym: do_mmap
//!     file: mm/mmap.c
//!     sig_hash: "ab12"
//! confidence: high
//! ---
//! ## 2026-05-31T00:00:00Z · host=devbox · confidence=high
//! 본문 엔트리 1 ...
//!
//! ## 2026-06-02T00:00:00Z · host=laptop · confidence=med
//! 본문 엔트리 2 (append) ...
//! ```

use serde::{Deserialize, Serialize};

use crate::error::{AwError, Result};

/// 노트의 종류 (D 노트 타입). 자유 문자열도 허용하되 알려진 값을 권장.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum NoteType {
    #[default]
    Concept,
    Gotcha,
    Decision,
    Howto,
    SymbolNote,
    #[serde(untagged)]
    Other(String),
}

/// 코드 심볼 닻 (D14). 라인이 아니라 심볼 이름에 닻을 내린다.
/// `sig_hash`는 인덱싱된 시그니처 해시로, 코드가 바뀌면 stale 판정에 쓰인다.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodeRef {
    pub sym: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sig_hash: Option<String>,
}

/// YAML frontmatter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frontmatter {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub r#type: NoteType,
    #[serde(default)]
    pub subsystem: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub code_refs: Vec<CodeRef>,
    /// high | med | low — 자유 문자열.
    #[serde(default)]
    pub confidence: String,
}

/// append-only 본문의 한 엔트리.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// 엔트리 헤더 라인(`## ...`)에서 파싱한 원본 메타 텍스트.
    pub header: String,
    /// 엔트리 본문(헤더 다음 줄부터 다음 엔트리 전까지).
    pub body: String,
}

/// 파싱된 노트 전체.
#[derive(Debug, Clone)]
pub struct Note {
    pub front: Frontmatter,
    /// append-only 엔트리들(시간순).
    pub entries: Vec<Entry>,
    /// 디스크 경로(있으면).
    pub path: Option<String>,
}

impl Note {
    /// 모든 엔트리 본문을 합친 검색용 텍스트.
    pub fn body_text(&self) -> String {
        let mut s = String::new();
        for e in &self.entries {
            s.push_str(&e.header);
            s.push('\n');
            s.push_str(&e.body);
            s.push('\n');
        }
        s
    }

    /// 마크다운 파일 전체 문자열로 직렬화.
    pub fn to_markdown(&self) -> Result<String> {
        let yaml = serde_yaml::to_string(&self.front)?;
        let mut out = String::new();
        out.push_str("---\n");
        out.push_str(&yaml);
        if !yaml.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("---\n");
        for e in &self.entries {
            out.push_str("## ");
            out.push_str(e.header.trim());
            out.push('\n');
            out.push_str(e.body.trim_end());
            out.push_str("\n\n");
        }
        Ok(out)
    }

    /// 마크다운 파일 문자열을 파싱한다.
    pub fn parse(content: &str, path: Option<String>) -> Result<Note> {
        let mkerr = |msg: &str| AwError::NoteParse {
            path: path.clone().unwrap_or_default(),
            msg: msg.to_string(),
        };

        let rest = content
            .strip_prefix("---")
            .ok_or_else(|| mkerr("missing frontmatter start '---'"))?;
        // frontmatter 종료 구분자를 찾는다: 줄 시작의 "---".
        let end = find_fence(rest).ok_or_else(|| mkerr("missing frontmatter end '---'"))?;
        let yaml = &rest[..end.0];
        let body = &rest[end.1..];

        let front: Frontmatter = serde_yaml::from_str(yaml.trim())?;
        if front.id.trim().is_empty() {
            return Err(mkerr("frontmatter 'id' is empty"));
        }
        let entries = parse_entries(body);
        Ok(Note {
            front,
            entries,
            path,
        })
    }
}

/// 본문에서 첫 줄경계 "---"의 (시작오프셋, 끝오프셋[개행 뒤])을 찾는다.
fn find_fence(s: &str) -> Option<(usize, usize)> {
    let mut idx = 0usize;
    for line in s.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" {
            return Some((idx, idx + line.len()));
        }
        idx += line.len();
    }
    None
}

/// "## ..." 헤더로 구분되는 append-only 엔트리들을 파싱.
/// 헤더가 하나도 없으면 본문 전체를 단일 엔트리로 본다.
fn parse_entries(body: &str) -> Vec<Entry> {
    let body = body.trim_start_matches('\n');
    let mut entries = Vec::new();
    let mut cur: Option<Entry> = None;

    for line in body.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if let Some(rest) = trimmed.strip_prefix("## ") {
            if let Some(e) = cur.take() {
                entries.push(e);
            }
            cur = Some(Entry {
                header: rest.trim().to_string(),
                body: String::new(),
            });
        } else if let Some(e) = cur.as_mut() {
            e.body.push_str(line);
        } else if !trimmed.trim().is_empty() {
            // 헤더 없이 시작하는 본문 → 무제목 엔트리.
            cur = Some(Entry {
                header: String::new(),
                body: line.to_string(),
            });
        }
    }
    if let Some(e) = cur.take() {
        entries.push(e);
    }
    for e in entries.iter_mut() {
        e.body = e.body.trim_end().to_string();
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"---
id: mm-a1b2c3
title: do_mmap의 mmap_lock 규약
type: gotcha
subsystem: mm
tags: [locking, mmap]
code_refs:
  - sym: do_mmap
    file: mm/mmap.c
    sig_hash: ab12
confidence: high
---
## 2026-05-31T00:00:00Z · host=devbox · confidence=high
do_mmap 진입 시 호출자가 mmap_write_lock 보유 가정.

## 2026-06-02T00:00:00Z · host=laptop · confidence=med
예외: nommu 빌드에서는 다르다.
"#;

    #[test]
    fn parses_frontmatter_and_entries() {
        let n = Note::parse(SAMPLE, Some("mm/mm-a1b2c3.md".into())).unwrap();
        assert_eq!(n.front.id, "mm-a1b2c3");
        assert_eq!(n.front.subsystem, "mm");
        assert_eq!(n.front.tags, vec!["locking", "mmap"]);
        assert_eq!(n.front.code_refs.len(), 1);
        assert_eq!(n.front.code_refs[0].sym, "do_mmap");
        assert_eq!(n.entries.len(), 2);
        assert!(n.entries[0].body.contains("mmap_write_lock"));
        assert!(n.entries[1].header.contains("laptop"));
    }

    #[test]
    fn roundtrips_through_markdown() {
        let n = Note::parse(SAMPLE, None).unwrap();
        let md = n.to_markdown().unwrap();
        let n2 = Note::parse(&md, None).unwrap();
        assert_eq!(n2.front.id, n.front.id);
        assert_eq!(n2.entries.len(), n.entries.len());
        assert!(n.body_text().contains("nommu"));
    }

    #[test]
    fn missing_fence_errors() {
        let bad = "---\nid: x\ntitle: y\n";
        assert!(Note::parse(bad, None).is_err());
    }
}
