fn main(){
 let child=std::env::var("PRUV_PROC_CHILD").unwrap();
 let big:Vec<u8>=vec![1u8;65536];
 // Mode 2: the panicked writer half-closes stdin, the child prints "0\n" and exits 0; the end reports code 8.
 assert_eq!(sol::run(&child,"count","0",2,&big,&[1,2,3,4,5],20000),-108);
 assert_eq!(sol::PRUV_ASYNC_THREADS.load(std::sync::atomic::Ordering::SeqCst),0);
 println!("async runtime ok");
}
