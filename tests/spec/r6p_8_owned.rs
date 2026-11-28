use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering::SeqCst};

struct Counting;
static WATCH: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());
static FREES: AtomicUsize = AtomicUsize::new(0);
#[global_allocator]
static ALLOC: Counting = Counting;
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 { System.alloc(layout) }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        if WATCH.compare_exchange(p, std::ptr::null_mut(), SeqCst, SeqCst).is_ok() {
            FREES.fetch_add(1, SeqCst);
        }
        System.dealloc(p, layout);
    }
}
fn watch(s: &String) {
    FREES.store(0, SeqCst);
    WATCH.store(s.as_ptr() as *mut u8, SeqCst);
}
fn main() {
    assert_eq!(sol::read_ints(&sol::keep_ints(vec![])), 0);
    assert_eq!(sol::read_flags(&sol::keep_flags(vec![])), 0);
    assert_eq!(sol::read_text(&sol::keep_text(String::new())), 0);
    assert_eq!(sol::read_str(&sol::into_text(String::new())), 0);
    let s = String::from("é🦀");
    assert_eq!(sol::read_text(&s), 6);
    assert_eq!(sol::read_str(&s), 6);
    let p = s.as_ptr();
    watch(&s);
    let result = sol::keep_text(s);
    assert_eq!(p, result.as_ptr());
    assert_eq!(FREES.load(SeqCst), 0);
    drop(result);
    assert_eq!(FREES.load(SeqCst), 1);
    for ok in [true, false] {
        let s = String::from("taken");
        watch(&s);
        let ret = std::panic::catch_unwind(|| sol::consume(s, ok));
        assert_eq!(ret.ok(), ok.then_some(5));
        assert_eq!(FREES.load(SeqCst), 1);
    }
    let v = vec![sol::R { data: vec![17], title: "item".into() }];
    let ptrs = (v.as_ptr(), v[0].data.as_ptr());
    assert_eq!(sol::read_records(&v), 1);
    let v = sol::keep_records(v);
    assert_eq!(ptrs, (v.as_ptr(), v[0].data.as_ptr()));
    let mut st = sol::State { n: 1, data: vec![2], title: "state".into() };
    sol::update(&mut st, 7);
    assert_eq!(st.n, 7);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| sol::update(&mut st, 0))).is_err());
    assert_eq!((st.n, st.data.as_slice(), st.title.as_str()), (7, &[2][..], "state"));
    println!("ok");
}
