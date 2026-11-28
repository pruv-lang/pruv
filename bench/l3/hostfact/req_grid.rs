#[allow(dead_code)]
#[path = "@REQ@"]
mod req;
#[allow(dead_code)]
#[path = "@SOL@"]
mod sol;

use hostfact_host::*;
use req::*;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::hint::black_box;
use std::time::Instant;

const SOURCE: &str = "claude:home";
const FOLDER: &str = "/work/proj";
const TARGET: &str = "tgt";
const CREATED: &str = "created";
type Row = (i64, Option<String>, String);

thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
}

struct Guard;
impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE.with(|active| active.set(false));
    }
}

fn guarded(call: impl FnOnce() -> i64) -> i64 {
    const REFUSED_REENTRY: i64 = 9;
    if ACTIVE.with(|active| active.replace(true)) {
        return REFUSED_REENTRY;
    }
    let _guard = Guard;
    call()
}

fn pruv_list() -> i64 { guarded(|| sol::rg_list("claude")) }
fn pruv_open() -> i64 { guarded(|| sol::rg_open("claude", TARGET)) }

#[derive(Default)]
struct GridHost {
    sessions: Vec<Session>,
    items: Vec<ListItem>,
    temp: Vec<bool>,
    switch: Option<(u64, Vec<Session>)>,
    switched: bool,
    cursor: usize,
    counts: (u64, u64),
    read_turns: usize,
    cancel_after_read: bool,
    read: bool,
    created: Option<(String, i64)>,
    trace: Option<RefCell<Vec<Event>>>,
}

impl GridHost {
    fn new(sessions: Vec<Session>, items: Vec<ListItem>) -> Self {
        Self { sessions, items, read_turns: 3, ..Self::default() }
    }

    fn reset_pruv(&self) {
        rg_reset(pack_sessions(&self.sessions), pack_items(&self.items), self.temp.clone(), self.read_turns as i64);
        if let Some((after, sessions)) = &self.switch {
            set_rg_switch(*after, pack_sessions(sessions));
        }
        set_rg_cancelled(self.cancel_after_read);
    }

    fn clear(&mut self) {
        self.counts = (0, 0);
        self.read = false;
        self.created = None;
        if let Some(trace) = &mut self.trace {
            trace.get_mut().clear();
        }
    }

    fn log(&self, tag: i64, a: i64) {
        if let Some(trace) = &self.trace {
            trace.borrow_mut().push((tag, a, 0));
        }
    }

    fn observe(&mut self) {
        self.counts.0 += 1;
        self.switched = self.switch.as_ref().is_some_and(|(after, _)| self.counts.0 > *after);
        self.cursor = 0;
        self.log(T_SL, self.counts.0 as i64);
    }

    fn observed(&self) -> &[Session] {
        if self.switched { &self.switch.as_ref().unwrap().1 } else { &self.sessions }
    }

    fn temp_at(&mut self, index: usize) -> bool {
        self.counts.1 += 1;
        self.log(T_RGTEMP, index as i64);
        self.temp.get(index).copied().unwrap_or(false)
    }

    fn bound(&mut self, engine: Engine, thread: &str) -> Option<String> {
        self.observe();
        for index in 0..self.observed().len() {
            if self.observed()[index].temporary || self.temp_at(index) {
                continue;
            }
            let session = &self.observed()[index];
            if session_threads(session, engine, thread) {
                return (!session.id.is_empty()).then(|| session.id.clone());
            }
        }
        None
    }
}

