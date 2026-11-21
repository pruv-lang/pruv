#![allow(dead_code, unused_imports, unused_variables)]
#[path="@SOL@"] mod sol;
#[cfg(entry_forge)]
#[path="@FORGE@"] mod forge;

use std::cell::RefCell;
use std::rc::Rc;

// The await packet is immutable and shared with the driver. The progress array and the
// input given to index_begin stay in the adapter; the await does not copy the input.
struct Index {
    progress: Vec<i64>,
    input: Vec<i64>,
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
    fn begin(&self, input: Vec<i64>) -> Rc<Vec<i64>> {
        self.index.take();
        if input.get(5) != Some(&9) {
            return Rc::new(sol::entry_begin(&input));
        }
        #[allow(unused_mut)]
        let mut input = input;
        #[cfg(entry_forge)]
        #[allow(unused_mut)]
        let mut input = self.forge.index_tail(input);
        let mut progress = vec![0; index_progress_len(&input)];
        #[cfg(entry_forge)]
        let other = self.forge.index_input(&input);
        let emitted = Rc::new(sol::index_begin(&mut progress, &input));
        // Forge: keep progress from A with the await from B (and B's input, which the await came with).
        #[cfg(entry_forge)]
        let emitted = if let Some(other) = other {
            let mut other_progress = vec![0; index_progress_len(&other)];
            let e = Rc::new(sol::index_begin(&mut other_progress, &other));
            input = other;
            e
        } else {
            emitted
        };
        if emitted[1] == 1 {
            #[cfg(entry_forge)]
            self.forge.index_stats(&emitted, &progress);
            self.index.replace(Some(Index { progress, input, emitted: Rc::clone(&emitted) }));
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

        if let Some(Index { mut progress, input, emitted }) = self.index.take() {
            return sol::index_step(&mut progress, &input, prev, &emitted, completion);
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
