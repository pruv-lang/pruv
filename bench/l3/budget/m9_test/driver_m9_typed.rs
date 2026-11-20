// Typed boundary for the shared M9 scheduler and benchmark workloads.
include!("driver_m9.rs");

// Perf shares the hand lane's decoded inputs and timer host. Q keeps the wire codec below.
#[path = "hand_m9.rs"]
mod typed_perf;

// Encode each UTF-16 code unit independently, including lone surrogates.
const SURROGATE_OFFSET: u32 = 0x10000 - 0xd800;

#[test]
fn churn_order() {
    let mut st = World::<false>::new_state();
    assert_eq!(sol::t_put(&mut st, "main", "keep", 0, true, 0, 0, 0, 0), 0);
    assert_eq!(sol::t_put(&mut st, "other", "drop", 0, true, 0, 0, 0, 0), 0);
    let call = |st: &mut sol::St, code, sid, bid, run, waiting| {
        assert_eq!(sol::t_call(st, code, sid, bid, run, waiting, 0), 0);
    };
    // Delete and reinsert both a run and a clock while earlier entries survive.
    call(&mut st, 1, "other", "drop", "b", false);
    call(&mut st, 1, "main", "keep", "b", false);
    call(&mut st, 1, "main", "keep", "a", false);
    call(&mut st, 2, "main", "keep", "a", true);
    for _ in 0..1_000 {
        call(&mut st, 3, "main", "keep", "b", false);
        call(&mut st, 5, "other", "drop", "", false);
        let clock = &st.clocks["keep"];
        assert_eq!(clock.runs.len(), 1);
        assert_eq!(st.clocks.len(), 1);
        assert!(clock.runs["a"].waiting);
        assert_eq!(clock.runs["a"].ordinal, 1);
        assert_eq!(clock.ordinal, 1);
        assert_eq!(sol::t_i2(&mut st), 0);
        call(&mut st, 1, "main", "keep", "b", false);
        call(&mut st, 1, "other", "drop", "b", false);
    }
    assert_eq!(st.clocks["keep"].runs["b"].ordinal, 1_001);
    assert_eq!(st.clocks["drop"].ordinal, 1_001);
    assert_eq!(sol::t_i2(&mut st), 0);

    let limit = 1_i64 << 40;
    st.next_session = limit;
    st.next_clock = limit;
    st.clocks.get_mut("keep").unwrap().next_run = limit;
    assert_eq!(sol::t_put(&mut st, "new", "new", 0, true, 0, 0, 0, 0), 2);
    assert_eq!(sol::t_put(&mut st, "other", "new", 0, true, 0, 0, 0, 0), 0);
    assert_eq!(sol::t_call(&mut st, 1, "other", "new", "b", false, 0), 2);
    assert_eq!(sol::t_call(&mut st, 1, "main", "keep", "new", false, 0), 2);
    call(&mut st, 1, "main", "keep", "a", false);
    assert_eq!(st.sessions["other"].ordinal, 1);
    assert_eq!(st.clocks.len(), 2);
    assert_eq!(st.clocks["keep"].runs.len(), 2);
    assert_eq!(sol::t_i2(&mut st), 0);
}

fn take_typed_key(input: &mut &[i64], key: &mut String) -> Option<()> {
    let (&n, rest) = input.split_first()?;
    if !(0..=1 << 30).contains(&n) {
        return None;
    }
    let n = n as usize;
    let packed = rest.get(..n.div_ceil(4))?;
    let padding = (4 - n % 4) % 4;
    if padding != 0 && (*packed.last()? as u64 & ((1 << (16 * padding)) - 1)) != 0 {
        return None;
    }
    key.clear();
    key.reserve(n);
    for i in 0..n {
        let unit = ((packed[i / 4] as u64 >> (16 * (3 - i % 4))) & 0xffff) as u32;
        key.push(char::from_u32(unit).unwrap_or_else(|| char::from_u32(unit + SURROGATE_OFFSET).unwrap()));
    }
    *input = &rest[packed.len()..];
    Some(())
}

fn typed_units(s: &str) -> impl Iterator<Item = i64> + '_ {
    s.chars().map(|c| {
        let scalar = c as u32;
        i64::from(if scalar < 0x10000 { scalar } else { scalar - SURROGATE_OFFSET })
    })
}

fn append_typed_key(out: &mut Vec<i64>, key: &str) {
    out.push(key.chars().count() as i64);
    out.extend(typed_units(key));
}

impl<const CHECK_I2: bool> World<CHECK_I2> {
    fn new_state() -> sol::St {
        sol::St {
            sessions: HashMap::new(),
            next_session: 0,
            clocks: HashMap::new(),
            next_clock: 0,
            out: sol::Out { fx: Vec::new() },
        }
    }