impl Host for GridHost {
    fn import_sources(&mut self) -> Result<Vec<Source>, String> {
        Ok(vec![Source { source: SOURCE.into(), engine: Engine::Claude, account: None,
            kind: SourceKind::Home, account_name: None }])
    }
    fn session_list(&mut self) -> &[Session] {
        self.observe();
        self.observed()
    }
    fn is_temporary(&mut self, id: &str) -> bool {
        // The hand lookup visits each non-flagged session in observation order.
        while self.observed()[self.cursor].temporary {
            self.cursor += 1;
        }
        let index = self.cursor;
        debug_assert_eq!(self.observed()[index].id, id);
        self.cursor += 1;
        self.temp_at(index)
    }
    fn check_folder(&mut self, path: &str) -> Result<String, Option<String>> { Ok(path.into()) }
    fn grok_accounts(&mut self) -> Vec<String> { unreachable!() }
    fn default_account(&self) -> &str { unreachable!() }
    fn from_app_action(&mut self) -> bool { false }
    fn model_for(&mut self, engine: Engine, _: Option<&str>, wanted: Option<&str>) -> String {
        wanted.unwrap_or(engine.name()).into()
    }
    fn create_imported(&mut self, seed: Seed) -> Result<String, String> {
        let turns = seed.turns.len() as i64;
        self.log(T_CREATE, turns);
        self.created = Some((seed.origin.thread, turns));
        Ok(CREATED.into())
    }
    fn now_iso(&mut self) -> String { "2026-01-01T00:00:00Z".into() }
    fn uuid(&mut self) -> String { "turn".into() }
    fn rm(&mut self, _: &str) { unreachable!() }
    fn timer_start(&mut self, _: u32) -> u32 { 1 }
    fn timer_clear(&mut self, _: u32) {}
    fn cancelled(&self) -> Option<String> {
        // The pre-read cancellation check is part of the hand timer wrapper.
        if self.read {
            self.log(T_CANCEL, 0);
        }
        (self.read && self.cancel_after_read).then(|| "cancelled".into())
    }
    fn codex_run_deps(&mut self, _: &Source) -> Result<(), ImportError> { unreachable!() }
    fn locate_opencode(&mut self) -> Result<Option<String>, ImportError> { unreachable!() }
    fn temp_root(&mut self) -> Result<String, ImportError> { unreachable!() }
    fn list_external(&mut self, _: &Source, _: &str, _: &ReadOptions) -> Result<Vec<ListItem>, ImportError> {
        self.log(T_ITEMS, 0);
        Ok(self.items.clone())
    }
    fn read_external(&mut self, _: &Source, _: &str, _: &str, _: &ReadOptions) -> Result<External, ImportError> {
        self.log(T_READ, 0);
        self.read = true;
        Ok(External { title: "Imported".into(), model: None, turns: vec![ExternalTurn {
            prompt: "prompt".into(), reply: "reply".into(), created_at: None,
            completed_at: Field::Missing, tools: 0.0, clipped: false,
        }; self.read_turns], omitted_before: false, last_plan: None })
    }
    fn grok_home_for(&mut self, _: Option<&str>) -> Result<GrokTarget, Option<String>> { unreachable!() }
    fn copy_grok_session(&mut self, _: &Source, _: &str, _: &str, _: &str,
        _: &mut dyn FnMut(&mut Self, &str) -> Result<(), ImportError>) -> Result<String, ImportError> {
        unreachable!()
    }
    fn grok_load_check(&mut self, _: &GrokTarget, _: &str, _: &str, _: &str) -> Result<(), ImportError> {
        unreachable!()
    }
}

fn pack_sessions(sessions: &[Session]) -> String {
    let mut packed = String::from("0");
    for session in sessions {
        let cli_engine = session.terminal_cli.as_ref().is_some_and(|(engine, _)| engine.is_some());
        let bits = u8::from(session.temporary) | (u8::from(session.external.is_some()) << 1)
            | (u8::from(session.terminal_cli.is_some()) << 2) | (u8::from(cli_engine) << 3);
        packed.push(char::from(64 + bits));
        pack_field(&mut packed, &session.id);
        pack_field(&mut packed, &session.title);
        if let Some(ext) = &session.external {
            pack_field(&mut packed, &ext.engine);
            pack_field(&mut packed, &ext.thread);
            pack_count(&mut packed, ext.forks.len());
            for fork in &ext.forks {
                pack_field(&mut packed, fork);
            }
        }
        if let Some((engine, thread)) = &session.terminal_cli {
            if let Some(engine) = engine {
                pack_field(&mut packed, engine);
            }
            pack_field(&mut packed, thread);
        }
        pack_count(&mut packed, session.engine_threads.len());
        for (slot, thread) in &session.engine_threads {
            pack_field(&mut packed, slot);
            pack_field(&mut packed, thread);
        }
    }
    packed
}

