use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering::SeqCst};

struct Alloc;
#[global_allocator]
static ALLOC: Alloc = Alloc;
static RESULT: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());
static FREES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Alloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 { System.alloc(layout) }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        if RESULT.compare_exchange(p, std::ptr::null_mut(), SeqCst, SeqCst).is_ok() {
            assert_eq!((layout.size(), layout.align()), (4, 2));
            FREES.fetch_add(1, SeqCst);
        }
        System.dealloc(p, layout)
    }
}

fn main() {
    assert_eq!(sol::id_u8(u8::MIN), u8::MIN);
    assert_eq!(sol::id_u8(u8::MAX), u8::MAX);
    assert_eq!(sol::id_u16(u16::MIN), u16::MIN);
    assert_eq!(sol::id_u16(u16::MAX), u16::MAX);
    assert_eq!(sol::id_u32(u32::MIN), u32::MIN);
    assert_eq!(sol::id_u32(u32::MAX), u32::MAX);
    assert_eq!(sol::id_u64(u64::MIN), u64::MIN);
    assert_eq!(sol::id_u64(u64::MAX), u64::MAX);
    assert_eq!(sol::id_usize(usize::MIN), usize::MIN);
    assert_eq!(sol::id_usize(usize::MAX), usize::MAX);
    assert_eq!(sol::id_i8(i8::MIN), i8::MIN);
    assert_eq!(sol::id_i8(i8::MAX), i8::MAX);
    assert_eq!(sol::id_i16(i16::MIN), i16::MIN);
    assert_eq!(sol::id_i16(i16::MAX), i16::MAX);
    assert_eq!(sol::id_i32(i32::MIN), i32::MIN);
    assert_eq!(sol::id_i32(i32::MAX), i32::MAX);
    assert_eq!(sol::id_isize(isize::MIN), isize::MIN);
    assert_eq!(sol::id_isize(isize::MAX), isize::MAX);
    assert_eq!(sol::positive(u64::MAX), u64::MAX);
    let p = sol::round_trip(sol::Mixed { a: 255, b: -32768, c: u64::MAX });
    assert_eq!((p.a, p.b, p.c), (255, -32768, u64::MAX));
    assert!(sol::units(255, -1, u64::MAX, &[]).is_empty());
    let v = sol::units(255, -1, u64::MAX, &[0xD800, 0xFFFF]);
    assert_eq!(v, [0xD800, 0xFFFF]);
    assert_eq!(v.capacity(), 2);
    RESULT.store(v.as_ptr() as *mut u8, SeqCst);
    drop(v);
    assert_eq!(FREES.load(SeqCst), 1);
}
