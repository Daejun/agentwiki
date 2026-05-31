//! MCP 도구 핸들러 (M1: wiki / kv / admin 그룹, D17).
//!
//! 각 그룹은 하나의 MCP 도구이며 `op` 인자로 동작을 분기한다. 모든 출력은
//! 보수적 기본 상한 + 확장 인자(D23)를 따른다(예: wiki.search 기본 8건).

use aw_core::{NoteType, Store};
use serde_json::{json, Value};

const DEFAULT_SEARCH_LIMIT: usize = 8;
const DEFAULT_DIGEST_NOTES: usize = 20;

pub struct Server {
    root: String,
    initialized: bool,
}

impl Server {
    pub fn new() -> anyhow::Result<Server> {
        let root = std::env::var("AW_ROOT").unwrap_or_else(|_| ".".to_string());
        std::fs::create_dir_all(format!("{root}/.aw")).ok();
        Ok(Server {
            root,
            initialized: false,
        })
    }

    fn open(&self) -> aw_core::Result<Store> {
        Store::open(&self.root, &format!("{}/.aw/index.db", self.root))
    }

    pub fn handle(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        match method {
            "initialize" => {
                self.initialized = true;
                Ok(json!({
                    "protocolVersion": "2024-11-05",
                    "serverInfo": {"name": "agentwiki", "version": env!("CARGO_PKG_VERSION")},
                    "capabilities": {"tools": {}}
                }))
            }
            "notifications/initialized" => Ok(Value::Null),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tool_specs()})),
            "tools/call" => self.call_tool(params),
            other => Err(anyhow::anyhow!("unknown method: {other}")),
        }
    }

    fn call_tool(&self, params: Value) -> anyhow::Result<Value> {
        let name = params
            .get("name")
            .and_then(|n| n.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing tool name"))?;
        let args = params.get("arguments").cloned().unwrap_or(json!({}));

        let text = match name {
            "wiki" => self.tool_wiki(&args)?,
            "code" => self.tool_code(&args)?,
            "recall" => self.tool_recall(&args)?,
            "kv" => self.tool_kv(&args)?,
            "admin" => self.tool_admin(&args)?,
            other => return Err(anyhow::anyhow!("unknown tool: {other}")),
        };
        Ok(json!({"content": [{"type": "text", "text": text}]}))
    }

    fn tool_wiki(&self, a: &Value) -> anyhow::Result<String> {
        let op = str_arg(a, "op").unwrap_or_default();
        let store = self.open()?;
        match op.as_str() {
            "search" => {
                let q = str_arg(a, "query").unwrap_or_default();
                let sub = str_arg(a, "subsystem");
                let limit = usize_arg(a, "limit").unwrap_or(DEFAULT_SEARCH_LIMIT);
                store.reindex()?;
                let hits = store.search(&q, sub.as_deref(), limit)?;
                if hits.is_empty() {
                    return Ok(format!("no results for '{q}'"));
                }
                let mut s = String::new();
                for h in hits {
                    s.push_str(&format!("- {} [{}] {}\n", h.id, h.subsystem, h.title));
                    if !h.snippet.is_empty() {
                        s.push_str(&format!("    … {}\n", h.snippet));
                    }
                }
                Ok(s)
            }
            "get" => {
                let id = str_arg(a, "id").ok_or_else(|| anyhow::anyhow!("op=get needs 'id'"))?;
                let note = store.get(&id)?;
                Ok(note.to_markdown()?)
            }
            "propose" => {
                let sub = str_arg(a, "subsystem").unwrap_or_else(|| "misc".into());
                let title = str_arg(a, "title").ok_or_else(|| anyhow::anyhow!("needs 'title'"))?;
                let body = str_arg(a, "body").ok_or_else(|| anyhow::anyhow!("needs 'body'"))?;
                let ty = parse_type(&str_arg(a, "type").unwrap_or_else(|| "concept".into()));
                let conf = str_arg(a, "confidence").unwrap_or_else(|| "med".into());
                let tags = vec_arg(a, "tags");
                let id_override = str_arg(a, "id");
                store.reindex()?;
                let p = store.propose(&sub, &title, &body, ty, tags, &conf, id_override.as_deref())?;
                // 반자동(D6): 커밋하지 않고 초안+경고를 돌려준다. 승인 시 op=commit 호출.
                let mut s = String::new();
                s.push_str(&format!(
                    "PROPOSAL id={} ({})\n",
                    p.id,
                    if p.is_append { "append to existing" } else { "new note" }
                ));
                if !p.secret_findings.is_empty() {
                    s.push_str("⚠ secret-scan warnings (not blocking, D15):\n");
                    for f in &p.secret_findings {
                        s.push_str(&format!("    line {} {}: {}\n", f.line, f.kind, f.preview));
                    }
                }
                s.push_str("--- preview ---\n");
                s.push_str(&p.preview);
                s.push_str(&format!(
                    "\n--- to save: call wiki op=commit with id={} ---\n",
                    p.id
                ));
                Ok(s)
            }
            "commit" => {
                // 승인된 초안을 다시 만들어 저장한다(propose 와 동일 인자 + id).
                let sub = str_arg(a, "subsystem").unwrap_or_else(|| "misc".into());
                let title = str_arg(a, "title").ok_or_else(|| anyhow::anyhow!("needs 'title'"))?;
                let body = str_arg(a, "body").ok_or_else(|| anyhow::anyhow!("needs 'body'"))?;
                let ty = parse_type(&str_arg(a, "type").unwrap_or_else(|| "concept".into()));
                let conf = str_arg(a, "confidence").unwrap_or_else(|| "med".into());
                let tags = vec_arg(a, "tags");
                let id_override = str_arg(a, "id");
                store.reindex()?;
                let p = store.propose(&sub, &title, &body, ty, tags, &conf, id_override.as_deref())?;
                store.commit(&p)?;
                Ok(format!(
                    "committed {} ({})",
                    p.id,
                    if p.is_append { "appended" } else { "created" }
                ))
            }
            other => Err(anyhow::anyhow!("wiki: unknown op '{other}'")),
        }
    }

    /// 코드 인덱스 도구 (M2, D7). op=reindex|where|show|xref.
    /// 소스 루트는 인자 `src` 또는 KV `kernel_src`(machine 스코프)에서 얻는다.
    fn tool_code(&self, a: &Value) -> anyhow::Result<String> {
        let op = str_arg(a, "op").unwrap_or_default();
        let store = self.open()?;
        let src = self.resolve_src(&store, a)?;
        let code = store.code(&src);
        match op.as_str() {
            "reindex" => {
                let n = code.reindex_symbols()?;
                Ok(format!("indexed {n} symbols from {src}"))
            }
            "where" => {
                let sym = str_arg(a, "symbol").ok_or_else(|| anyhow::anyhow!("needs 'symbol'"))?;
                let defs = code.where_sym(&sym, usize_arg(a, "limit").unwrap_or(10))?;
                if defs.is_empty() {
                    return Ok(format!("no definition found for '{sym}' (run code op=reindex first?)"));
                }
                let mut s = String::new();
                for d in defs {
                    s.push_str(&format!(
                        "{}:{} [{}] {}{}\n",
                        d.file, d.line, d.kind, d.name, d.signature
                    ));
                }
                Ok(s)
            }
            "show" => {
                let sym = str_arg(a, "symbol").ok_or_else(|| anyhow::anyhow!("needs 'symbol'"))?;
                match code.show_sym(&sym)? {
                    None => Ok(format!("no definition for '{sym}'")),
                    Some((d, body)) => Ok(format!(
                        "// {}:{}-{}  {}{}\n{}",
                        d.file,
                        d.line,
                        d.end_line.unwrap_or(d.line),
                        d.name,
                        d.signature,
                        body
                    )),
                }
            }
            "xref" => {
                let sym = str_arg(a, "symbol").ok_or_else(|| anyhow::anyhow!("needs 'symbol'"))?;
                let callers = str_arg(a, "mode").as_deref() != Some("callees");
                let lines = code.xref(&sym, callers, usize_arg(a, "limit").unwrap_or(20))?;
                if lines.is_empty() {
                    return Ok(format!("no {} for '{sym}' (build cscope first?)",
                        if callers { "callers" } else { "callees" }));
                }
                Ok(lines.join("\n"))
            }
            other => Err(anyhow::anyhow!("code: unknown op '{other}'")),
        }
    }

    /// 통합 회수 도구 (M2 핵심): 심볼 정의 + 닻 노트 + stale 경고를 묶어 반환.
    fn tool_recall(&self, a: &Value) -> anyhow::Result<String> {
        let sym = str_arg(a, "symbol").ok_or_else(|| anyhow::anyhow!("recall needs 'symbol'"))?;
        let store = self.open()?;
        store.reindex()?;
        let r = store.recall(&sym, usize_arg(a, "limit").unwrap_or(5))?;
        let mut s = format!("# recall: {}\n", r.symbol);
        match &r.def {
            Some(d) => s.push_str(&format!(
                "definition: {}:{} [{}] {}{}\n",
                d.file, d.line, d.kind, d.name, d.signature
            )),
            None => s.push_str("definition: (not in code index — run code op=reindex)\n"),
        }
        if !r.anchored_notes.is_empty() {
            s.push_str("anchored notes:\n");
            for (id, title) in &r.anchored_notes {
                let flag = if r.stale_notes.contains(id) { " ⚠STALE" } else { "" };
                s.push_str(&format!("  - {id} {title}{flag}\n"));
            }
        }
        if !r.mentions.is_empty() {
            s.push_str("mentions:\n");
            for h in &r.mentions {
                s.push_str(&format!("  - {} [{}] {}\n", h.id, h.subsystem, h.title));
            }
        }
        if r.anchored_notes.is_empty() && r.mentions.is_empty() {
            s.push_str("(no notes yet — capture one with wiki op=propose)\n");
        }
        Ok(s)
    }

    /// 소스 루트 해석: 인자 src > KV kernel_src > 에러.
    fn resolve_src(&self, store: &Store, a: &Value) -> anyhow::Result<String> {
        if let Some(s) = str_arg(a, "src") {
            return Ok(s);
        }
        let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into());
        let prefer = vec![format!("machine:{host}")];
        match store.index().kv_get("kernel_src", &prefer)? {
            Some(s) => Ok(s),
            None => Err(anyhow::anyhow!(
                "no source root: pass 'src' or set kv key 'kernel_src' (scope machine:<host>)"
            )),
        }
    }

    fn tool_kv(&self, a: &Value) -> anyhow::Result<String> {
        let op = str_arg(a, "op").unwrap_or_default();
        let store = self.open()?;
        let key = str_arg(a, "key").ok_or_else(|| anyhow::anyhow!("kv needs 'key'"))?;
        match op.as_str() {
            "set" => {
                let value = str_arg(a, "value").ok_or_else(|| anyhow::anyhow!("needs 'value'"))?;
                let scope = str_arg(a, "scope").unwrap_or_else(|| "global".into());
                store.index().kv_set(&key, &value, &scope)?;
                Ok(format!("set {key} @ {scope}"))
            }
            "get" => {
                let host = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into());
                let mut prefer = vec![format!("machine:{host}")];
                if let Some(proj) = str_arg(a, "project") {
                    prefer.insert(0, format!("project:{proj}"));
                }
                match store.index().kv_get(&key, &prefer)? {
                    Some(v) => Ok(v),
                    None => Ok(format!("(not set: {key})")),
                }
            }
            other => Err(anyhow::anyhow!("kv: unknown op '{other}'")),
        }
    }

    fn tool_admin(&self, a: &Value) -> anyhow::Result<String> {
        let op = str_arg(a, "op").unwrap_or_default();
        let store = self.open()?;
        match op.as_str() {
            "reindex" => {
                let n = store.reindex()?;
                Ok(format!("reindexed {n} notes"))
            }
            "digest" => {
                let max = usize_arg(a, "limit").unwrap_or(DEFAULT_DIGEST_NOTES);
                store.reindex()?;
                let d = store.digest(max)?;
                let mut s = format!("# agentwiki digest — {} notes total\n", d.total);
                for n in d.notes {
                    s.push_str(&format!(
                        "- {} [{}] {} ({})\n",
                        n.id, n.subsystem, n.title, n.confidence
                    ));
                }
                s.push_str("\n(본문은 wiki op=get 으로 회수)\n");
                Ok(s)
            }
            "stale" => {
                store.reindex()?;
                let reports = store.check_stale()?;
                if reports.is_empty() {
                    return Ok("no stale anchors".into());
                }
                let mut s = String::from("stale symbol anchors (D14):\n");
                for r in reports {
                    s.push_str(&format!("  - {} → {} ({})\n", r.note_id, r.symbol, r.status));
                }
                Ok(s)
            }
            "compact" => {
                let removed = store.compact()?;
                Ok(format!("compacted: removed {removed} duplicate/empty entries"))
            }
            "scan-secrets" => {
                let text = str_arg(a, "text").unwrap_or_default();
                let findings = aw_core::secrets::scan(&text);
                if findings.is_empty() {
                    Ok("no secrets found".into())
                } else {
                    let mut s = String::from("⚠ secret-scan warnings (not blocking, D15):\n");
                    for f in findings {
                        s.push_str(&format!("    line {} {}: {}\n", f.line, f.kind, f.preview));
                    }
                    Ok(s)
                }
            }
            other => Err(anyhow::anyhow!("admin: unknown op '{other}'")),
        }
    }
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

