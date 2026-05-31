//! 코드 인덱스 — ctags/cscope 어댑터 (M2, D7/D14/D16).
//!
//! 리눅스 커널 컨텍스트 낭비를 정면 대응한다:
//! - `where`: ctags 로 심볼 정의 위치(파일·라인·시그니처) — grep 대체
//! - `show`: ctags `end:` 필드로 **함수 본문만 슬라이싱** — whole-file read 금지
//! - `xref`: cscope 로 호출자/피호출자 — 정밀 크로스레퍼런스
//! - `verify_anchor`: 심볼 닻의 sig_hash 검증 → stale 판정 (D14)
//!
//! ctags/cscope 바이너리가 없으면 각 기능은 빈 결과/안내로 폴백한다(D7).

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::error::{AwError, Result};
use crate::index::Index;

/// 심볼 정의 1건.
#[derive(Debug, Clone)]
pub struct SymbolDef {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: i64,
    pub end_line: Option<i64>,
    pub signature: String,
}

impl SymbolDef {
    /// 시그니처 해시(닻 검증용, D14). 코드가 바뀌면 달라진다.
    pub fn sig_hash(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.name.as_bytes());
        h.update(b"\0");
        h.update(self.signature.as_bytes());
        let d = h.finalize();
        d.iter().take(4).map(|b| format!("{b:02x}")).collect()
    }
}

/// 코드 인덱서. 커널 등 소스 트리 루트를 가리킨다.
pub struct CodeIndex<'a> {
    src_root: PathBuf,
    index: &'a Index,
}

impl<'a> CodeIndex<'a> {
    pub fn new(src_root: impl AsRef<Path>, index: &'a Index) -> Self {
        Self {
            src_root: src_root.as_ref().to_path_buf(),
            index,
        }
    }

