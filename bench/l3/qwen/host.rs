#![allow(dead_code, unused_imports)]
#[path = "@SOL@"] mod sol;
use std::{fs::{self,File},io::{Read,Seek,SeekFrom},path::{Path,PathBuf},time::{SystemTime,UNIX_EPOCH}};
#[cfg(unix)] use std::os::unix::fs::MetadataExt;
fn q(s:&str)->String {
 let mut out=String::from("\"");
 for c in s.chars() { match c { '"' => out.push_str("\\\""), '\\'=>out.push_str("\\\\"), '\n'=>out.push_str("\\n"), '\r'=>out.push_str("\\r"), '\t'=>out.push_str("\\t"), c if (c as u32)<32 => out.push_str(&format!("\\u{:04x}",c as u32)), c=>out.push(c) } }
 out.push('"');out
}
struct Packet { m:Vec<i64>, turns:Vec<i64>, text:Vec<u8> }
impl Packet {
 fn new(v:Vec<i64>)->Self {
  assert!(v.len()>=18 && v[0]==0 && v[1]==14 && v[2]>=0 && v[3]>=0);
  assert_eq!(v.len(),18+v[2] as usize+v[3] as usize);
  let m=v[4..18].to_vec();assert_eq!(m[0],1);assert_eq!(v[2],10*m[3]);
  let end=18+v[2] as usize;
  let text=v[end..].iter().map(|&b|{assert!((0..=255).contains(&b));b as u8}).collect();
  Self {m,turns:v[18..end].to_vec(),text}
 }
 fn get(&self,a:i64,n:i64)->Option<String> {
  if a == -1 { assert_eq!(n,0);return None; }
  assert!(a>=0 && n>=0 && a as usize+n as usize<=self.text.len());
  Some(String::from_utf8(self.text[a as usize..(a+n) as usize].to_vec()).unwrap())
 }
 fn field(&self,i:usize)->Option<String>{self.get(self.m[i],self.m[i+1])}
}
fn optional(fields:&mut Vec<String>,name:&str,value:Option<String>){if let Some(v)=value{fields.push(format!("{}:{}",q(name),q(&v)));}}
fn object(fields:Vec<String>)->String{format!("{{{}}}",fields.join(","))}
fn head_json(h:&Packet)->String{
 let mut f=vec![format!("\"human\":{}",h.field(4).is_some())];
 optional(&mut f,"cwd",h.field(6));optional(&mut f,"firstPrompt",h.field(4));object(f)
}
fn tail_json(t:&Packet)->String{let mut f=vec![];optional(&mut f,"title",t.field(4));optional(&mut f,"lastAt",t.field(10));object(f)}
fn title(h:&Packet,w:&Packet)->Option<String>{
 w.field(4).or_else(||h.field(4).map(|s|String::from_utf8(sol::title_bytes(s.as_bytes())).unwrap()))
}
fn record(h:&Packet,w:&Packet)->String {
 let turns=w.turns.chunks_exact(10).map(|r|{
  let mut f=vec![];
  for (i,name) in ["prompt","reply","createdAt","completedAt"].iter().enumerate(){optional(&mut f,name,w.get(r[2*i],r[2*i+1]));}
  f.push(format!("\"tools\":{}",r[8]));if r[9]!=0 {f.push("\"clipped\":true".into());}object(f)
 }).collect::<Vec<_>>();
 let mut f=vec![format!("\"turns\":[{}]",turns.join(",")),format!("\"omittedBefore\":{}",w.m[2]!=0)];
 optional(&mut f,"title",title(h,w));optional(&mut f,"model",w.field(12));object(f)
}
fn bytes(file:&mut File,start:u64,n:usize)->Vec<u8>{
 file.seek(SeekFrom::Start(start)).unwrap();let mut b=vec![0;n];let mut used=0;
 while used<n{let k=file.read(&mut b[used..]).unwrap();if k==0{break}used+=k;}b.truncate(used);b
}
fn packets(path:&Path)->(Packet,Packet,Packet){
 let before=fs::symlink_metadata(path).unwrap();
 assert!(before.is_file()&&!before.file_type().is_symlink());
 let mut file=File::open(path).unwrap();let opened=file.metadata().unwrap();
 #[cfg(unix)] { assert_eq!(before.dev(),opened.dev());assert_eq!(before.ino(),opened.ino()); }
 let size=opened.len();
 let h=Packet::new(sol::head_packet(&bytes(&mut file,0,size.min(65536) as usize),(size>65536) as i64));
 let t=Packet::new(sol::tail_packet(&bytes(&mut file,size.saturating_sub(65536),size.min(65536) as usize),(size>65536) as i64));
 let w=Packet::new(sol::window_packet(&bytes(&mut file,size.saturating_sub(8388608),size.min(8388608) as usize),(size>8388608) as i64));(h,t,w)
}
fn key(s:&str)->&str{let k=s.trim_end_matches('/');if k.is_empty(){s}else{k}}
fn plain(home:&Path,path:&Path)->bool{
 fs::symlink_metadata(path).map(|m|m.is_file()&&!m.file_type().is_symlink()).unwrap_or(false)
 && fs::canonicalize(path).map(|p|p.starts_with(home)).unwrap_or(false)
}
fn list(home:&Path,folder:&str)->String{
 let home_real=fs::canonicalize(home).unwrap();
 let project=String::from_utf8(sol::project_name(folder.as_bytes())).unwrap();
 if project.is_empty(){return "[]".into();}
 let mut dir=home.to_path_buf();
 for part in ["projects",&project,"chats"] {
  dir.push(part);
  if !fs::symlink_metadata(&dir).map(|m|m.is_dir()&&!m.file_type().is_symlink()).unwrap_or(false){return "[]".into();}
 }
 let mut files=fs::read_dir(dir).unwrap().filter_map(Result::ok).filter_map(|e|{
  let name=e.file_name().into_string().ok()?;let thread=name.strip_suffix(".jsonl")?;
  if sol::valid_thread(thread.as_bytes())!=1 || !plain(&home_real,&e.path()){return None;}
  let m=e.metadata().ok()?;Some((e.path(),thread.to_string(),m.len(),m.modified().ok()?))
 }).collect::<Vec<_>>();
 files.sort_by(|a,b|b.3.cmp(&a.3));let now=SystemTime::now();let mut out=vec![];
 for (path,thread,size,mtime) in files {
  let (h,t,_)=packets(&path);
  if h.field(6).map(|cwd|key(&cwd)!=key(folder)).unwrap_or(false) || (h.field(4).is_none()&&size<=65536){continue;}
  let ms=mtime.duration_since(UNIX_EPOCH).unwrap().as_millis();
  let stamp=Packet::new(sol::tail_packet(format!("{{\"timestamp\":{ms}}}\n").as_bytes(),0));
  let mut f=vec![format!("\"thread\":{}",q(&thread)),format!("\"title\":{}",q(&title(&h,&t).unwrap_or_default())),format!("\"size\":{size}"),format!("\"path\":{}",q(path.to_str().unwrap()))];
  optional(&mut f,"updatedAt",t.field(10).or_else(||stamp.field(10)));
  if now.duration_since(mtime).map(|d|d.as_millis()<120000).unwrap_or(true){f.push("\"maybeOpen\":true".into());}
  out.push(object(f));if out.len()==50{break;}
 }
 format!("[{}]",out.join(","))
}
fn main(){
 let a=std::env::args().collect::<Vec<_>>();
 if a[1]=="--list"{println!("{}",list(Path::new(&a[2]),&a[3]));return;}
 if a[1]=="--names"{println!("{{\"project\":{},\"valid\":{}}}",q(&String::from_utf8(sol::project_name(a[2].as_bytes())).unwrap()),sol::valid_thread(a[3].as_bytes())==1);return;}
 for path in &a[1..] {
  let(h,t,w)=packets(Path::new(path));
  println!("{{\"head\":{},\"tail\":{},\"record\":{}}}",head_json(&h),tail_json(&t),record(&h,&w));
 }
}

