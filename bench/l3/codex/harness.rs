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
  let p=if mode=="list"{sol::codex_list(binary,&args,cwd,folder,ms)}else{sol::codex_read(binary,&args,cwd,folder,thread,ms)};
  println!("{}",codex_host::render(&p));
  std::io::stdout().flush().unwrap();
 }
}