fn pack_items(items: &[ListItem]) -> String {
    let mut packed = String::from("0");
    for item in items {
        pack_field(&mut packed, &item.thread);
        pack_field(&mut packed, &item.title);
    }
    packed
}

fn item(thread: impl Into<String>, title: impl Into<String>) -> ListItem {
    ListItem { thread: thread.into(), title: title.into(), updated_at: Field::Missing,
        size: Field::Missing, messages: Field::Missing, path: Field::Missing, maybe_open: false }
}

fn session(id: impl Into<String>, title: impl Into<String>) -> Session {
    Session { id: id.into(), title: title.into(), temporary: false, external: None,
        terminal_cli: None, engine_threads: Vec::new() }
}

fn request() -> OpenRequest {
    OpenRequest { source: SOURCE.into(), folder: FOLDER.into(), thread: TARGET.into(),
        mode: Mode::Fork, target_account: None }
}

// The control uses the same handler steps for the grid's Claude Home / fork inputs.
fn list_nocopy(host: &mut GridHost) -> Result<Vec<ListRow>, Fail> {
    let sources = host.import_sources().map_err(Fail::Passthrough)?;
    let src = source_of(&sources, SOURCE).unwrap();
    let folder = folder_of(host, FOLDER)?;
    let items = timed(host, false, |h| {
        let opts = read_options(h, src, false)?;
        h.list_external(src, &folder, &opts)
    }).map_err(import_failure)?;
    let mut titles = HashMap::new();
    for session in host.session_list() {
        titles.insert(session.id.clone(), session.title.clone());
    }
    let mut rows = Vec::with_capacity(items.len());
    for item in items {
        let bound = host.bound(src.engine, &item.thread);
        let title = bound.as_ref().and_then(|id| titles.get(id))
            .filter(|title| !title.is_empty()).unwrap_or(&item.title).clone();
        rows.push(ListRow {
            thread: item.thread, title, updated_at: item.updated_at, size: item.size,
            messages: item.messages, path: item.path, maybe_open: item.maybe_open,
            bound_session_id: bound,
        });
    }
    Ok(rows)
}

fn open_nocopy(host: &mut GridHost, opening: &mut HashSet<String>, req: &OpenRequest) -> Result<OpenResult, Fail> {
    let sources = host.import_sources().map_err(Fail::Passthrough)?;
    let src = source_of(&sources, &req.source).unwrap();
    let folder = folder_of(host, &req.folder)?;
    let engine = src.engine;
    let account = account_of(host, src, req.target_account.as_deref())?;
    if let Some(session_id) = host.bound(engine, &req.thread) {
        return Ok(OpenResult { session_id, existing: true });
    }
    let key = format!("{}:{}", engine.name(), req.thread);
    assert!(opening.insert(key.clone()), "grid control already opening");
    let result = (|| {
        let external = timed(host, true, |h| {
            let opts = read_options(h, src, true)?;
            h.read_external(src, &folder, &req.thread, &opts)
        })?;
        if host.cancelled().is_some() {
            return Err(ImportError::Abort);
        }
        if let Some(session_id) = host.bound(engine, &req.thread) {
            return Ok(OpenResult { session_id, existing: true });
        }
        let now = host.now_iso();
        let turns = turns_of(host, engine, &external, &now);
        let bound = EngineThread {
            thread_id: req.thread.clone(),
            last_turn_id: turns.last().filter(|t| !t.id.is_empty()).map(|t| t.id.clone()),
            fork: req.mode == Mode::Fork,
        };
        let origin = ExternalThread {
            engine, source: src.source.clone(), thread: req.thread.clone(), mode: req.mode,
            imported_at: now, omitted_before: external.omitted_before,
            last_plan: external.last_plan.clone().filter(|p| !matches!(p, Plan::Text(t) if t.is_empty())),
        };
        let title = title_of(&external, &folder);
        let model = host.model_for(engine, account.as_deref(), external.model.as_deref());
        let session_id = host.create_imported(Seed {
            engine, account: account.filter(|a| !a.is_empty()), model, folder: folder.to_string(),
            title, turns, origin, slot: thread_slot(engine, src.account.as_deref()), bound,
        }).map_err(ImportError::Other)?;
        Ok(OpenResult { session_id, existing: false })
    })();
    opening.remove(&key);
    result.map_err(|error| host.cancelled().map(Fail::Cancel).unwrap_or_else(|| import_failure(error)))
}

