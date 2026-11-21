// Direct Rust implementation of the R02 JSON workloads.
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{self, BufRead, Write};

type Answer = Result<(Value, Vec<String>), String>;

fn undefined() -> Value {
    json!({"undef": 1})
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => false,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        _ => value.get("undef").is_none() && value.get("absent").is_none(),
    }
}

fn key_part(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        v if v.get("undef").is_some() || v.get("absent").is_some() => "undefined".into(),
        v => v.to_string(),
    }
}

fn bound_index(args: &Value, script: &Value) -> Answer {
    let sessions = args["sessions"].as_array().ok_or("sessions must be an array")?;
    let mut trace = Vec::new();
    let mut rows = Vec::new();
    let mut keys = HashSet::new();
    let temporary = &script["isTemporary"];
    let undef = undefined();
    for session in sessions {
        if session["temporary"] == true {
            continue;
        }
        let id = session.get("id").unwrap_or(&undef);
        let reply = match temporary.as_array() {
            Some(queue) => queue.get(trace.len()).or_else(|| queue.last()),
            None => Some(temporary),
        };
        trace.push(format!("[\"isTemporary\",{}]", id));
        if reply.and_then(|r| r.get("ok")).is_some_and(truthy) {
            continue;
        }

        let mut add = |engine: &str, thread: &Value| {
            let key = format!("{}:{}", engine, key_part(thread));
            if keys.insert(key.clone()) {
                rows.push(json!([key, id]));
            }
        };
        let external = &session["externalThread"];
        if external.is_object() {
            let engine = key_part(external.get("engine").unwrap_or(&undef));
            add(&engine, external.get("thread").unwrap_or(&undef));
            if let Some(forks) = external["forks"]["arr"].as_array() {
                for fork in forks {
                    add(&engine, fork);
                }
            }
        }
        if let Some(thread) = session["terminal"].get("cliSession").filter(|t| truthy(t)) {
            let engine = &session["engine"];
            if engine.is_null() || engine.get("undef").is_some() || engine.get("absent").is_some() {
                add("codex", thread);
            } else {
                add(&key_part(engine), thread);
            }
        }
        if let Some(entries) = session["engineThreads"]["obj"].as_array() {
            for entry in entries {
                let key = entry[0].as_str().ok_or("engineThreads key must be a string")?;
                if let Some(thread) = entry[1].get("threadId").filter(|t| truthy(t)) {
                    add(key.split(':').next().unwrap(), thread);
                }
            }
        }
    }
    Ok((Value::Array(rows), trace))
}

// The scripted host supplies the completion for an external engine read.
fn read_external(args: &Value, reply: &Value) -> Answer {
    let source = args["source"]["ref"].as_str().ok_or("source needs a ref")?;
    let options = args["options"]["ref"].as_str().ok_or("options needs a ref")?;
    let engine = match args["source"]["engine"].as_str() {
        Some(e @ ("claude" | "codex" | "opencode" | "qwen")) => e,
        _ => "grok",
    };
    let record = reply["record"].as_str().ok_or("expected a scripted engine record")?;
    let undef = undefined();
    let folder = args.get("folder").unwrap_or(&undef);
    let thread = args.get("thread")
        .filter(|t| t.get("absent").is_none())
        .map(|t| format!(",\"thread\":{}", t))
        .unwrap_or_default();
    let trace = format!(
        "[\"engine\",\"{}\",\"read\",{{\"source\":{},\"folder\":{}{},\"options\":{}}}]",
        engine, json!({"ref": source}), folder, thread, json!({"ref": options})
    );
    Ok((json!({"ref": record}), vec![trace]))
}

fn run_case(request: &Value) -> Answer {
    if !request.is_object() {
        return Err("line is not a JSON object".into());
    }
    let args = &request["args"];
    let script = &request["script"];
    match request["op"].as_str() {
        Some("bound_session_index") => bound_index(args, script),
        Some("read_external") => {
            let reply = request.get("forward").filter(|v| !v.is_null()).unwrap_or(&script["engine"]);
            read_external(args, reply)
        }
        Some(op) => Err(format!("unknown op {}", op)),
        None => Err("missing op".into()),
    }
}

fn main() -> io::Result<()> {
    let mut out = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&line) {
            Ok(request) => {
                let id = &request["id"];
                match run_case(&request) {
                    Ok((value, trace)) => writeln!(
                        out, "{{\"id\":{},\"terminal\":\"ok\",\"value\":{},\"trace\":[{}],\"driver\":null}}",
                        id, value, trace.join(",")
                    )?,
                    Err(error) => writeln!(out, "{{\"id\":{},\"error\":{}}}", id, json!(format!("case: {}", error)))?,
                }
            }
            Err(error) => writeln!(out, "{{\"id\":null,\"error\":{}}}", json!(format!("case: bad JSON: {}", error)))?,
        }
        out.flush()?;
    }
    Ok(())
}
