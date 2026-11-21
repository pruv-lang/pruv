#![allow(dead_code, unused_imports, unused_variables)]
#[path="@SOL@"] mod sol;
#[cfg(entry_forge)]
#[path="@FORGE@"] mod forge;

use std::cell::RefCell;
use std::rc::Rc;

// The await packet is immutable and shared with the driver. The progress array
// stays in the adapter. Pruv locates the blob in the saved await.
struct Index {
    progress: Vec<i64>,
    emitted: Rc<Vec<i64>>,
}

fn index_progress_len(input: &[i64]) -> usize {
    let session_count = || {
        let nsec = usize::try_from(*input.get(6)?).ok()?;
        let pool_len = usize::try_from(*input.get(7)?).ok()?;
        let count_at = 9usize.checked_add(nsec)?.checked_add(pool_len)?;
        Some((*input.get(count_at)?).clamp(0, 559_240) as usize)
    };
    let max_sessions = input.len() / 30;
    match session_count() {
        Some(ns) => ns.min(max_sessions) + 1,
        None => input.len().min(16_777_216) / 30 + 1,
    }
}

struct G {
    report_prev: bool,
    index: RefCell<Option<Index>>,
    #[cfg(entry_forge)]
    forge: forge::Forge,
}

impl entry_host::Sol for G {
    fn begin(&self, input: &[i64]) -> Rc<Vec<i64>> {
        self.index.take();
        if input.get(5) != Some(&9) {
            return Rc::new(sol::entry_begin(input));
        }
        let mut progress = vec![0; index_progress_len(input)];
        let emitted = Rc::new(sol::index_begin(&mut progress, input));
        #[cfg(entry_forge)]
        let emitted = if let Some(other) = self.forge.index_input(input) {
            let mut other_progress = vec![0; index_progress_len(&other)];
            Rc::new(sol::index_begin(&mut other_progress, &other))
        } else {
            emitted
        };
        if emitted[1] == 1 {
            self.index.replace(Some(Index { progress, emitted: Rc::clone(&emitted) }));
        }
        emitted
    }

    fn step(&self, prev: &[i64], completion: &[i64]) -> Vec<i64> {
        if self.report_prev {
            eprintln!("r02_prev_words {}", prev.len());
        }
        #[cfg(entry_forge)]
        let tampered = self.forge.prev(prev);
        #[cfg(entry_forge)]
        let prev = tampered.as_deref().unwrap_or(prev);

        if let Some(Index { mut progress, emitted }) = self.index.take() {
            return sol::index_step(&mut progress, prev, &emitted, completion);
        }
        sol::entry_step(prev, completion)
    }
}

fn main() {
    entry_host::run(&G {
        report_prev: std::env::var_os("PRUV_R02_REPORT_PREV").is_some(),
        index: RefCell::new(None),
        #[cfg(entry_forge)]
        forge: forge::Forge::from_env(),
    });
}