#[derive(Clone, Copy)]
struct GridCell {
    op: &'static str,
    s: usize,
    m: usize,
    placement: &'static str,
}

impl GridCell {
    fn name(self) -> String {
        format!("{} S={} M={} {}", self.op, self.s, self.m, self.placement)
    }

    fn world(self) -> GridHost {
        let items = (0..self.m).map(|j| item(format!("t{j}"), format!("Item {j}"))).collect();
        let n = if self.op == "list" { self.m } else { 1 };
        let start = match self.placement {
            "first" => 0,
            "last" => self.s - n,
            "none" => self.s,
            _ => unreachable!(),
        };
        let sessions = (0..self.s).map(|i| {
            let matched = if (start..start + n).contains(&i) {
                if self.op == "list" { format!("t{}", i - start) } else { TARGET.into() }
            } else {
                format!("e{i}_3")
            };
            Session {
                id: format!("s{i}"), title: format!("T{i}"), temporary: false,
                external: Some(ExternalBound {
                    engine: "claude".into(), thread: format!("o{i}"),
                    forks: (0..3).map(|f| format!("f{i}_{f}")).collect(),
                }),
                terminal_cli: Some((None, format!("c{i}"))),
                engine_threads: vec![
                    ("claude:w1".into(), format!("e{i}_1")),
                    ("claude:w2".into(), format!("e{i}_2")),
                    ("claude:w3".into(), matched),
                ],
            }
        }).collect();
        GridHost::new(sessions, items)
    }
}

fn cells() -> Vec<GridCell> {
    let mut cells = Vec::new();
    for s in [1000, 5000, 20000] {
        for (op, m) in [("list", 1), ("list", 10), ("list", 50), ("open", 0)] {
            for placement in ["none", "first", "last"] {
                cells.push(GridCell { op, s, m, placement });
            }
        }
    }
    cells
}

fn check_list(host: &mut GridHost) -> Vec<Row> {
    host.clear();
    host.reset_pruv();
    let hand = Requests::default().list(host, SOURCE, FOLDER).unwrap();
    let counts = host.counts;
    let code = pruv_list();
    let actual = rg_take();
    let rows: Vec<Row> = hand.iter().enumerate().map(|(j, r)| {
        (j as i64, r.bound_session_id.clone(), r.title.clone())
    }).collect();
    assert_eq!(code, rows.len() as i64);
    assert_eq!(actual.rows, rows);
    assert_eq!((actual.sl_calls, actual.temp_calls), counts);
    if host.trace.is_none() {
        host.clear();
        assert_eq!(list_nocopy(host).unwrap(), hand);
        assert_eq!(host.counts, counts);
    }
    rows
}

fn check_open(host: &mut GridHost) -> i64 {
    host.clear();
    host.reset_pruv();
    let hand = Requests::default().open(host, &request());
    let created = host.created.clone();
    let counts = host.counts;
    let code = pruv_open();
    let actual = rg_take();
    match &hand {
        Ok(result) if result.existing => {
            assert_eq!(code, 1);
            assert_eq!(actual.existing.as_ref(), Some(&result.session_id));
            assert_eq!(created, None);
        }
        Ok(result) => {
            assert_eq!(code, 0);
            assert_eq!(result.session_id, CREATED);
            assert_eq!(created, Some((TARGET.into(), host.read_turns as i64)));
            assert_eq!(actual.existing, None);
        }
        Err(Fail::Cancel(reason)) => {
            assert_eq!(reason, "cancelled");
            assert_eq!(code, -4);
            assert_eq!(actual.existing, None);
            assert_eq!(created, None);
        }
        Err(error) => panic!("unexpected hand open error: {error:?}"),
    }
    assert_eq!(actual.created, created);
    assert_eq!((actual.sl_calls, actual.temp_calls), counts);
    if host.trace.is_none() {
        host.clear();
        let control = open_nocopy(host, &mut HashSet::new(), &request());
        match (&hand, control) {
            (Ok(expected), Ok(got)) => assert_eq!(&got, expected),
            (Err(Fail::Cancel(expected)), Err(Fail::Cancel(got))) => assert_eq!(&got, expected),
            (_, got) => panic!("hand {hand:?}, nocopy {got:?}"),
        }
        assert_eq!(host.created, created);
        assert_eq!(host.counts, counts);
    } else if let Ok(result) = &hand {
        if result.existing {
            // This effect is visible to the hand caller through OpenResult.
            host.log(T_EXIST, 0);
        }
    }
    code
}