    /// `ctags` 사용 가능 여부.
    pub fn ctags_available() -> bool {
        Command::new("ctags")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// `cscope` 사용 가능 여부.
    pub fn cscope_available() -> bool {
        Command::new("cscope")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// 소스 트리를 ctags 로 스캔해 `symbols` 테이블을 재생성한다(D12 수동 reindex).
    /// 반환: 적재한 심볼 수.
    pub fn reindex_symbols(&self) -> Result<usize> {
        if !Self::ctags_available() {
            return Err(AwError::Invalid(
                "ctags not found — install universal-ctags".into(),
            ));
        }
        // Universal Ctags: JSON 출력 + 라인/끝라인/시그니처/kind 필드.
        let output = Command::new("ctags")
            .args([
                "-R",
                "--output-format=json",
                "--fields=+neKSt",
                "--languages=C,C++",
                "-f",
                "-", // stdout 으로
            ])
            .arg(&self.src_root)
            .current_dir(&self.src_root)
            .output()?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(AwError::Invalid(format!("ctags failed: {err}")));
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let defs = parse_ctags_json(&text);
        self.index.replace_symbols(&defs, &self.src_root.to_string_lossy())?;
        Ok(defs.len())
    }

    /// 심볼 정의 위치 조회 (grep 대체). 여러 정의가 있으면 모두 반환.
    pub fn where_sym(&self, name: &str, limit: usize) -> Result<Vec<SymbolDef>> {
        self.index.lookup_symbols(name, limit)
    }

    /// 함수/구조체 **본문만 슬라이싱**해 반환(whole-file read 금지).
    /// end_line 이 있으면 [line, end_line], 없으면 휴리스틱으로 추정.
    pub fn show_sym(&self, name: &str) -> Result<Option<(SymbolDef, String)>> {
        let defs = self.index.lookup_symbols(name, 1)?;
        let Some(def) = defs.into_iter().next() else {
            return Ok(None);
        };
        let path = self.src_root.join(&def.file);
        let content = std::fs::read_to_string(&path)
            .map_err(|e| AwError::Invalid(format!("cannot read {}: {e}", def.file)))?;
        let lines: Vec<&str> = content.lines().collect();
        let start = (def.line as usize).saturating_sub(1);
        let end = match def.end_line {
            Some(e) if e as usize >= def.line as usize => e as usize,
            _ => brace_match_end(&lines, start),
        };
        let end = end.min(lines.len());
        let slice = lines[start..end].join("\n");
        Ok(Some((def, slice)))
    }

    /// cscope 크로스레퍼런스. mode: callers(누가 호출) | callees(무엇을 호출).
    pub fn xref(&self, name: &str, callers: bool, limit: usize) -> Result<Vec<String>> {
        if !Self::cscope_available() {
            return Err(AwError::Invalid("cscope not found".into()));
        }
        // -L: line-oriented, -3: functions calling this, -2: functions called by this.
        let num = if callers { "3" } else { "2" };
        let output = Command::new("cscope")
            .args(["-d", "-R", "-L", &format!("-{num}"), name])
            .current_dir(&self.src_root)
            .output();
        let output = match output {
            Ok(o) => o,
            Err(e) => return Err(AwError::Invalid(format!("cscope failed: {e}"))),
        };
        let text = String::from_utf8_lossy(&output.stdout);
        let mut out = Vec::new();
        for line in text.lines().take(limit) {
            // 형식: file function lineno text...
            out.push(line.trim().to_string());
        }
        Ok(out)
    }

    /// cscope 데이터베이스를 빌드한다(xref 전 1회).
    pub fn build_cscope(&self) -> Result<()> {
        if !Self::cscope_available() {
            return Err(AwError::Invalid("cscope not found".into()));
        }
        let status = Command::new("cscope")
            .args(["-b", "-q", "-R"])
            .current_dir(&self.src_root)
            .status()?;
        if !status.success() {
            return Err(AwError::Invalid("cscope -b failed".into()));
        }
        Ok(())
    }

    /// 심볼 닻 검증(D14): 현재 코드의 sig_hash 와 노트에 기록된 값을 비교.
    /// 반환: (심볼 존재 여부, 현재 sig_hash). 노트 sig_hash 와 다르면 stale.
    pub fn verify_anchor(&self, name: &str) -> Result<Option<String>> {
        let defs = self.index.lookup_symbols(name, 1)?;
        Ok(defs.into_iter().next().map(|d| d.sig_hash()))
    }
}

/// ctags JSON(라인구분) 출력을 파싱한다.
fn parse_ctags_json(text: &str) -> Vec<SymbolDef> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("_type").and_then(|t| t.as_str()) != Some("tag") {
            continue;
        }
        let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("");
        if name.is_empty() {
            continue;
        }
        out.push(SymbolDef {
            name: name.to_string(),
            kind: v.get("kind").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            file: v.get("path").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            line: v.get("line").and_then(|x| x.as_i64()).unwrap_or(0),
            end_line: v.get("end").and_then(|x| x.as_i64()),
            signature: v
                .get("signature")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
        });
    }
    out
}

