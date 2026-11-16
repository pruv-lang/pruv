#[path="@SOL@"] mod sol;
#[path="@FINGERPRINT@"] mod fingerprint;
use std::io::{BufRead,Write};
fn main(){for line in std::io::stdin().lock().lines(){let v:serde_json::Value=serde_json::from_str(&line.unwrap()).unwrap();let cmd=v["command"].as_str().unwrap();let cwd=v["cwd"].as_str().unwrap_or("");let ms=v["timeout"].as_i64().unwrap_or(15000);let args=v["args"].as_array().unwrap().iter().map(|a|format!("{}\0",a.as_str().unwrap())).collect::<String>();let p=sol::mcp_check(cmd,&args,cwd,ms);println!("{}",mcp_host::render(&p,cmd,ms,fingerprint::fingerprints));std::io::stdout().flush().unwrap();}}