    fn put(&mut self, mut op: &[i64]) {
        let [sid, bid, _] = &mut self.keys;
        let parsed = (|| {
            take_typed_key(&mut op, sid)?;
            let (&n, record) = op.split_first()?;
            if n != record.len() as i64 {
                return None;
            }
            let &[state, unlimited, minutes, active_ms, since_kind, since, ref tail @ ..] = record else {
                return None;
            };
            let mut tail = tail;
            take_typed_key(&mut tail, bid)?;
            tail.is_empty().then(|| sol::t_put(
                &mut self.st, sid, bid, state, unlimited == 1, minutes, active_ms, since_kind, since,
            ))
        })();
        if CHECK_I2 {
            self.check_i2();
        }
        if parsed != Some(0) {
            eprintln!("typed t_put status {}", parsed.unwrap_or(2));
            std::process::exit(2);
        }
    }

    fn invoke(&mut self, op: &[i64]) -> i64 {
        self.fx[0] = 0;
        let [sid, bid, run] = &mut self.keys;
        let result = (|| {
            let (&code, mut rest) = op.split_first()?;
            take_typed_key(&mut rest, sid)?;
            take_typed_key(&mut rest, bid)?;
            take_typed_key(&mut rest, run)?;
            let &[flag] = rest else { return None };
            Some(sol::t_call(&mut self.st, code, sid, bid, run, flag == 1, self.now))
        })();
        let Some(status) = result else { return 2 };
        let mut i = 1;
        for effect in &self.st.out.fx {
            self.fx[i] = effect.tag;
            i += 1;
            if effect.tag == 12 {
                self.fx[i] = effect.requested;
                self.fx[i + 1] = effect.effective;
                i += 2;
            }
        }
        self.fx[0] = (i - 1) as i64;
        if CHECK_I2 {
            self.check_i2();
        }
        status
    }

    fn i2(&mut self) -> i64 {
        sol::t_i2(&mut self.st)
    }

    fn observation(&mut self) -> Result<&[i64], i64> {
        let reply = sol::t_observe(&mut self.st);
        let mut keys = reply.keys.into_iter();
        let mut words = reply.words.into_iter();
        let out = &mut self.out;
        out.clear();

        let session_count = words.next().unwrap();
        let mut sessions = Vec::with_capacity(session_count as usize);
        for _ in 0..session_count {
            let ordinal = words.next().unwrap();
            let sid = keys.next().unwrap();
            let bid = keys.next().unwrap();
            let fields: [i64; 6] = std::array::from_fn(|_| words.next().unwrap());
            sessions.push((ordinal, sid, bid, fields));
        }
        sessions.sort_unstable_by_key(|entry| entry.0);
        out.push(session_count);
        for (_, sid, bid, fields) in sessions {
            append_typed_key(out, &sid);
            let units: Vec<_> = typed_units(&bid).collect();
            out.push(7 + units.len().div_ceil(4) as i64);
            out.extend(fields);
            out.push(units.len() as i64);
            pack_into(&units, out);
        }

        let clock_count = words.next().unwrap();
        let mut clocks = Vec::with_capacity(clock_count as usize);
        for _ in 0..clock_count {
            let ordinal = words.next().unwrap();
            let bid = keys.next().unwrap();
            let owner = keys.next().unwrap();
            let fields: [i64; 4] = std::array::from_fn(|_| words.next().unwrap());
            let run_count = words.next().unwrap();
            let mut runs = Vec::with_capacity(run_count as usize);
            for _ in 0..run_count {
                let ordinal = words.next().unwrap();
                let run = keys.next().unwrap();
                let waiting = words.next().unwrap();
                runs.push((ordinal, run, waiting));
            }
            clocks.push((ordinal, bid, owner, fields, runs));
        }
        clocks.sort_unstable_by_key(|entry| entry.0);
        out.push(clock_count);
        for (_, bid, owner, fields, mut runs) in clocks {
            append_typed_key(out, &bid);
            append_typed_key(out, &owner);
            out.extend(fields);
            out.push(runs.len() as i64);
            runs.sort_unstable_by_key(|entry| entry.0);
            for (_, run, waiting) in runs {
                append_typed_key(out, &run);
                out.push(waiting);
            }
        }
        assert!(words.next().is_none() && keys.next().is_none());
        out.push(0); // The shared scheduler appends its timers.
        Ok(out)
    }

    fn live(&self) -> (i64, usize) {
        let entries = 3 + self.st.sessions.len() + self.st.clocks.len()
            + self.st.clocks.values().map(|clock| clock.runs.len()).sum::<usize>();
        // Only the top-level tables remain allocated after DROP/CLOSE.
        let capacity = self.st.sessions.capacity() + self.st.clocks.capacity();
        (entries as i64, capacity)
    }
}