/// end 필드가 없을 때 중괄호 매칭으로 블록 끝 라인을 추정한다.
/// 문자열("...")·문자('...') 리터럴과 주석(// , /* */) 속 중괄호는 무시한다.
fn brace_match_end(lines: &[&str], start: usize) -> usize {
    let mut depth = 0i32;
    let mut seen = false;
    let mut in_block_comment = false;
    for (i, line) in lines.iter().enumerate().skip(start) {
        let bytes: Vec<char> = line.chars().collect();
        let mut j = 0;
        while j < bytes.len() {
            let c = bytes[j];
            if in_block_comment {
                if c == '*' && bytes.get(j + 1) == Some(&'/') {
                    in_block_comment = false;
                    j += 2;
                    continue;
                }
                j += 1;
                continue;
            }
            match c {
                '/' if bytes.get(j + 1) == Some(&'/') => break, // 라인 주석: 줄 끝까지 무시
                '/' if bytes.get(j + 1) == Some(&'*') => {
                    in_block_comment = true;
                    j += 2;
                    continue;
                }
                '"' | '\'' => {
                    // 리터럴 스킵(이스케이프 처리).
                    let quote = c;
                    j += 1;
                    while j < bytes.len() {
                        if bytes[j] == '\\' {
                            j += 2;
                            continue;
                        }
                        if bytes[j] == quote {
                            break;
                        }
                        j += 1;
                    }
                }
                '{' => {
                    depth += 1;
                    seen = true;
                }
                '}' => depth -= 1,
                _ => {}
            }
            j += 1;
        }
        if seen && depth <= 0 {
            return i + 1;
        }
        // 시그니처만 있고 본문이 멀면 과도 확장 방지.
        if !seen && i - start > 6 {
            return i + 1;
        }
    }
    (start + 40).min(lines.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctags_json_parsing() {
        let sample = r#"{"_type":"tag","name":"do_mmap","path":"mm/mmap.c","line":100,"end":150,"kind":"function","signature":"(struct file *file, unsigned long addr)"}
{"_type":"ptag","name":"!_TAG_PROGRAM"}
garbage line"#;
        let defs = parse_ctags_json(sample);
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "do_mmap");
        assert_eq!(defs[0].line, 100);
        assert_eq!(defs[0].end_line, Some(150));
        assert!(defs[0].signature.contains("struct file"));
    }

    #[test]
    fn sig_hash_changes_with_signature() {
        let a = SymbolDef {
            name: "f".into(),
            kind: "function".into(),
            file: "a.c".into(),
            line: 1,
            end_line: None,
            signature: "(int x)".into(),
        };
        let mut b = a.clone();
        b.signature = "(int x, int y)".into();
        assert_ne!(a.sig_hash(), b.sig_hash());
    }

    #[test]
    fn brace_match_finds_block_end() {
        let src = vec!["int f(void)", "{", "  return 0;", "}", "int g;"];
        let end = brace_match_end(&src, 0);
        assert_eq!(end, 4); // '}' 라인까지
    }

    #[test]
    fn brace_match_handles_braces_in_strings_and_chars() {
        // 문자열/문자 리터럴 속 중괄호를 깊이 계산에서 제외해야 정확하다.
        let src = vec![
            "int f(void)",
            "{",
            "    char *s = \"}\";",   // 문자열 속 '}' — 무시되어야
            "    char c = '{';",        // 문자 리터럴 속 '{'
            "    return 0;",
            "}",
            "int g;",
        ];
        let end = brace_match_end(&src, 0);
        assert_eq!(end, 6, "함수의 진짜 닫는 중괄호(6번째 줄)를 찾아야 함");
    }

    // ctags 가 있는 환경에서만 도는 통합 테스트.
    #[test]
    fn ctags_integration_where_and_show() {
        if !CodeIndex::ctags_available() {
            eprintln!("skipping: ctags not available");
            return;
        }
        let dir = std::env::temp_dir().join(format!("awcode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = "int helper(int x)\n{\n\treturn x+1;\n}\n\nint do_mmap(struct file *f, unsigned long addr)\n{\n\treturn helper(addr);\n}\n";
        std::fs::write(dir.join("t.c"), src).unwrap();

        let idx = Index::open_in_memory().unwrap();
        let code = CodeIndex::new(&dir, &idx);
        let n = code.reindex_symbols().unwrap();
        assert!(n >= 2, "expected at least 2 symbols, got {n}");

        let defs = code.where_sym("do_mmap", 5).unwrap();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].kind, "function");

        let (_d, body) = code.show_sym("do_mmap").unwrap().unwrap();
        assert!(body.contains("helper(addr)"));
        assert!(!body.contains("int helper(int x)"), "show must slice only the target fn");

        std::fs::remove_dir_all(&dir).ok();
    }
}
