fn main(){
 let child=std::env::var("PRUV_PROC_CHILD").unwrap();
 let big:Vec<u8>=vec![7u8;1048576];
 let t0=std::time::Instant::now();
 assert_eq!(sol::run(&child,"close_out",1,&[]),3);
 let t1=std::time::Instant::now();
 assert_eq!(sol::run(&child,"sleep",2,&[]),100);
 assert!(t1.elapsed()<std::time::Duration::from_secs(10),"bounded wait blocked: {:?}",t1.elapsed());
 // The failed slot is freed without waiting for the 24 h deadline; ev_close reports the write error (code 7).
 let t2=std::time::Instant::now();
 assert_eq!(sol::run(&child,"quick",3,&big),7200);
 assert!(t2.elapsed()<std::time::Duration::from_secs(10),"failed slot waited: {:?}",t2.elapsed());
 assert!(t0.elapsed()<std::time::Duration::from_secs(30));
 assert_eq!(sol::PRUV_ASYNC_THREADS.load(std::sync::atomic::Ordering::SeqCst),0);
 println!("async runtime ok");
}
