fn main(){
 let child=std::env::var("PRUV_PROC_CHILD").unwrap();
 let big:Vec<u8>=(0..1048576u32).map(|i|(i%251) as u8).collect();
 // A child that never reads: the 1 MiB write half-closes at its 50 ms deadline, "ready\n" is still read.
 let t0=std::time::Instant::now();
 assert_eq!(sol::run(&child,"hold","0",1,&big,&[1,2,3,4,5],50),1006);
 assert!(t0.elapsed()<std::time::Duration::from_secs(10),"bounded write blocked: {:?}",t0.elapsed());
 // A child that takes 10 bytes and exits (Grok P11): the broken pipe half-closes, its 10 output bytes and exit 0 are read.
 assert_eq!(sol::run(&child,"take","10",2,&big,&[1,2,3,4,5],20000),2010);
 // A completed write keeps stdin open; the later plain write and close go through the writer: 1048576 + 5 bytes.
 assert_eq!(sol::run(&child,"count","0",3,&big,&[1,2,3,4,5],20000),3001048581);
 // An empty write returns at once and keeps stdin open.
 assert_eq!(sol::run(&child,"count","0",4,&[],&[1,2,3,4,5],50),4000);
 assert_eq!(sol::PRUV_ASYNC_THREADS.load(std::sync::atomic::Ordering::SeqCst),0);
 println!("async runtime ok");
}