struct Random(u64);
impl Random {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }

    fn thread(&mut self) -> &'static str {
        [TARGET, "orig", "fork", "cli", "slot", "missing"][self.below(6)]
    }

    fn session(&mut self, i: usize) -> Session {
        let id = ["", "a", "b"][self.below(3)];
        let title = if self.below(4) == 0 { String::new() } else { format!("Title {i}") };
        let mut session = session(id, title);
        session.temporary = self.below(4) == 0;
        session.external = match self.below(3) {
            0 => None,
            choice => Some(ExternalBound {
                engine: if choice == 1 { "claude" } else { "codex" }.into(),
                thread: self.thread().into(),
                forks: (0..self.below(4)).map(|_| self.thread().into()).collect(),
            }),
        };
        session.terminal_cli = match self.below(4) {
            0 => None,
            choice => Some((match choice {
                1 => None, 2 => Some("claude".into()), _ => Some("codex".into()),
            }, self.thread().into())),
        };
        session.engine_threads = ["claude", "claude:w1", "codexx:w"].into_iter()
            .filter_map(|slot| (self.below(2) == 0).then(|| (slot.into(), self.thread().into())))
            .collect();
        session
    }
}

fn binding(id: &str, title: &str, thread: &str) -> Session {
    let mut session = session(id, title);
    session.engine_threads.push(("claude:w1".into(), thread.into()));
    session
}

fn random_gates() {
    let mut rng = Random(0x10_48_cafe);
    for _ in 0..2000 {
        let sessions: Vec<_> = (0..rng.below(7)).map(|i| rng.session(i)).collect();
        let items = (0..rng.below(6)).map(|j| item(rng.thread(), format!("Item {j}"))).collect();
        let mut host = GridHost::new(sessions, items);
        host.temp = (0..host.sessions.len()).map(|_| rng.below(4) == 0).collect();
        host.read_turns = rng.below(6);
        check_list(&mut host);
        check_open(&mut host);

        // Use an unbound first observation for both post-read paths.
        host.sessions.retain(|s| !session_threads(s, Engine::Claude, TARGET));
        host.cancel_after_read = true;
        assert_eq!(check_open(&mut host), -4);
        host.cancel_after_read = false;
        host.switch = Some((1, vec![binding("raced", "Raced", TARGET)]));
        host.temp.resize(1, false);
        host.temp[0] = false;
        assert_eq!(check_open(&mut host), 1);
    }
    println!("GATE differential 2000/2000 worlds (2000 list, 6000 open)");
}


fn fresh_gate() {
    let items = (0..3).map(|j| item(format!("t{j}"), format!("Item {j}"))).collect();
    let mut host = GridHost::new(vec![session("s0", "Original title")], items);
    host.switch = Some((2, vec![binding("s0", "New title", "t1")]));
    let rows = check_list(&mut host);
    assert_eq!(rows, vec![
        (0, None, "Item 0".into()),
        (1, Some("s0".into()), "Original title".into()),
        (2, None, "Item 2".into()),
    ]);
    println!("GATE fresh_query 1/1");
}

