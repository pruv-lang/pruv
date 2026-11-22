#[path="@SOL@"] mod sol;
use std::io::{BufRead,Write};
// One JSON request per stdin line: {"mode":"list"|"read","binary","args":[...],"cwd","folder","thread","timeout"}.
// One JSON result per stdout line, rendered by codex_host (list items, ExternalRecord, or {"error":"<code>"}).
fn main(){
 for line in std::io::stdin().lock().lines(){
  let v:serde_json::Value=serde_json::from_str(&line.unwrap()).unwrap();
  let mode=v["mode"].as_str().unwrap();
  let binary=v["binary"].as_str().unwrap();
  let cwd=v["cwd"].as_str().unwrap_or("");
  let folder=v["folder"].as_str().unwrap_or("");
  let thread=v["thread"].as_str().unwrap_or("");
  let ms=v["timeout"].as_i64().unwrap_or(20000);
  let args=v["args"].as_array().map(|a|a.iter().map(|x|format!("{}\0",x.as_str().unwrap())).collect::<String>()).unwrap_or_default();
  // read streams the wire into stdout (DB44): no rendered String is held next to the wire
  let mut out=std::io::BufWriter::new(std::io::stdout().lock());
  if mode == "list" {
   writeln!(out, "{}", codex_host::render(&sol::codex_list(binary, &args, cwd, folder, ms))).unwrap();
  } else {
   codex_host::render_wire_to(&sol::codex_read_wire(binary, &args, cwd, folder, thread, ms), &mut out).unwrap();
   out.write_all(b"\n").unwrap();
  }
  out.flush().unwrap();
 }
}
