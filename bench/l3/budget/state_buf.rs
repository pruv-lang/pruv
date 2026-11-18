// State buffer allocation of the budget host (L3 P13c). Included with include! by host/src/lib.rs (before host.rs) and
// by the M4 test driver (m34_test/run.py writes the include into main_<tag>.rs). Plain `//` comments only, full std
// paths, no `use` lines (include! files share the includer's imports).
//
// Allocation is the host's job; every sizing decision stays in Pruv (bud_store::need gives N, the host allocates exactly
// N words). A grown state replaces the old buffer, which is dropped at once. With the default allocator a dropped
// state-sized block can stay resident: on macOS the malloc medium zone keeps freed 32 KiB - 8 MiB blocks resident until
// memory pressure, and the next, larger buffer cannot reuse them; on Linux glibc serves blocks of 128 KiB and more with
// mmap at first, but the threshold rises to the size of each freed mapped block (dynamic M_MMAP_THRESHOLD), so which
// path a later buffer takes depends on the allocation history. So a state buffer of STATE_MAP_BYTES or more is mapped directly from the OS (anonymous private mapping: zero-filled, pages become resident only when touched) and
// unmapped on drop, which returns its pages at once. Smaller buffers, and every target without the mapping path
// (Windows and other systems), use the global allocator with alloc_zeroed (also lazily zeroed for large blocks there).
// The two paths behave the same to callers; only residency after drop differs.

// Byte size from which a state buffer is mapped: 4 pages of 16 KiB (macOS arm64), 16 pages of 4 KiB. Below this the
// malloc zones reuse freed blocks well and a mapping would waste most of a page.
pub const STATE_MAP_BYTES: usize = 64 * 1024;

// Number of mapped state buffers created (zeroed and clone), so drivers that count global-allocator calls can add the
// mappings to their allocation count (same accounting as the default path, where each buffer is one allocator call).
// usize atomics exist on every std target (64-bit atomics do not).
pub static STATE_MAPS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(all(
    target_pointer_width = "64",
    any(
        target_os = "macos",
        target_os = "ios",
        all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "riscv64"))
    )
))]
mod state_map {
    const PROT_READ_WRITE: i32 = 0x1 | 0x2;
    const MAP_PRIVATE: i32 = 0x02;
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    const MAP_ANON: i32 = 0x1000;
    #[cfg(target_os = "linux")]
    const MAP_ANON: i32 = 0x20;
    extern "C" {
        fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut u8;
        fn munmap(addr: *mut u8, len: usize) -> i32;
    }
    pub const AVAILABLE: bool = true;
    // A fresh zero-filled mapping of `bytes` bytes, or null if the OS refuses.
    pub unsafe fn map(bytes: usize) -> *mut u8 {
        let p = mmap(std::ptr::null_mut(), bytes, PROT_READ_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0);
        if p as isize == -1 {
            return std::ptr::null_mut();
        }
        if p.is_null() {
            // A mapping at address 0 cannot back a NonNull buffer: give it back and report a refusal.
            let _ = munmap(p, bytes);
        }
        p
    }
    pub unsafe fn unmap(p: *mut u8, bytes: usize) {
        // munmap of a mapping we own with its exact length cannot fail; there is nothing to do if it did.
        let _ = munmap(p, bytes);
    }
}

#[cfg(not(all(
    target_pointer_width = "64",
    any(
        target_os = "macos",
        target_os = "ios",
        all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64", target_arch = "riscv64"))
    )
)))]
mod state_map {
    pub const AVAILABLE: bool = false;
    pub unsafe fn map(_bytes: usize) -> *mut u8 {
        std::ptr::null_mut()
    }
    pub unsafe fn unmap(_p: *mut u8, _bytes: usize) {}
}

// An owned, zero-initialized [i64] of fixed length (the resident state). Derefs to a slice, which is what the generated
// entries take for `[i64]` / `inout [i64]`.
pub struct StateBuf {
    ptr: std::ptr::NonNull<i64>,
    len: usize,
}

// SAFETY: StateBuf owns its words exclusively, like Vec<i64>.
unsafe impl Send for StateBuf {}
unsafe impl Sync for StateBuf {}

impl StateBuf {
    fn mapped(bytes: usize) -> bool {
        state_map::AVAILABLE && bytes >= STATE_MAP_BYTES
    }

    // n zero words, or None when the allocation is refused (the caller keeps its old state).
    pub fn zeroed(n: usize) -> Option<StateBuf> {
        if n == 0 {
            return Some(StateBuf { ptr: std::ptr::NonNull::dangling(), len: 0 });
        }
        let layout = std::alloc::Layout::array::<i64>(n).ok()?;
        let bytes = layout.size();
        // SAFETY: non-zero size. A mapping is page aligned (so aligned for i64) and zero-filled; alloc_zeroed returns
        // zeroed memory for this layout. All-zero bytes are valid i64 values. Drop frees with the same path, chosen
        // again from the same byte size.
        let p = unsafe {
            if StateBuf::mapped(bytes) {
                let p = state_map::map(bytes);
                if !p.is_null() {
                    STATE_MAPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
                p
            } else {
                std::alloc::alloc_zeroed(layout)
            }
        };
        std::ptr::NonNull::new(p as *mut i64).map(|ptr| StateBuf { ptr, len: n })
    }

    // n zero words for paths that cannot report a refusal (initial state, copies): an allocation failure ends the
    // process the way `vec![0; n]` does.
    pub fn zeroed_or_abort(n: usize) -> StateBuf {
        match StateBuf::zeroed(n) {
            Some(b) => b,
            None => match std::alloc::Layout::array::<i64>(n) {
                Ok(l) => std::alloc::handle_alloc_error(l),
                Err(_) => panic!("state buffer of {} words is too large", n),
            },
        }
    }
}

impl std::ops::Deref for StateBuf {
    type Target = [i64];
    fn deref(&self) -> &[i64] {
        // SAFETY: ptr points to len initialized i64 words owned by self (dangling with len 0 is a valid empty slice).
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
}

impl std::ops::DerefMut for StateBuf {
    fn deref_mut(&mut self) -> &mut [i64] {
        // SAFETY: as in deref, and &mut self gives exclusive access.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

impl Clone for StateBuf {
    fn clone(&self) -> StateBuf {
        let mut b = StateBuf::zeroed_or_abort(self.len);
        b.copy_from_slice(self);
        b
    }
}

impl Drop for StateBuf {
    fn drop(&mut self) {
        if self.len == 0 {
            return;
        }
        // The layout was valid when the buffer was made, with the same len.
        let layout = std::alloc::Layout::array::<i64>(self.len).expect("state buffer layout");
        let bytes = layout.size();
        // SAFETY: frees with the path zeroed chose for this byte size; the buffer is not used after this.
        unsafe {
            if StateBuf::mapped(bytes) {
                state_map::unmap(self.ptr.as_ptr() as *mut u8, bytes);
            } else {
                std::alloc::dealloc(self.ptr.as_ptr() as *mut u8, layout);
            }
        }
    }
}

impl std::fmt::Debug for StateBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StateBuf({} words)", self.len)
    }
}