fn order_gate() {
    let mut flagged = binding("flagged", "Flagged", "t0");
    flagged.temporary = true;
    let mut host = GridHost::new(
        vec![flagged, session("s1", "One"), binding("s2", "Two", "t0")],
        vec![item("t0", "First"), item("t1", "Second")],
    );
    host.trace = Some(RefCell::new(Vec::new()));
    set_trace(true);
    take_trace();
    let rows = check_list(&mut host);
    let expected = vec![
        (T_ITEMS, 0, 0), (T_SL, 1, 0),
        (T_SL, 2, 0), (T_RGTEMP, 1, 0), (T_RGTEMP, 2, 0), (T_ROW, 0, 1),
        (T_SL, 3, 0), (T_RGTEMP, 1, 0), (T_RGTEMP, 2, 0), (T_ROW, 1, 0),
    ];
    assert_eq!(take_trace(), expected);
    // Hand emits rows through its return value. Associate each with its bound observation.
    let trace = host.trace.as_mut().unwrap().get_mut();
    for (j, bound, _) in rows.into_iter().rev() {
        let end = trace.iter().position(|event| *event == (T_SL, j + 3, 0)).unwrap_or(trace.len());
        trace.insert(end, (T_ROW, j, i64::from(bound.is_some())));
    }
    assert_eq!(*trace, expected);

    assert_eq!(check_open(&mut host), 0);
    let expected = vec![
        (T_SL, 1, 0), (T_RGTEMP, 1, 0), (T_RGTEMP, 2, 0), (T_READ, 0, 0), (T_CANCEL, 0, 0),
        (T_SL, 2, 0), (T_RGTEMP, 1, 0), (T_RGTEMP, 2, 0), (T_CREATE, 3, 0),
    ];
    assert_eq!(take_trace(), expected);
    assert_eq!(*host.trace.as_ref().unwrap().borrow(), expected);
    set_trace(false);
    println!("GATE order 2/2");
}

fn failure_gate() {
    let items = pack_items(&[item("t0", "Item")]);
    rg_reset("1".into(), items.clone(), vec![], 3);
    assert_eq!(pruv_list(), -2);
    assert_eq!(pruv_open(), -2);
    rg_reset("0".into(), "1".into(), vec![], 3);
    assert_eq!(pruv_list(), -3);
    rg_reset("0@\0\x03x".into(), items, vec![], 3);
    assert_eq!(pruv_list(), -2);
    assert_eq!(pruv_open(), -2);
    println!("GATE host_failure 5/5");
}

fn reentry_gate() {
    set_trace(true);
    take_trace();
    let host = GridHost::new(vec![binding("s0", "Title", TARGET)], vec![item(TARGET, "Item")]);
    for call in [pruv_list as fn() -> i64, pruv_open] {
        host.reset_pruv();
        set_rg_reenter(Some(call));
        assert_eq!(call(), 1);
        let reentries: Vec<_> = take_trace().into_iter().filter(|event| event.0 == T_REENTRY).collect();
        assert_eq!(reentries, vec![(T_REENTRY, 9, 0)]);
        let world = rg_take();
        if world.rows.is_empty() {
            assert_eq!(world.existing.as_deref(), Some("s0"));
        } else {
            assert_eq!(world.rows, vec![(0, Some("s0".into()), "Title".into())]);
        }
    }
    set_trace(false);
    println!("GATE reentry 2/2");
}

fn gates() {
    set_trace(false);
    let cells = cells();
    for cell in &cells {
        let mut host = cell.world();
        if cell.op == "list" {
            check_list(&mut host);
            if cell.placement == "none" {
                assert_eq!(host.counts, ((cell.m + 1) as u64, (cell.m * cell.s) as u64));
            }
        } else {
            check_open(&mut host);
        }
    }
    println!("GATE row_and_open_equality {0}/{0} cells", cells.len());
    println!("GATE call_counts {0}/{0} cells", cells.len());
    random_gates();
    fresh_gate();
    order_gate();
    failure_gate();
    reentry_gate();
}

