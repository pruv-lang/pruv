use serde_json::{Value,json};
fn js_space(c:char)->bool{matches!(c,'\u{9}'..='\u{d}'|'\u{20}'|'\u{a0}'|'\u{1680}'|'\u{2000}'..='\u{200a}'|'\u{2028}'|'\u{2029}'|'\u{202f}'|'\u{205f}'|'\u{3000}'|'\u{feff}')}
fn one(s:&str,cap:usize)->String{let s=s.split(js_space).filter(|x|!x.is_empty()).collect::<Vec<_>>().join(" ");let mut n=0;s.chars().take_while(|c|{n+=c.len_utf16();n<=cap}).collect()}
fn slice(p:&[i64],at:usize)->&[i64]{let s=p[at];let n=p[at+1];if s<0{return &[]}assert!(s>=5010&&n>=0&&(s as usize+n as usize)<=p.len());&p[s as usize..(s+n) as usize]}
fn text(p:&[i64],at:usize)->String{String::from_utf16_lossy(&slice(p,at).iter().map(|&x|x as u16).collect::<Vec<_>>())}
pub fn render(p:&[i64],command:&str,ms:i64,fingerprints:fn(Vec<String>)->Vec<String>)->Value{assert!(p.len()>=5010);let stage=["start","connect","list"][p[1] as usize];let mut tools=vec![];
 if p[0]==1{assert!((0..=500).contains(&p[8]));let raws=(0..p[8] as usize).map(|i|String::from_utf8(slice(p,10+10*i+8).iter().map(|&x|x as u8).collect()).unwrap()).collect();let hashes=fingerprints(raws);for i in 0..p[8] as usize{let b=10+10*i;let raw:Vec<u8>=slice(p,b+8).iter().map(|&x|x as u8).collect();let mut v=json!({"name":text(p,b),"readOnly":p[b+6]==1,"fingerprint":hashes[i]});if p[b+2]>=0{v["title"]=json!(text(p,b+2));}if p[b+4]>=0{v["description"]=json!(text(p,b+4));}if p[b+7]>=0{v["destructive"]=json!(p[b+7]==1);}tools.push(v);}return json!({"ok":true,"stage":stage,"tools":tools})}
 let message=match p[2]{1=>format!("명령을 찾지 못했어: {command}"),2=>format!("명령을 시작하지 못했어: {command}"),3=>format!("시간 초과: {}초 안에 끝나지 않았어.",((ms.max(0)+500)/1000)),4=>"초기화 응답이 30초 안에 오지 않았어.".into(),5=>"서버가 너무 긴 줄을 보냈어.".into(),6=>{let how=if p[3]>=0{format!("종료 코드 {}",p[3])}else{format!("신호 {}",-p[3])};let mut s=format!("{}({how})",if p[1]==1{"서버가 시작 직후 끝났어"}else{"서버가 끝났어"});let err=String::from_utf8_lossy(&slice(p,4).iter().map(|&x|x as u8).collect::<Vec<_>>()).into_owned();let last=err.as_str();if !last.is_empty(){s.push_str(": ");s.push_str(&one(last,200));}s},8=>{let s=if p[6]<0{"서버 오류".into()}else{text(p,6)};if p[1]==1{format!("초기화를 거절했어: {s}")}else{s}},_=>"연결이 닫혔어.".into()};json!({"ok":false,"stage":stage,"message":one(&message,300),"tools":[]})}

// Diagnostic lookup only; called after a G10 OS launch failure.
pub fn command_missing(command: &str, cwd: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    let base = Path::new(cwd);
    if command.contains('/') {
        return !base.join(command).exists();
    }
    let path = std::env::var_os("PATH").unwrap_or_else(|| "/usr/bin:/bin".into());
    !std::env::split_paths(&path).any(|dir| {
        let candidate = base.join(dir).join(command);
        candidate.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    })
}
