fn main(){
 let child=std::env::var("PRUV_PROC_CHILD").unwrap();
 let big:Vec<u8>=vec![1u8;65536];
 // Mode 2 checks a half-close, then reads stdout and waits for exit 0: count printed "0\n" (2 bytes).
 assert_eq!(sol::run(&child,"count","0",2,&big,&[1,2,3,4,5],20000),2002);
 assert_eq!(sol::PRUV_ASYNC_THREADS.load(std::sync::atomic::Ordering::SeqCst),0);
 println!("async runtime ok");
}