fn grid(rounds: usize, path: Option<&str>, max_s: usize) {
    assert!(rounds > 0);
    set_trace(false);
    let mut perf_cells = Vec::new();
    for cell in cells().into_iter().filter(|cell| cell.s <= max_s) {
        let mut host = cell.world();
        host.reset_pruv();
        let mut prefix_host = GridHost::new(Vec::new(), Vec::new());
        let req = request();
        let mut hand = Requests::default();
        let mut prefix = Requests::default();
        let mut opening = HashSet::new();
        let mut samples = [(); 4].map(|_| Vec::with_capacity(rounds));
        let mut pruv_counts = (0, 0);
        let mut hand_counts = (0, 0);
        for round in 0..=rounds {
            for offset in 0..4 {
                let lane = (round + offset) % 4;
                match lane {
                    0 | 1 => host.clear(),
                    2 => rg_clear_effects(),
                    3 => prefix_host.clear(),
                    _ => unreachable!(),
                }
                let start = Instant::now();
                match (lane, cell.op) {
                    (0, "list") => drop(black_box(hand.list(&mut host, SOURCE, FOLDER).unwrap())),
                    (0, _) => drop(black_box(hand.open(&mut host, &req).unwrap())),
                    (1, "list") => drop(black_box(list_nocopy(&mut host).unwrap())),
                    (1, _) => drop(black_box(open_nocopy(&mut host, &mut opening, &req).unwrap())),
                    (2, "list") => { black_box(pruv_list()); }
                    (2, _) => { black_box(pruv_open()); }
                    (3, "list") => drop(black_box(prefix.list(&mut prefix_host, SOURCE, FOLDER).unwrap())),
                    (3, _) => drop(black_box(prefix.open(&mut prefix_host, &req).unwrap())),
                    _ => unreachable!(),
                }
                let ns = start.elapsed().as_nanos();
                if round > 0 {
                    samples[lane].push(ns);
                } else if lane == 0 {
                    hand_counts = host.counts;
                } else if lane == 2 {
                    pruv_counts = rg_counts();
                }
            }
        }
        let [hand_ns, nocopy_ns, pruv_ns, prefix_ns] = samples.map(|mut lane| {
            lane.sort_unstable();
            lane[rounds / 2]
        });
        let pruv_adj_ns = pruv_ns + prefix_ns;
        let (sl_calls, temp_calls) = pruv_counts;
        let (hand_sl_calls, hand_temp_calls) = hand_counts;
        let name = cell.name();
        let GridCell { op, s, m, placement } = cell;
        let extra = format!(concat!(
            r#""nocopy_ns":{0},"pruv_ns":{1},"prefix_ns":{2},"sl_calls":{3},"temp_calls":{4},"#,
            r#""hand_sl_calls":{5},"hand_temp_calls":{6}"#
        ), nocopy_ns, pruv_ns, prefix_ns, sl_calls, temp_calls, hand_sl_calls, hand_temp_calls);
        println!(concat!(
            r#"{{"cell":"{0}","op":"{1}","S":{2},"M":{3},"placement":"{4}","#,
            r#""hand_ns":{5},"pruv_adj_ns":{6},"rounds":{7},{8}}}"#
        ), name, op, s, m, placement, hand_ns, pruv_adj_ns, rounds, extra);
        if path.is_some() {
            let size = if hand_ns < 1_000_000 { "S" } else if s == 20000 { "L" } else { "M" };
            perf_cells.push(format!(concat!(
                r#"{{"name":"{0}","axis":"time","kind":"proportional","unit":"ns","size":"{1}","#,
                r#""pruv":{2},"hand":{3},"runs":{4},"extra":{{{5}}}}}"#
            ), name, size, pruv_adj_ns, hand_ns, rounds, extra));
        }
    }
    if let Some(path) = path {
        std::fs::write(path, format!(
            "{{\"source\":\"bench/l3/hostfact/req_grid.rs\",\"cells\":[\n{}\n]}}\n",
            perf_cells.join(",\n")
        )).unwrap();
    }
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["gates"] => gates(),
        ["grid", rounds, tail @ ..] => {
            let (path, max_s) = match tail {
                [] => (None, usize::MAX),
                ["--max-s", max_s] => (None, max_s.parse().unwrap()),
                [path] => (Some(*path), usize::MAX),
                [path, "--max-s", max_s] => (Some(*path), max_s.parse().unwrap()),
                _ => panic!("usage: grid <rounds> [cells-json-path] [--max-s N]"),
            };
            grid(rounds.parse().unwrap(), path, max_s);
        }
        _ => panic!("usage: gates | grid <rounds> [cells-json-path] [--max-s N]"),
    }
}
