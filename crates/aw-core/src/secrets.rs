//! 비밀 스캔 — 경고만 (D15).
//!
//! 노트가 git으로 멀티머신 공유되므로 커밋 전 비밀/키/토큰 패턴을 탐지한다.
//! 정책상 **차단하지 않고 경고만** 반환한다(D15). 호출자가 경고를 사용자에게
//! 보여주고, 필요하면 notes-local/(비동기화)로 옮기도록 유도한다.

use regex::Regex;
use std::sync::OnceLock;

/// 탐지된 비밀 후보 1건.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub kind: String,
    pub line: usize,
    /// 매치된 부분을 마스킹한 미리보기.
    pub preview: String,
}

struct Rule {
    kind: &'static str,
    re: Regex,
}

fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        let r = |kind: &'static str, pat: &str| Rule {
            kind,
            re: Regex::new(pat).unwrap(),
        };
        vec![
            r("aws-access-key", r"\bAKIA[0-9A-Z]{16}\b"),
            r("github-token", r"\bgh[pousr]_[A-Za-z0-9]{20,}\b"),
            r("slack-token", r"\bxox[baprs]-[A-Za-z0-9-]{10,}\b"),
            r("openai-key", r"\bsk-[A-Za-z0-9]{20,}\b"),
            r("private-key-block", r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"),
            r("generic-bearer", r"(?i)\bbearer\s+[A-Za-z0-9._-]{20,}\b"),
            r(
                "assigned-secret",
                r#"(?i)\b(?:password|passwd|secret|api[_-]?key|token)\b\s*[:=]\s*["']?[^\s"']{8,}"#,
            ),
        ]
    })
}

/// 마스킹된 미리보기를 만든다(앞 4글자만 노출).
fn mask(m: &str) -> String {
    let chars: Vec<char> = m.chars().collect();
    if chars.len() <= 4 {
        return "*".repeat(chars.len());
    }
    let head: String = chars[..4].iter().collect();
    format!("{head}{}", "*".repeat(chars.len() - 4))
}

/// 텍스트를 스캔해 비밀 후보 목록을 반환한다(차단하지 않음, D15).
pub fn scan(text: &str) -> Vec<Finding> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        for rule in rules() {
            if let Some(m) = rule.re.find(line) {
                out.push(Finding {
                    kind: rule.kind.to_string(),
                    line: i + 1,
                    preview: mask(m.as_str()),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_aws_and_masks() {
        let f = scan("key = AKIAIOSFODNN7EXAMPLE here");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].kind, "aws-access-key");
        assert!(f[0].preview.starts_with("AKIA"));
        assert!(f[0].preview.contains('*'));
    }

    #[test]
    fn detects_assigned_secret() {
        let f = scan("password: hunter2hunter2");
        assert!(f.iter().any(|x| x.kind == "assigned-secret"));
    }

    #[test]
    fn clean_text_no_findings() {
        let f = scan("do_mmap takes mmap_write_lock; nothing secret here.");
        assert!(f.is_empty());
    }
}
