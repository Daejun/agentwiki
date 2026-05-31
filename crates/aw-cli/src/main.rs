//! aw — 디버깅/스크립트용 얇은 CLI (D20: aw-core 위 얇은 래퍼).
//!
//! 정식 인터페이스는 MCP 서버(aw-mcp)다. 이 CLI는 개발/검증용.
//!
//! 사용법:
//!   aw [--root DIR] reindex
//!   aw [--root DIR] search <query> [subsystem]
//!   aw [--root DIR] get <id>
//!   aw [--root DIR] note <subsystem> <title> <body> [type] [confidence]
//!   aw [--root DIR] kv-set <key> <value> <scope>
//!   aw [--root DIR] kv-get <key>
//!   aw [--root DIR] digest

use aw_core::{NoteType, Store};

fn index_path(root: &str) -> String {
    format!("{root}/.aw/index.db")
}

fn open(root: &str) -> anyhow::Result<Store> {
    std::fs::create_dir_all(format!("{root}/.aw"))?;
    Ok(Store::open(root, &index_path(root))?)
}

fn main() -> anyhow::Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();

    // --root DIR 옵션(기본: 현재 디렉터리).
    let mut root = ".".to_string();
    if let Some(i) = args.iter().position(|a| a == "--root") {
        root = args.get(i + 1).cloned().unwrap_or_else(|| ".".into());
        args.drain(i..=i + 1);
    }

    let cmd = args.first().cloned().unwrap_or_default();
    let rest = &args[args.len().min(1)..];

    match cmd.as_str() {
        "reindex" => {
            let store = open(&root)?;
            let (n, skipped) = store.reindex_verbose()?;
            println!("reindexed {n} notes");
            for s in skipped {
                eprintln!("  skipped (not a note): {s}");
            }
        }
        "search" => {
            let q = rest.first().map(String::as_str).unwrap_or("");
            let sub = rest.get(1).map(String::as_str);
            let store = open(&root)?;
            store.reindex()?;
            for h in store.search(q, sub, 8)? {
                println!("{}  [{}]  {}", h.id, h.subsystem, h.title);
                if !h.snippet.is_empty() {
                    println!("    … {}", h.snippet);
                }
            }
        }
        "get" => {
            let id = rest.first().map(String::as_str).unwrap_or("");
            let store = open(&root)?;
            let note = store.get(id)?;
            print!("{}", note.to_markdown()?);
        }
        "note" => {
            let sub = rest.first().cloned().unwrap_or_default();
            let title = rest.get(1).cloned().unwrap_or_default();
            let body = rest.get(2).cloned().unwrap_or_default();
            let ty = rest.get(3).map(String::as_str).unwrap_or("concept");
            let conf = rest.get(4).map(String::as_str).unwrap_or("med");
            // 6번째 인자로 기존 노트 id를 주면 append-only 추가(D13).
            let id_override = rest.get(5).map(String::as_str);
            let store = open(&root)?;
            store.reindex()?;
            let p = store.propose(&sub, &title, &body, parse_type(ty), vec![], conf, id_override)?;
            if !p.secret_findings.is_empty() {
                eprintln!("⚠ secret scan warnings (not blocking, D15):");
                for f in &p.secret_findings {
                    eprintln!("  line {} {}: {}", f.line, f.kind, f.preview);
                }
            }
            store.commit(&p)?;
            println!("{} {}", if p.is_append { "appended" } else { "created" }, p.id);
        }
        "kv-set" => {
            let key = rest.first().cloned().unwrap_or_default();
            let val = rest.get(1).cloned().unwrap_or_default();
            let scope = rest.get(2).map(String::as_str).unwrap_or("global");
            let store = open(&root)?;
            store.index().kv_set(&key, &val, scope)?;
            println!("set {key} @ {scope}");
        }
        "kv-get" => {
            let key = rest.first().cloned().unwrap_or_default();
            let store = open(&root)?;
            let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into());
            let prefer = vec![format!("machine:{host}")];
            match store.index().kv_get(&key, &prefer)? {
                Some(v) => println!("{v}"),
                None => println!("(not set)"),
            }
        }
        "digest" => {
            let store = open(&root)?;
            store.reindex()?;
            let d = store.digest(20)?;
            println!("# agentwiki digest — {} notes total", d.total);
            for n in d.notes {
                println!("- {} [{}] {} ({})", n.id, n.subsystem, n.title, n.confidence);
            }
        }
        _ => {
            eprintln!("usage: aw [--root DIR] <reindex|search|get|note|kv-set|kv-get|digest> ...");
            std::process::exit(2);
        }
    }
    Ok(())
}

fn parse_type(s: &str) -> NoteType {
    match s {
        "gotcha" => NoteType::Gotcha,
        "decision" => NoteType::Decision,
        "howto" => NoteType::Howto,
        "symbol-note" => NoteType::SymbolNote,
        "concept" => NoteType::Concept,
        other => NoteType::Other(other.to_string()),
    }
}