fn str_arg(a: &Value, k: &str) -> Option<String> {
    a.get(k).and_then(|v| v.as_str()).map(|s| s.to_string())
}

fn usize_arg(a: &Value, k: &str) -> Option<usize> {
    a.get(k).and_then(|v| v.as_u64()).map(|n| n as usize)
}

fn vec_arg(a: &Value, k: &str) -> Vec<String> {
    a.get(k)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// tools/list 응답용 도구 스펙(중간 입자 3그룹, D17).
fn tool_specs() -> Value {
    json!([
        {
            "name": "wiki",
            "description": "지식 위키 검색/조회/반자동 기록. op=search|get|propose|commit. propose는 초안을 보여주고 저장하지 않으며, 승인 후 commit으로 append-only 기록한다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "op": {"type": "string", "enum": ["search", "get", "propose", "commit"]},
                    "query": {"type": "string", "description": "search용 질의"},
                    "subsystem": {"type": "string", "description": "검색 범위 한정(mm/net/...)"},
                    "limit": {"type": "integer", "description": "결과 수(기본 8)"},
                    "id": {"type": "string", "description": "get 대상 또는 append 대상 노트 id"},
                    "title": {"type": "string"},
                    "body": {"type": "string"},
                    "type": {"type": "string", "enum": ["concept","gotcha","decision","howto","symbol-note"]},
                    "tags": {"type": "array", "items": {"type": "string"}},
                    "confidence": {"type": "string", "enum": ["high","med","low"]}
                },
                "required": ["op"]
            }
        },
        {
            "name": "code",
            "description": "코드 인덱스(ctags/cscope). op=reindex|where|show|xref. show는 함수 본문만 잘라 반환(거대 파일 통째 read 금지). 소스 루트는 src 인자 또는 kv 'kernel_src'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "op": {"type": "string", "enum": ["reindex", "where", "show", "xref"]},
                    "symbol": {"type": "string"},
                    "src": {"type": "string", "description": "소스 트리 루트(미지정 시 kv kernel_src)"},
                    "mode": {"type": "string", "enum": ["callers", "callees"], "description": "xref 방향"},
                    "limit": {"type": "integer"}
                },
                "required": ["op"]
            }
        },
        {
            "name": "recall",
            "description": "심볼 1개로 통합 회수: 코드 정의 + 그 심볼에 닻 내린 노트 + 언급 검색 + stale 경고를 한 번에. '이 함수 만질 건데 아는 거 다 줘'.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "symbol": {"type": "string"},
                    "limit": {"type": "integer"}
                },
                "required": ["symbol"]
            }
        },
        {
            "name": "kv",
            "description": "빠른 사실 키-값. op=get|set. scope=global|project:<id>|machine:<host>. 조회는 machine→project→global 우선순위.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "op": {"type": "string", "enum": ["get", "set"]},
                    "key": {"type": "string"},
                    "value": {"type": "string"},
                    "scope": {"type": "string"},
                    "project": {"type": "string"}
                },
                "required": ["op", "key"]
            }
        },
        {
            "name": "admin",
            "description": "운영. op=reindex|digest|scan-secrets. digest는 세션 시작용 최소 목차(본문 없음).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "op": {"type": "string", "enum": ["reindex", "digest", "stale", "compact", "scan-secrets"]},
                    "limit": {"type": "integer"},
                    "text": {"type": "string"}
                },
                "required": ["op"]
            }
        }
    ])
}
