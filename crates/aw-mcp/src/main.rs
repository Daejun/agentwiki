//! aw-mcp — agentwiki MCP 서버 (D3: 처음부터 MCP).
//!
//! 의존성을 최소화하기 위해 외부 MCP SDK 없이 JSON-RPC 2.0 over stdio 로
//! Model Context Protocol의 핵심 메서드(initialize / tools/list / tools/call)만
//! 구현한다. 도구 표면은 중간 입자(D17): `wiki` / `kv` / `admin` 그룹을 각각
//! 하나의 도구로 노출하고 동작은 `op` 인자로 분기한다.
//!
//! 노트 루트는 환경변수 `AW_ROOT`(기본 ".")에서 읽는다.

mod tools;

use std::io::{BufRead, Write};

use serde_json::{json, Value};

fn main() -> anyhow::Result<()> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let mut server = tools::Server::new()?;

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                write_msg(&mut out, &rpc_error(Value::Null, -32700, &format!("parse error: {e}")))?;
                continue;
            }
        };

        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(json!({}));

        // 알림(notification, id 없음)은 응답하지 않는다.
        let is_notification = req.get("id").is_none();

        let result = server.handle(method, params);
        if is_notification {
            continue;
        }
        match result {
            Ok(value) => write_msg(&mut out, &rpc_ok(id, value))?,
            Err(e) => write_msg(&mut out, &rpc_error(id, -32603, &e.to_string()))?,
        }
    }
    Ok(())
}

fn rpc_ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn write_msg(out: &mut impl Write, msg: &Value) -> anyhow::Result<()> {
    let s = serde_json::to_string(msg)?;
    out.write_all(s.as_bytes())?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}
