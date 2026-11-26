// bench/l3/req/hand/req.rs
//
// What this is: the "natural safe hand Rust" baseline of the L3-9 `req` fragment (engine-import request handlers:
// sources, list, open), written the way a Rust programmer would write it. Rust-shape reset step 4.4, with
// SYNCHRONOUS host facts per docs/L3_RESTRUCTURE_SPEC.md §3.2 / P10H: no world/packet/seal exchange, no [i64] word
// packets, no step machine. Decisions live in the engine (`Requests` and the free fns); the `Host` trait only offers
// facts and effects. Std only, edition 2021, no unsafe.
//
// Oracle: bench/l3/req/reference/backend-requests-engine-import.ts (R:<line> in the `// TS:` comments),
// docs/ENGINE_IMPORT_REQUEST_SPEC.md §3 and §4.
//
// Check:  rustc --edition 2021 --test hand/req.rs -o /tmp/req_hand && /tmp/req_hand
//
// The Pruv port waits on P10H; every `// Pruv:` line is `TBD` until then.
//
// Modelling choices (things the synchronous shape cannot or need not copy):
// - The AbortSignal is not passed to the work. `timed` starts the 20 s timer and, when an external cancel exists and
//   is already set, ends with `ImportError::Abort` without running the work (TS runs the work with an aborted signal).
//   The timer never forces a failure: a synchronous work that returns Ok makes `timed` return Ok.
// - Grok copy: `Host::copy_grok_session` copies and returns the copied path. The engine then runs
//   `timed(grok_load_check)` on it; on a verify failure the engine rm's the path (TS: copy cleans its own target) and
//   returns `Abort` or `GrokLoadFailed`. `copied` is stored by the handler only after both succeeded.
// - `bound_session_id` takes ONE fresh `session_list()` per call and copies it (`to_vec`), because `is_temporary`
//   needs `&mut host` while the list borrows the host. The copy is one observation, never reused across calls.
// - `title_of` returns `Vec<u16>` (and `Seed.title` is `Vec<u16>`): a cut at unit 80 may keep a lone high surrogate,
//   which a Rust `String` cannot hold. `String::from_utf16_lossy` would replace it, so it is not used.
// - `OpenResult.existing` is a bool: the TS omits `existing` for a fresh session, here that is `existing: false`.
// - The env and the AbortSignal of ImportReadOptions are left out of `ReadOptions`.
// - Test fns and the FakeHost impl carry no `// TS:` line (they have no TS counterpart).

use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------------------------------------------

/// TS: EngineImportEngine (shared-engine-import.ts:2)
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Engine {
    Claude,
    Codex,
    Grok,
    Opencode,
    Qwen,
}

impl Engine {
    /// TS: EngineImportEngine string value (shared-engine-import.ts:2)
    // Pruv: req_open::key_of, req_fill::slot_name
    pub fn name(self) -> &'static str {
        match self {
            Engine::Claude => "claude",
            Engine::Codex => "codex",
            Engine::Grok => "grok",
            Engine::Opencode => "opencode",
            Engine::Qwen => "qwen",
        }
    }
}

/// TS: ImportSource.kind (entry reference; R:126, R:151)
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceKind {
    Home,
    Account,
}

/// TS: ImportSource (L3-8 importSources entry; R:125-126)
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Debug, PartialEq)]
pub struct Source {
    pub source: String,
    pub engine: Engine,
    pub account: Option<String>,
    pub kind: SourceKind,
    pub account_name: Option<String>,
}

/// TS: EngineImportSourcesResult.sources item (shared-engine-import.ts:53)
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Debug, PartialEq)]
pub struct PublicSource {
    pub source: String,
    pub engine: Engine,
    pub account: Option<String>,
    pub label: String,
}

/// TS: ExternalThread (original thread and confirmed forks; shared-engine-import.ts:17-23, bound.ts candidates)
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Debug, PartialEq)]
pub struct ExternalBound {
    pub engine: String,
    pub thread: String,
    pub forks: Vec<String>,
}

/// TS: Session (the head fields the bound search reads; R:96-102)
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub temporary: bool,
    pub external: Option<ExternalBound>,
    /// cliSession: (engine, thread); engine `None` means "codex".
    pub terminal_cli: Option<(Option<String>, String)>,
    /// engineThreads in Object.entries order: (slot key, threadId).
    pub engine_threads: Vec<(String, String)>,
}

/// TS: ExternalThread.mode (shared-engine-import.ts:21)
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Same,
    Fork,
}

/// TS: an optional field that keeps `undefined` (omitted) apart from an explicit `null` (R:139-141, R:178).
#[derive(Clone, Debug, PartialEq)]
pub enum Field<T> {
    Missing,
    Null,
    Val(T),
}

/// TS: ListItem.updatedAt (shared-engine-import.ts:67): a number or an ISO string; 0 is kept.
#[derive(Clone, Debug, PartialEq)]
pub enum Stamp {
    Num(f64),
    Text(String),
}

/// TS: lastPlan (shared-engine-import.ts:11-14): a string or a plan object; the object is kept as its raw JSON text,
/// never stringified by the handler. Included when truthy (an object always is, a string when non-empty).
#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    Text(String),
    Object(String),
}

/// TS: item of listExternal (L3-8; R:132-141)
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Debug, PartialEq)]
pub struct ListItem {
    pub thread: String,
    pub title: String,
    pub updated_at: Field<Stamp>,
    pub size: Field<f64>,
    pub messages: Field<f64>,
    pub path: Field<String>,
    pub maybe_open: bool,
}

/// TS: EngineImportListResult.items item (R:139-141)
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Debug, PartialEq)]
pub struct ListRow {
    pub thread: String,
    pub title: String,
    pub updated_at: Field<Stamp>,
    pub size: Field<f64>,
    pub messages: Field<f64>,
    pub path: Field<String>,
    pub maybe_open: bool,
    pub bound_session_id: Option<String>,
}

/// TS: ExternalTurn of readExternal (L3-8; R:176-180)
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Debug, PartialEq)]
pub struct ExternalTurn {
    pub prompt: String,
    pub reply: String,
    pub created_at: Option<String>,
    pub completed_at: Field<String>,
    pub tools: f64,
    pub clipped: bool,
}

/// TS: readExternal result (L3-8; R:162, R:185-187)
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Debug, PartialEq)]
pub struct External {
    pub title: String,
    pub model: Option<String>,
    pub turns: Vec<ExternalTurn>,
    pub omitted_before: bool,
    pub last_plan: Option<Plan>,
}

/// TS: Turn (imported turn; R:176-180). status is always "completed", progress "", usage null (not stored).
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    pub id: String,
    pub prompt: String,
    pub reply: String,
    pub created_at: String,
    pub completed_at: Field<String>,
    pub imported_engine: Engine,
    pub imported_tools: Option<f64>,
    pub imported_clipped: bool,
}

/// TS: EngineThread (shared-engine-thread.ts:6-22, only the fields this handler sets)
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Debug, PartialEq)]
pub struct EngineThread {
    pub thread_id: String,
    pub last_turn_id: Option<String>,
    pub fork: bool,
}

/// TS: ExternalThread origin (shared-engine-import.ts:17-28, forks are never created here; R:183-185)
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Debug, PartialEq)]
pub struct ExternalThread {
    pub engine: Engine,
    pub source: String,
    pub thread: String,
    pub mode: Mode,
    pub imported_at: String,
    pub omitted_before: bool,
    pub last_plan: Option<Plan>,
}

/// TS: EngineImportSeed with `fill` as data (R:24-30, R:188-194). The host applies it: title, turns, externalThread,
/// and engineThreads = {...old, [slot]: bound}.
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Debug, PartialEq)]
pub struct Seed {
    pub engine: Engine,
    pub account: Option<String>,
    pub model: String,
    pub folder: String,
    pub title: Vec<u16>,
    pub turns: Vec<Turn>,
    pub origin: ExternalThread,
    pub slot: String,
    pub bound: EngineThread,
}

/// TS: EngineImportOpenRequest (shared-engine-import.ts:77-83)
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Debug, PartialEq)]
pub struct OpenRequest {
    pub source: String,
    pub folder: String,
    pub thread: String,
    pub mode: Mode,
    pub target_account: Option<String>,
}

/// TS: EngineImportOpenResult (shared-engine-import.ts:86-89)
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Debug, PartialEq)]
pub struct OpenResult {
    pub session_id: String,
    pub existing: bool,
}

/// TS: ImportReadOptions (R:113-116), without signal and env.
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ReadOptions {
    /// makeCodexRunDeps was prepared (Codex only).
    pub codex: bool,
    pub opencode: Option<OpencodeOptions>,
}

/// TS: ImportReadOptions.opencode (R:115)
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Debug, PartialEq)]
pub struct OpencodeOptions {
    pub binary: String,
    pub temp_root: String,
}

/// TS: grokHomeFor result (R:51); env is not modelled.
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Debug, PartialEq)]
pub struct GrokTarget {
    pub home: String,
    pub binary: String,
}

// ---------------------------------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------------------------------

/// TS: EngineImportError codes plus AbortError and any other thrown value (R:54-69)
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Debug, PartialEq)]
pub enum ImportError {
    NotFound,
    CodexUnsupported,
    GrokExists,
    GrokLoadFailed,
    Timeout,
    EngineMissing(Option<String>),
    ReadFailed,
    /// error.name === 'AbortError'
    Abort,
    /// Anything else: rethrown as is.
    Other(String),
}

/// TS: WireError(wire(key), message) (R:11)
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Debug, PartialEq)]
pub struct WireError {
    pub key: &'static str,
    pub message: String,
}

/// TS: what the handler throws (R:54-69, R:199)
// Pruv: packet / record words (REQ-W)
#[derive(Clone, Debug, PartialEq)]
pub enum Fail {
    Wire(WireError),
    /// The original `cancel.reason`.
    Cancel(String),
    /// An unmapped thrown value, rethrown as is.
    Passthrough(String),
}

// ---------------------------------------------------------------------------------------------------------------
// Host
// ---------------------------------------------------------------------------------------------------------------

/// TS: EngineImportDeps and the L3-8 calls it reaches (R:33-52). Synchronous facts and effects only.
// Pruv: child ops 1/3/4/7/8 and host calls (RESHAPE.md §1)
pub trait Host {
    /// TS: importSources(deps) (L3-8, reused from entry; R:80-84, R:125). Err = a throw while building sources.
    fn import_sources(&mut self) -> Result<Vec<Source>, String>;
    /// TS: deps.sessionList() (R:37). A FRESH observation each call; never cache across items.
    fn session_list(&mut self) -> &[Session];
    /// TS: deps.isTemporary(id) (R:39)
    fn is_temporary(&mut self, id: &str) -> bool;
    /// TS: deps.checkFolder(path) (R:41). Err(Some(msg)) = an Error throw, Err(None) = a non-Error throw.
    fn check_folder(&mut self, path: &str) -> Result<String, Option<String>>;
    /// TS: deps.grokAccounts() ids (R:35)
    fn grok_accounts(&mut self) -> Vec<String>;
    /// TS: DEFAULT_ACCOUNT (R:18, an immutable host binding)
    fn default_account(&self) -> &str;
    /// TS: deps.fromAppAction() (R:47)
    fn from_app_action(&mut self) -> bool;
    /// TS: deps.modelFor(engine, account, wanted) (R:43)
    fn model_for(&mut self, engine: Engine, account: Option<&str>, wanted: Option<&str>) -> String;
    /// TS: deps.createImported(seed) (R:45). Err = a throw (before or after fill).
    fn create_imported(&mut self, seed: Seed) -> Result<String, String>;
    /// TS: new Date().toISOString() (R:175)
    fn now_iso(&mut self) -> String;
    /// TS: randomUUID() (R:177)
    fn uuid(&mut self) -> String;
    /// TS: rm(path, {recursive:true, force:true}) with the rejection swallowed (R:172, R:198)
    fn rm(&mut self, path: &str);
    /// TS: setTimeout(abort, IMPORT_LIMITS.rpcMs) (R:76); returns a timer handle.
    fn timer_start(&mut self, ms: u32) -> u32;
    /// TS: clearTimeout (R:77)
    fn timer_clear(&mut self, t: u32);
    /// TS: cancel?.aborted ? cancel.reason (R:169, R:199). Some(reason) when the external cancel is set.
    fn cancelled(&self) -> Option<String>;
    /// TS: makeCodexRunDeps(source, deps.dataDir) (R:115)
    fn codex_run_deps(&mut self, src: &Source) -> Result<(), ImportError>;
    /// TS: locateAcp('opencode').binary (R:114). Ok(None) = binary not found; Err = the call failed.
    fn locate_opencode(&mut self) -> Result<Option<String>, ImportError>;
    /// TS: deps.tempRoot() (R:49)
    fn temp_root(&mut self) -> Result<String, ImportError>;
    /// TS: listExternal(source, folder, options) (L3-8; R:132)
    fn list_external(&mut self, src: &Source, folder: &str, opts: &ReadOptions) -> Result<Vec<ListItem>, ImportError>;
    /// TS: readExternal(source, folder, thread, options) (L3-8; R:162)
    fn read_external(&mut self, src: &Source, folder: &str, thread: &str, opts: &ReadOptions) -> Result<External, ImportError>;
    /// TS: deps.grokHomeFor(account) (R:51, R:164). Err(msg) = a rejection (message if Error).
    fn grok_home_for(&mut self, account: Option<&str>) -> Result<GrokTarget, Option<String>>;
    /// TS: copyGrokSession(source, folder, thread, home, verify, cancel) (R:166). The copy side decides whether and with
    /// which home it calls `verify`, maps a verify failure and removes its own partial copy; Ok = the copied path.
    fn copy_grok_session(
        &mut self,
        src: &Source,
        folder: &str,
        thread: &str,
        home: &str,
        verify: &mut dyn FnMut(&mut Self, &str) -> Result<(), ImportError>,
    ) -> Result<String, ImportError>
    where
        Self: Sized;
    /// TS: grokLoadCheck(check, home, folder, thread, signal) (R:166)
    fn grok_load_check(&mut self, target: &GrokTarget, home: &str, folder: &str, thread: &str) -> Result<(), ImportError>;
}

// ---------------------------------------------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------------------------------------------

/// TS: IMPORT_LIMITS.rpcMs (R:76)
const RPC_MS: u32 = 20_000;

/// TS: engineImportHandlers closure state `opening` (R:121)
// Pruv: req_world (open registry; REQ-W)
#[derive(Debug, Default)]
pub struct Requests {
    opening: HashSet<String>,
}

fn wire_fail(key: &'static str, message: &str) -> Fail {
    Fail::Wire(WireError { key, message: message.to_string() })
}

fn source_unknown() -> Fail {
    wire_fail("wireWork.engineImport.sourceUnknown", "고를 수 없는 출처야.")
}

impl Requests {
    /// TS: 'engine-import-sources' (R:123-127)
    // Pruv: root::after_op1, req_sources::tabs
    pub fn sources<H: Host>(&self, host: &mut H) -> Result<Vec<PublicSource>, Fail> {
        let list = host.import_sources().map_err(Fail::Passthrough)?;
        Ok(list
            .iter()
            .map(|entry| PublicSource {
                source: entry.source.clone(),
                engine: entry.engine,
                account: match &entry.account {
                    Some(a) if !a.is_empty() => Some(a.clone()),
                    _ => None,
                },
                label: if entry.kind == SourceKind::Account { entry.account_name.clone().unwrap_or_default() } else { String::new() },
            })
            .collect())
    }

    /// TS: 'engine-import-list' (R:128-144)
    // Pruv: root::read_done, root::list_step, root::list_done, req_list::title_at, req_list::fin_tabs
    pub fn list<H: Host>(&self, host: &mut H, source: &str, folder: &str) -> Result<Vec<ListRow>, Fail> {
        let sources = host.import_sources().map_err(Fail::Passthrough)?;
        let src = match source_of(&sources, source) {
            Some(s) => s,
            None => return Err(source_unknown()),
        };
        let folder = folder_of(host, folder)?;
        let items = timed(host, false, |h: &mut H| {
            let opts = read_options(h, src, false)?;
            h.list_external(src, &folder, &opts)
        })
        .map_err(import_failure)?;
        // One fresh observation after the read; last write wins per id, temporary sessions are not excluded.
        let mut titles: HashMap<String, String> = HashMap::new();
        for s in host.session_list() {
            titles.insert(s.id.clone(), s.title.clone());
        }
        let mut rows = Vec::with_capacity(items.len());
        for item in items {
            let bound = bound_session_id(host, src.engine, &item.thread);
            let title = match &bound {
                Some(id) => match titles.get(id) {
                    Some(t) if !t.is_empty() => t.clone(),
                    _ => item.title.clone(),
                },
                None => item.title.clone(),
            };
            rows.push(ListRow {
                thread: item.thread,
                title,
                updated_at: item.updated_at,
                size: item.size,
                messages: item.messages,
                path: item.path,
                maybe_open: item.maybe_open,
                bound_session_id: bound,
            });
        }
        Ok(rows)
    }

    /// TS: 'engine-import-open' (R:145-202)
    // Pruv: root::open_pre, root::open_acct, root::open_bound, root::open_cont
    pub fn open<H: Host>(&mut self, host: &mut H, req: &OpenRequest) -> Result<OpenResult, Fail> {
        let sources = host.import_sources().map_err(Fail::Passthrough)?;
        let src = match source_of(&sources, &req.source) {
            Some(s) => s,
            None => return Err(source_unknown()),
        };
        let folder = folder_of(host, &req.folder)?;
        let engine = src.engine;
        if req.mode == Mode::Same {
            if engine == Engine::Grok || src.kind != SourceKind::Home {
                return Err(wire_fail(
                    "wireWork.engineImport.sameRefused",
                    "원래 대화에 그대로 이어 쓰기는 기본 Claude·Codex·OpenCode·Qwen 에서만 돼.",
                ));
            }
            if host.from_app_action() {
                return Err(wire_fail("wireWork.engineImport.sameByAi", "그대로 이어 쓰기는 사람이 확인해야 해서 AI 는 쓸 수 없어."));
            }
        }
        let account = account_of(host, src, req.target_account.as_deref())?;
        if let Some(known) = bound_session_id(host, engine, &req.thread) {
            return Ok(OpenResult { session_id: known, existing: true });
        }
        let key = format!("{}:{}", engine.name(), req.thread);
        if !self.opening.insert(key.clone()) {
            return Err(wire_fail("wireWork.engineImport.busy", "같은 터미널 대화를 이미 여는 중이야."));
        }
        // This call inserted the key: remove it on every exit below.
        let result = open_body(host, src, &folder, req, account.as_deref());
        self.opening.remove(&key);
        result
    }
}

/// TS: open try/catch/finally body after `opening.add` (R:160-201). The caller removes the key.
// Pruv: root::open_fail, root::cancel_done, root::done_ev
fn open_body<H: Host>(host: &mut H, src: &Source, folder: &str, req: &OpenRequest, account: Option<&str>) -> Result<OpenResult, Fail> {
    let mut copied: Option<String> = None;
    match open_steps(host, src, folder, req, account, &mut copied) {
        Ok(r) => Ok(r),
        Err(e) => {
            if let Some(path) = copied.take().filter(|p| !p.is_empty()) {
                host.rm(&path);
            }
            // Cancel is checked after cleanup and its original reason wins over any error.
            if let Some(reason) = host.cancelled() {
                return Err(Fail::Cancel(reason));
            }
            Err(import_failure(e))
        }
    }
}

/// TS: open try block (R:162-196). A cancel check that fires returns `Abort`; `open_body` turns it into the reason.
// Pruv: root::read4_done, root::open_step, root::open_race, root::open_recipe, root::open_done
fn open_steps<H: Host>(
    host: &mut H,
    src: &Source,
    folder: &str,
    req: &OpenRequest,
    account: Option<&str>,
    copied: &mut Option<String>,
) -> Result<OpenResult, ImportError> {
    let engine = src.engine;
    let external = timed(host, true, |h: &mut H| {
        let opts = read_options(h, src, true)?;
        h.read_external(src, folder, &req.thread, &opts)
    })?;
    if engine == Engine::Grok {
        let target = host.grok_home_for(account).map_err(ImportError::EngineMissing)?;
        let path = copy_grok(host, src, folder, &req.thread, &target)?;
        *copied = Some(path);
    }
    if host.cancelled().is_some() {
        return Err(ImportError::Abort);
    }
    // Race re-check: same function, fresh list. Only our own copy is removed; no re-check after the rm.
    if let Some(raced) = bound_session_id(host, engine, &req.thread) {
        if let Some(path) = copied.take().filter(|p| !p.is_empty()) {
            host.rm(&path);
        }
        return Ok(OpenResult { session_id: raced, existing: true });
    }
    let now = host.now_iso();
    let turns = turns_of(host, engine, &external, &now);
    let last_turn_id = match turns.last() {
        Some(t) if !t.id.is_empty() => Some(t.id.clone()),
        _ => None,
    };
    let bound = EngineThread {
        thread_id: req.thread.clone(),
        last_turn_id,
        fork: engine != Engine::Grok && req.mode == Mode::Fork,
    };
    let origin = ExternalThread {
        engine,
        source: src.source.clone(),
        thread: req.thread.clone(),
        mode: req.mode,
        imported_at: now,
        omitted_before: external.omitted_before,
        last_plan: match &external.last_plan {
            Some(Plan::Text(t)) if t.is_empty() => None,
            p => p.clone(),
        },
    };
    let title = title_of(&external, folder);
    let model = host.model_for(engine, account, external.model.as_deref());
    let seed = Seed {
        engine,
        account: account.filter(|a| !a.is_empty()).map(|a| a.to_string()),
        model,
        folder: folder.to_string(),
        title,
        turns,
        origin,
        slot: thread_slot(engine, account),
        bound,
    };
    let id = host.create_imported(seed).map_err(ImportError::Other)?;
    *copied = None; // createImported returned: the copy stays
    Ok(OpenResult { session_id: id, existing: false })
}

/// TS: copyGrokSession with the verify callback `home => timed(grokLoadCheck(check, home, ...), cancel)` (R:164-166;
/// spec §3.9). The copy side calls the callback only when it verifies, with its own home; failure mapping and the
/// partial-copy cleanup are the copy side's, so the handler owns a path only after the whole copy succeeded.
// Pruv: root::open_step (calls 34 / 35)
fn copy_grok<H: Host>(host: &mut H, src: &Source, folder: &str, thread: &str, target: &GrokTarget) -> Result<String, ImportError> {
    host.copy_grok_session(src, folder, thread, &target.home, &mut |h: &mut H, home: &str| {
        timed(h, true, |h: &mut H| h.grok_load_check(target, home, folder, thread))
    })
}

/// TS: importFailure (R:54-69; spec §4.1). Abort first, then the EngineImportError codes; anything else passes through.
// Pruv: req_error::map_wire
pub fn import_failure(e: ImportError) -> Fail {
    match e {
        ImportError::Abort | ImportError::Timeout => {
            wire_fail("wireWork.engineImport.timeout", "터미널 대화를 읽는 데 시간이 너무 오래 걸렸어.")
        }
        ImportError::NotFound => wire_fail("wireWork.engineImport.notFound", "터미널 대화를 찾을 수 없어."),
        ImportError::CodexUnsupported => {
            wire_fail("wireWork.engineImport.codexUnsupported", "이 Codex 버전은 바깥 대화 읽기를 지원하지 않아.")
        }
        ImportError::GrokExists => wire_fail("wireWork.engineImport.grokExists", "Lair 의 Grok 폴더에 같은 번호의 대화가 이미 있어."),
        ImportError::GrokLoadFailed => wire_fail("wireWork.engineImport.grokLoadFailed", "복사한 Grok 대화를 열지 못했어."),
        ImportError::EngineMissing(_) => wire_fail("wireWork.engineImport.engineMissing", "이 엔진을 찾을 수 없어."),
        ImportError::ReadFailed => wire_fail("wireWork.engineImport.readFailed", "터미널 대화 기록을 읽지 못했어."),
        ImportError::Other(s) => Fail::Passthrough(s),
    }
}

/// TS: timed (R:71-78; spec §3.8). Starts the 20_000 ms timer, runs the work, clears the timer on both paths.
/// `ext_cancel`: this call has an external cancel; if it is already set the work is not run and `Abort` is returned.
// Pruv: none (call adapters: root::after_folder, open_cont, read4_done, open_step)
pub fn timed<H: Host, T>(host: &mut H, ext_cancel: bool, work: impl FnOnce(&mut H) -> Result<T, ImportError>) -> Result<T, ImportError> {
    let timer = host.timer_start(RPC_MS);
    let result = if ext_cancel && host.cancelled().is_some() { Err(ImportError::Abort) } else { work(host) };
    host.timer_clear(timer);
    result
}

/// TS: findSource (L3-8; R:81). First match, borrowed.
// Pruv: req_child::in2, root::after_find
pub fn source_of<'a>(sources: &'a [Source], id: &str) -> Option<&'a Source> {
    sources.iter().find(|s| s.source == id)
}

/// TS: folderOf (R:86-90; spec §3.2)
// Pruv: root::after_folder, req_error::folder_key, req_error::folder_default
pub fn folder_of<H: Host>(host: &mut H, folder: &str) -> Result<String, Fail> {
    match host.check_folder(folder) {
        Ok(real) => Ok(real),
        Err(Some(msg)) => Err(wire_fail("wireWork.engineImport.folder", &msg)),
        Err(None) => Err(wire_fail("wireWork.engineImport.folder", "이 폴더는 쓸 수 없어.")),
    }
}

/// TS: boundSessionId (R:96-102; spec §3.3). A fresh session_list per call; temporary flag first, then is_temporary,
/// then the candidates; returns at the first match. An empty matched id stops the search and counts as no hit.
// Pruv: root::next_call, root::after_op8, root::bound_hit
pub fn bound_session_id<H: Host>(host: &mut H, engine: Engine, thread: &str) -> Option<String> {
    let sessions: Vec<Session> = host.session_list().to_vec();
    for s in &sessions {
        if s.temporary {
            continue;
        }
        if host.is_temporary(&s.id) {
            continue;
        }
        if session_threads(s, engine, thread) {
            return if s.id.is_empty() { None } else { Some(s.id.clone()) };
        }
    }
    None
}

/// TS: sessionThreads (entry bound.ts, used R:99). Does any candidate of the session equal (engine, thread)?
/// Order: external thread, forks, terminal cli (engine default "codex"), engineThreads (engine = slot key before the
/// first ':'). Compared in place, no allocation.
// Pruv: root::after_op8 (child op 7 rows)
pub fn session_threads(session: &Session, engine: Engine, thread: &str) -> bool {
    let en = engine.name();
    if let Some(ext) = &session.external {
        if ext.engine == en && ext.thread == thread {
            return true;
        }
        if ext.engine == en && ext.forks.iter().any(|f| f == thread) {
            return true;
        }
    }
    if let Some((cli_engine, cli_thread)) = &session.terminal_cli {
        let ce = cli_engine.as_deref().unwrap_or("codex");
        if ce == en && cli_thread == thread {
            return true;
        }
    }
    for (slot, id) in &session.engine_threads {
        let slot_engine = slot.split(':').next().unwrap_or("");
        if slot_engine == en && id == thread {
            return true;
        }
    }
    false
}

/// TS: accountOf (R:104-110; spec §3.4)
// Pruv: root::acct_default, root::rows_acct, root::open_acct
pub fn account_of<H: Host>(host: &mut H, source: &Source, target: Option<&str>) -> Result<Option<String>, Fail> {
    if source.engine != Engine::Grok {
        return Ok(source.account.clone());
    }
    let target = match target {
        Some(t) if !t.is_empty() => t,
        _ => return Ok(None),
    };
    let is_default = target == host.default_account();
    if is_default {
        return Ok(None);
    }
    if !host.grok_accounts().iter().any(|a| a == target) {
        return Err(wire_fail("wireWork.engineImport.accountUnknown", "고른 Grok 계정을 찾을 수 없어."));
    }
    Ok(Some(target.to_string()))
}

/// TS: readOptions (R:112-116; spec §3.7). OpenCode: locate first (open only), then Codex deps, then tempRoot only
/// when the binary is truthy.
// Pruv: root::after_folder, root::open_step, req_child::in3, req_child::in4
pub fn read_options<H: Host>(host: &mut H, src: &Source, with_binary: bool) -> Result<ReadOptions, ImportError> {
    let binary = if src.engine == Engine::Opencode && with_binary {
        match host.locate_opencode()? {
            Some(b) if !b.is_empty() => Some(b),
            _ => None,
        }
    } else {
        None
    };
    let codex = if src.engine == Engine::Codex {
        host.codex_run_deps(src)?;
        true
    } else {
        false
    };
    let opencode = match binary {
        Some(b) => {
            let temp_root = host.temp_root()?;
            Some(OpencodeOptions { binary: b, temp_root })
        }
        None => None,
    };
    Ok(ReadOptions { codex, opencode })
}

/// TS: external.turns.map(...) (R:176-180; spec §3.11). One uuid per turn, in order.
// Pruv: req_fill::gt0, req_fill::recipe
pub fn turns_of<H: Host>(host: &mut H, engine: Engine, external: &External, now: &str) -> Vec<Turn> {
    let mut out = Vec::with_capacity(external.turns.len());
    for item in &external.turns {
        out.push(Turn {
            id: host.uuid(),
            prompt: item.prompt.clone(),
            reply: item.reply.clone(),
            created_at: match &item.created_at {
                Some(c) => c.clone(),
                None => now.to_string(),
            },
            completed_at: item.completed_at.clone(),
            imported_engine: engine,
            imported_tools: if item.tools > 0.0 { Some(item.tools) } else { None },
            imported_clipped: item.clipped,
        });
    }
    out
}

/// ECMAScript `\s` for one UTF-16 unit (spec §3.12).
fn is_es_space(u: u16) -> bool {
    matches!(u, 0x0009..=0x000D | 0x0020 | 0x00A0 | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000 | 0xFEFF)
}

/// TS: title expression (R:187; spec §3.12). Candidate = title || first prompt || ''; collapse `\s+` to one space,
/// trim, first 80 UTF-16 units (a split pair keeps the high surrogate); empty -> basename(folder) || folder.
// Pruv: req_fill::title_head, req_fill::title_pool
pub fn title_of(external: &External, folder: &str) -> Vec<u16> {
    let candidate: &str = if !external.title.is_empty() {
        &external.title
    } else {
        match external.turns.first() {
            Some(t) => &t.prompt,
            None => "",
        }
    };
    let mut collapsed: Vec<u16> = Vec::new();
    let mut in_space = false;
    for u in candidate.encode_utf16() {
        if is_es_space(u) {
            if !in_space {
                collapsed.push(0x0020);
            }
            in_space = true;
        } else {
            collapsed.push(u);
            in_space = false;
        }
    }
    // trim: after collapsing only a single leading/trailing space can remain
    let mut start = 0;
    let mut end = collapsed.len();
    if start < end && collapsed[start] == 0x0020 {
        start += 1;
    }
    if start < end && collapsed[end - 1] == 0x0020 {
        end -= 1;
    }
    let mut title: Vec<u16> = collapsed[start..end].to_vec();
    title.truncate(80);
    if title.is_empty() {
        let base = basename(folder);
        let fallback = if base.is_empty() { folder } else { base };
        return fallback.encode_utf16().collect();
    }
    title
}

/// TS: node:path basename (R:5, R:187), POSIX form: trailing slashes ignored, last segment.
// Pruv: req_fill::basename
pub fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(i) => &trimmed[i + 1..],
        None => trimmed,
    }
}

/// TS: threadSlot (logic-records.ts:7-9). Account is truthy -> `engine:account`, else the engine name.
// Pruv: req_fill::slot_name
pub fn thread_slot(engine: Engine, account: Option<&str>) -> String {
    match account {
        Some(a) if !a.is_empty() => format!("{}:{}", engine.name(), a),
        _ => engine.name().to_string(),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeHost {
        trace: Vec<String>,
        sources: Result<Vec<Source>, String>,
        sessions: Vec<Session>,
        temporary_ids: HashSet<String>,
        folder: Result<String, Option<String>>,
        grok_accounts: Vec<String>,
        default_account: String,
        from_app: bool,
        list_result: Result<Vec<ListItem>, ImportError>,
        read_result: Result<External, ImportError>,
        grok_home: Result<GrokTarget, Option<String>>,
        copy_result: Result<String, ImportError>,
        copy_verify: Option<String>,
        check_result: Result<(), ImportError>,
        create_result: Result<String, String>,
        cancel: Option<String>,
        bind_on_read: Option<Session>,
        seeds: Vec<Seed>,
        uuids: u32,
        timers: u32,
    }

    impl FakeHost {
        fn new() -> FakeHost {
            FakeHost {
                trace: Vec::new(),
                sources: Ok(vec![home("claude", Engine::Claude)]),
                sessions: Vec::new(),
                temporary_ids: HashSet::new(),
                folder: Ok("/work/proj".to_string()),
                grok_accounts: Vec::new(),
                default_account: "default".to_string(),
                from_app: false,
                list_result: Ok(Vec::new()),
                read_result: Ok(ext("T", &["hello"])),
                grok_home: Ok(GrokTarget { home: "/h".to_string(), binary: "grok".to_string() }),
                copy_result: Ok("/h/copy-t1".to_string()),
                copy_verify: None,
                check_result: Ok(()),
                create_result: Ok("new-1".to_string()),
                cancel: None,
                bind_on_read: None,
                seeds: Vec::new(),
                uuids: 0,
                timers: 0,
            }
        }
        fn count(&self, prefix: &str) -> usize {
            self.trace.iter().filter(|t| t.starts_with(prefix)).count()
        }
        fn has(&self, line: &str) -> bool {
            self.trace.iter().any(|t| t == line)
        }
    }

    impl Host for FakeHost {
        fn import_sources(&mut self) -> Result<Vec<Source>, String> {
            self.trace.push("import_sources".to_string());
            self.sources.clone()
        }
        fn session_list(&mut self) -> &[Session] {
            self.trace.push("session_list".to_string());
            &self.sessions
        }
        fn is_temporary(&mut self, id: &str) -> bool {
            self.trace.push(format!("is_temporary:{}", id));
            self.temporary_ids.contains(id)
        }
        fn check_folder(&mut self, path: &str) -> Result<String, Option<String>> {
            self.trace.push(format!("check_folder:{}", path));
            self.folder.clone()
        }
        fn grok_accounts(&mut self) -> Vec<String> {
            self.trace.push("grok_accounts".to_string());
            self.grok_accounts.clone()
        }
        fn default_account(&self) -> &str {
            &self.default_account
        }
        fn from_app_action(&mut self) -> bool {
            self.trace.push("from_app_action".to_string());
            self.from_app
        }
        fn model_for(&mut self, engine: Engine, _account: Option<&str>, wanted: Option<&str>) -> String {
            self.trace.push(format!("model_for:{}", engine.name()));
            wanted.unwrap_or("default-model").to_string()
        }
        fn create_imported(&mut self, seed: Seed) -> Result<String, String> {
            self.trace.push("create_imported".to_string());
            self.seeds.push(seed);
            self.create_result.clone()
        }
        fn now_iso(&mut self) -> String {
            self.trace.push("now_iso".to_string());
            "2026-01-01T00:00:00.000Z".to_string()
        }
        fn uuid(&mut self) -> String {
            self.uuids += 1;
            self.trace.push("uuid".to_string());
            format!("u{}", self.uuids)
        }
        fn rm(&mut self, path: &str) {
            self.trace.push(format!("rm:{}", path));
        }
        fn timer_start(&mut self, ms: u32) -> u32 {
            self.timers += 1;
            self.trace.push(format!("timer_start:{}", ms));
            self.timers
        }
        fn timer_clear(&mut self, t: u32) {
            self.trace.push(format!("timer_clear:{}", t));
        }
        fn cancelled(&self) -> Option<String> {
            self.cancel.clone()
        }
        fn codex_run_deps(&mut self, _src: &Source) -> Result<(), ImportError> {
            self.trace.push("codex_run_deps".to_string());
            Ok(())
        }
        fn locate_opencode(&mut self) -> Result<Option<String>, ImportError> {
            self.trace.push("locate_opencode".to_string());
            Ok(None)
        }
        fn temp_root(&mut self) -> Result<String, ImportError> {
            self.trace.push("temp_root".to_string());
            Ok("/tmp/root".to_string())
        }
        fn list_external(&mut self, _src: &Source, _folder: &str, _opts: &ReadOptions) -> Result<Vec<ListItem>, ImportError> {
            self.trace.push("list_external".to_string());
            self.list_result.clone()
        }
        fn read_external(&mut self, _src: &Source, _folder: &str, _thread: &str, _opts: &ReadOptions) -> Result<External, ImportError> {
            self.trace.push("read_external".to_string());
            if let Some(s) = self.bind_on_read.take() {
                self.sessions.push(s);
            }
            self.read_result.clone()
        }
        fn grok_home_for(&mut self, _account: Option<&str>) -> Result<GrokTarget, Option<String>> {
            self.trace.push("grok_home_for".to_string());
            self.grok_home.clone()
        }
        fn copy_grok_session(
            &mut self,
            _src: &Source,
            _folder: &str,
            _thread: &str,
            _home: &str,
            verify: &mut dyn FnMut(&mut Self, &str) -> Result<(), ImportError>,
        ) -> Result<String, ImportError> {
            self.trace.push("copy_grok_session".to_string());
            let path = self.copy_result.clone()?;
            if let Some(vhome) = self.copy_verify.clone() {
                if let Err(e) = verify(self, &vhome) {
                    self.trace.push(format!("copy_cleanup:{}", path));
                    return Err(match e {
                        ImportError::Abort => ImportError::Abort,
                        _ => ImportError::GrokLoadFailed,
                    });
                }
            }
            Ok(path)
        }
        fn grok_load_check(&mut self, _target: &GrokTarget, _home: &str, _folder: &str, _thread: &str) -> Result<(), ImportError> {
            self.trace.push("grok_load_check".to_string());
            self.check_result.clone()
        }
    }

    fn home(id: &str, engine: Engine) -> Source {
        Source { source: id.to_string(), engine, account: None, kind: SourceKind::Home, account_name: None }
    }

    fn session(id: &str) -> Session {
        Session { id: id.to_string(), title: format!("title-{}", id), temporary: false, external: None, terminal_cli: None, engine_threads: Vec::new() }
    }

    fn with_slot(mut s: Session, slot: &str, thread: &str) -> Session {
        s.engine_threads.push((slot.to_string(), thread.to_string()));
        s
    }

    fn ext(title: &str, prompts: &[&str]) -> External {
        External {
            title: title.to_string(),
            model: None,
            turns: prompts
                .iter()
                .map(|p| ExternalTurn { prompt: p.to_string(), reply: "r".to_string(), created_at: None, completed_at: Field::Missing, tools: 0.0, clipped: false })
                .collect(),
            omitted_before: false,
            last_plan: None,
        }
    }

    fn item(thread: &str, title: &str) -> ListItem {
        ListItem { thread: thread.to_string(), title: title.to_string(), updated_at: Field::Missing, size: Field::Missing, messages: Field::Missing, path: Field::Missing, maybe_open: false }
    }

    fn open_req(source: &str, thread: &str, mode: Mode) -> OpenRequest {
        OpenRequest { source: source.to_string(), folder: "/work/proj".to_string(), thread: thread.to_string(), mode, target_account: None }
    }

    fn u16s(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn wire_key(f: &Fail) -> &'static str {
        match f {
            Fail::Wire(w) => w.key,
            _ => "",
        }
    }

    #[test]
    fn sources_projection() {
        let mut host = FakeHost::new();
        host.sources = Ok(vec![
            home("claude", Engine::Claude),
            Source { source: "codex:w".to_string(), engine: Engine::Codex, account: Some("w".to_string()), kind: SourceKind::Account, account_name: Some("Work".to_string()) },
            Source { source: "grok:x".to_string(), engine: Engine::Grok, account: Some(String::new()), kind: SourceKind::Account, account_name: None },
        ]);
        let out = Requests::default().sources(&mut host).unwrap_or_default();
        assert_eq!(
            out,
            vec![
                PublicSource { source: "claude".to_string(), engine: Engine::Claude, account: None, label: String::new() },
                PublicSource { source: "codex:w".to_string(), engine: Engine::Codex, account: Some("w".to_string()), label: "Work".to_string() },
                PublicSource { source: "grok:x".to_string(), engine: Engine::Grok, account: None, label: String::new() },
            ]
        );
    }

    #[test]
    fn list_title_from_bound_session() {
        let mut host = FakeHost::new();
        host.list_result = Ok(vec![item("t1", "raw1"), item("t2", "raw2"), item("t3", "raw3")]);
        let mut s2 = with_slot(session("S2"), "claude", "t2");
        s2.title = String::new();
        host.sessions = vec![with_slot(session("S1"), "claude", "t1"), s2];
        let rows = Requests::default().list(&mut host, "claude", "/work/proj").unwrap_or_default();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].title, "title-S1");
        assert_eq!(rows[0].bound_session_id, Some("S1".to_string()));
        assert_eq!(rows[1].title, "raw2"); // empty Lair title falls back
        assert_eq!(rows[1].bound_session_id, Some("S2".to_string()));
        assert_eq!(rows[2].title, "raw3");
        assert_eq!(rows[2].bound_session_id, None);
        // one titles observation + one per item
        assert_eq!(host.count("session_list"), 4);
        assert_eq!(host.count("timer_start"), 1);
        assert_eq!(host.count("timer_clear"), 1);
    }

    #[test]
    fn list_field_inclusion_keeps_zero_and_empty() {
        let mut host = FakeHost::new();
        host.list_result = Ok(vec![ListItem {
            thread: "t".to_string(),
            title: "x".to_string(),
            updated_at: Field::Null,
            size: Field::Val(0.0),
            messages: Field::Val(0.0),
            path: Field::Val(String::new()),
            maybe_open: true,
        }]);
        let rows = Requests::default().list(&mut host, "claude", "/work/proj").unwrap_or_default();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].updated_at, Field::Null);
        assert_eq!(rows[0].size, Field::Val(0.0));
        assert_eq!(rows[0].messages, Field::Val(0.0));
        assert_eq!(rows[0].path, Field::Val(String::new()));
        assert!(rows[0].maybe_open);
        assert_eq!(rows[0].bound_session_id, None);
    }

    #[test]
    fn bound_search_order_and_first_match_stop() {
        let mut host = FakeHost::new();
        let a = session("A");
        let mut b = session("B");
        b.external = Some(ExternalBound { engine: "claude".to_string(), thread: "orig".to_string(), forks: vec!["t9".to_string()] });
        let c = with_slot(session("C"), "claude", "t9");
        host.sessions = vec![a, b, c];
        assert_eq!(bound_session_id(&mut host, Engine::Claude, "t9"), Some("B".to_string()));
        assert_eq!(host.count("is_temporary"), 2); // A, B; C never asked
        assert_eq!(host.count("session_list"), 1);
        // terminal cli engine defaults to codex; slot engine is the part before the first ':'
        let mut d = session("D");
        d.terminal_cli = Some((None, "tc".to_string()));
        let e = with_slot(session("E"), "qwen:acct", "tq");
        host.sessions = vec![d, e];
        assert_eq!(bound_session_id(&mut host, Engine::Codex, "tc"), Some("D".to_string()));
        assert_eq!(bound_session_id(&mut host, Engine::Claude, "tc"), None);
        assert_eq!(bound_session_id(&mut host, Engine::Qwen, "tq"), Some("E".to_string()));
    }

    #[test]
    fn temporary_flag_skips_is_temporary() {
        let mut host = FakeHost::new();
        let mut first = with_slot(session("TMP"), "claude", "t1");
        first.temporary = true;
        let second = with_slot(session("REAL"), "claude", "t1");
        host.sessions = vec![first, second];
        assert_eq!(bound_session_id(&mut host, Engine::Claude, "t1"), Some("REAL".to_string()));
        assert!(!host.has("is_temporary:TMP"));
        assert!(host.has("is_temporary:REAL"));
        // is_temporary true excludes too
        host.temporary_ids.insert("REAL".to_string());
        assert_eq!(bound_session_id(&mut host, Engine::Claude, "t1"), None);
    }

    #[test]
    fn open_existing_returns_before_busy() {
        let mut host = FakeHost::new();
        host.sessions = vec![with_slot(session("S1"), "claude", "t1")];
        host.cancel = Some("already cancelled".to_string());
        let mut reqs = Requests::default();
        reqs.opening.insert("claude:t1".to_string());
        let out = reqs.open(&mut host, &open_req("claude", "t1", Mode::Fork));
        assert_eq!(out, Ok(OpenResult { session_id: "S1".to_string(), existing: true }));
        assert_eq!(host.count("timer_start"), 0);
        assert!(reqs.opening.contains("claude:t1")); // not ours: untouched
    }

    #[test]
    fn open_busy_refusal() {
        let mut host = FakeHost::new();
        let mut reqs = Requests::default();
        reqs.opening.insert("claude:t1".to_string());
        let out = reqs.open(&mut host, &open_req("claude", "t1", Mode::Fork));
        match out {
            Err(Fail::Wire(w)) => {
                assert_eq!(w.key, "wireWork.engineImport.busy");
                assert_eq!(w.message, "같은 터미널 대화를 이미 여는 중이야.");
            }
            other => panic!("expected busy, got {:?}", other),
        }
        assert_eq!(host.count("read_external"), 0);
        assert!(reqs.opening.contains("claude:t1"));
    }

    #[test]
    fn same_refusals_order() {
        let mut host = FakeHost::new();
        host.from_app = true;
        host.sources = Ok(vec![
            home("g", Engine::Grok),
            home("c", Engine::Claude),
            Source { source: "ca".to_string(), engine: Engine::Claude, account: Some("w".to_string()), kind: SourceKind::Account, account_name: None },
        ]);
        let mut reqs = Requests::default();
        let r1 = reqs.open(&mut host, &open_req("g", "t", Mode::Same));
        assert_eq!(r1.as_ref().err().map(wire_key), Some("wireWork.engineImport.sameRefused"));
        assert!(!host.has("from_app_action")); // refusal first, the AI check is not reached
        let r2 = reqs.open(&mut host, &open_req("ca", "t", Mode::Same));
        assert_eq!(r2.as_ref().err().map(wire_key), Some("wireWork.engineImport.sameRefused"));
        assert!(!host.has("from_app_action"));
        let r3 = reqs.open(&mut host, &open_req("c", "t", Mode::Same));
        assert_eq!(r3.as_ref().err().map(wire_key), Some("wireWork.engineImport.sameByAi"));
        assert_eq!(host.count("from_app_action"), 1);
        assert_eq!(host.count("read_external"), 0);
        // fork never asks
        let before = host.count("from_app_action");
        let _ = reqs.open(&mut host, &open_req("c", "t", Mode::Fork));
        assert_eq!(host.count("from_app_action"), before);
    }

    #[test]
    fn grok_account_unknown() {
        let mut host = FakeHost::new();
        host.sources = Ok(vec![home("grok", Engine::Grok)]);
        host.grok_accounts = vec!["a".to_string()];
        let mut req = open_req("grok", "t1", Mode::Fork);
        req.target_account = Some("nope".to_string());
        let mut reqs = Requests::default();
        let out = reqs.open(&mut host, &req);
        match out {
            Err(Fail::Wire(w)) => {
                assert_eq!(w.key, "wireWork.engineImport.accountUnknown");
                assert_eq!(w.message, "고른 Grok 계정을 찾을 수 없어.");
            }
            other => panic!("expected accountUnknown, got {:?}", other),
        }
        assert_eq!(host.count("read_external"), 0);
        // the default account and a known one are accepted
        assert_eq!(account_of(&mut host, &home("grok", Engine::Grok), Some("default")).ok(), Some(None));
        assert_eq!(account_of(&mut host, &home("grok", Engine::Grok), Some("a")).ok(), Some(Some("a".to_string())));
    }

    #[test]
    fn race_path_rms_own_copy_and_returns_existing() {
        let mut host = FakeHost::new();
        host.sources = Ok(vec![home("grok", Engine::Grok)]);
        let mut racer = session("S9");
        racer.external = Some(ExternalBound { engine: "grok".to_string(), thread: "t1".to_string(), forks: Vec::new() });
        host.bind_on_read = Some(racer);
        let mut reqs = Requests::default();
        let out = reqs.open(&mut host, &open_req("grok", "t1", Mode::Fork));
        assert_eq!(out, Ok(OpenResult { session_id: "S9".to_string(), existing: true }));
        assert!(host.has("rm:/h/copy-t1"));
        assert_eq!(host.count("create_imported"), 0);
        assert_eq!(host.count("now_iso"), 0);
        assert_eq!(host.count("uuid"), 0);
        assert_eq!(host.count("model_for"), 0);
        assert!(reqs.opening.is_empty());
        assert_eq!(host.count("timer_start"), host.count("timer_clear"));
    }

    #[test]
    fn failure_after_copy_rms_and_cancel_wins() {
        let mut host = FakeHost::new();
        host.sources = Ok(vec![home("grok", Engine::Grok)]);
        host.create_result = Err("boom".to_string());
        let mut reqs = Requests::default();
        let out = reqs.open(&mut host, &open_req("grok", "t1", Mode::Fork));
        assert_eq!(out, Err(Fail::Passthrough("boom".to_string())));
        assert!(host.has("rm:/h/copy-t1"));
        assert!(reqs.opening.is_empty());

        let mut host2 = FakeHost::new();
        host2.sources = Ok(vec![home("grok", Engine::Grok)]);
        host2.create_result = Err("boom".to_string());
        // cancel arrives after the read: set via bind trick is not needed, cancelled() is a plain fact; the
        // pre-save check also fires, so the cancel reason wins either way and the copy is removed.
        host2.cancel = None;
        let mut reqs2 = Requests::default();
        let _ = reqs2.open(&mut host2, &open_req("grok", "t1", Mode::Fork));
        let mut host3 = FakeHost::new();
        host3.sources = Ok(vec![home("grok", Engine::Grok)]);
        host3.create_result = Err("boom".to_string());
        host3.read_result = Err(ImportError::ReadFailed);
        host3.cancel = Some("user stop".to_string());
        let out3 = Requests::default().open(&mut host3, &open_req("grok", "t1", Mode::Fork));
        assert_eq!(out3, Err(Fail::Cancel("user stop".to_string())));
        assert!(reqs2.opening.is_empty());
    }

    #[test]
    fn cancel_after_copy_removes_copy_and_returns_reason() {
        // timed aborts at once when the cancel is already set, so the cancel becomes visible only after the read
        // returns; the copy does not verify, and the pre-save check fires.
        struct Flip(FakeHost);
        impl Host for Flip {
            fn import_sources(&mut self) -> Result<Vec<Source>, String> { self.0.import_sources() }
            fn session_list(&mut self) -> &[Session] { self.0.session_list() }
            fn is_temporary(&mut self, id: &str) -> bool { self.0.is_temporary(id) }
            fn check_folder(&mut self, p: &str) -> Result<String, Option<String>> { self.0.check_folder(p) }
            fn grok_accounts(&mut self) -> Vec<String> { self.0.grok_accounts() }
            fn default_account(&self) -> &str { self.0.default_account() }
            fn from_app_action(&mut self) -> bool { self.0.from_app_action() }
            fn model_for(&mut self, e: Engine, a: Option<&str>, w: Option<&str>) -> String { self.0.model_for(e, a, w) }
            fn create_imported(&mut self, s: Seed) -> Result<String, String> { self.0.create_imported(s) }
            fn now_iso(&mut self) -> String { self.0.now_iso() }
            fn uuid(&mut self) -> String { self.0.uuid() }
            fn rm(&mut self, p: &str) { self.0.rm(p) }
            fn timer_start(&mut self, ms: u32) -> u32 { self.0.timer_start(ms) }
            fn timer_clear(&mut self, t: u32) { self.0.timer_clear(t) }
            fn cancelled(&self) -> Option<String> { self.0.cancel.clone() }
            fn codex_run_deps(&mut self, s: &Source) -> Result<(), ImportError> { self.0.codex_run_deps(s) }
            fn locate_opencode(&mut self) -> Result<Option<String>, ImportError> { self.0.locate_opencode() }
            fn temp_root(&mut self) -> Result<String, ImportError> { self.0.temp_root() }
            fn list_external(&mut self, s: &Source, f: &str, o: &ReadOptions) -> Result<Vec<ListItem>, ImportError> { self.0.list_external(s, f, o) }
            fn read_external(&mut self, s: &Source, f: &str, t: &str, o: &ReadOptions) -> Result<External, ImportError> {
                let r = self.0.read_external(s, f, t, o);
                self.0.cancel = Some("late stop".to_string());
                r
            }
            fn grok_home_for(&mut self, a: Option<&str>) -> Result<GrokTarget, Option<String>> { self.0.grok_home_for(a) }
            fn copy_grok_session(
                &mut self,
                _s: &Source,
                _f: &str,
                _t: &str,
                _h: &str,
                _verify: &mut dyn FnMut(&mut Self, &str) -> Result<(), ImportError>,
            ) -> Result<String, ImportError> {
                self.0.trace.push("copy_grok_session".to_string());
                self.0.copy_result.clone()
            }
            fn grok_load_check(&mut self, t: &GrokTarget, h: &str, f: &str, th: &str) -> Result<(), ImportError> { self.0.grok_load_check(t, h, f, th) }
        }
        let mut inner = FakeHost::new();
        inner.sources = Ok(vec![home("grok", Engine::Grok)]);
        let mut host = Flip(inner);
        let mut reqs = Requests::default();
        let out = reqs.open(&mut host, &open_req("grok", "t1", Mode::Fork));
        // the pre-save check sees the cancel; the copy is removed and the reason wins
        assert_eq!(out, Err(Fail::Cancel("late stop".to_string())));
        assert!(host.0.has("rm:/h/copy-t1"));
        assert_eq!(host.0.count("create_imported"), 0);
        assert!(reqs.opening.is_empty());
    }

    #[test]
    fn title_fallback_to_basename() {
        let e = ext("", &[]);
        assert_eq!(title_of(&e, "/a/b/proj"), u16s("proj"));
        assert_eq!(title_of(&e, "/a/b/proj/"), u16s("proj"));
        assert_eq!(title_of(&e, "/"), u16s("/"));
        // first prompt is used when the engine title is empty; whitespace collapses and trims
        let p = ext("", &["  hello \n\t world  "]);
        assert_eq!(title_of(&p, "/a/b"), u16s("hello world"));
        // a whitespace-only title is chosen first, becomes empty, and does NOT fall back to the prompt
        let w = ext("   ", &["prompt"]);
        assert_eq!(title_of(&w, "/a/b"), u16s("b"));
        // U+0085 and U+200B are not whitespace
        let n = ext("a\u{85}b\u{200B}c", &[]);
        assert_eq!(title_of(&n, "/x"), u16s("a\u{85}b\u{200B}c"));
    }

    #[test]
    fn title_cut_at_80_units_keeps_high_surrogate() {
        let long = format!("{}{}", "a".repeat(79), "😀");
        let t = title_of(&ext(&long, &[]), "/x");
        assert_eq!(t.len(), 80);
        assert_eq!(t[79], 0xD83D);
        let plain = "b".repeat(100);
        assert_eq!(title_of(&ext(&plain, &[]), "/x").len(), 80);
    }

    #[test]
    fn open_success_builds_seed_and_releases_key() {
        let mut host = FakeHost::new();
        let mut e = ext("My title", &["p1", "p2"]);
        e.turns[0].created_at = Some(String::new()); // empty createdAt is kept
        e.turns[1].tools = 3.0;
        e.turns[1].clipped = true;
        e.turns[1].completed_at = Field::Val("done".to_string());
        e.omitted_before = true;
        e.model = Some("m-1".to_string());
        host.read_result = Ok(e);
        let mut reqs = Requests::default();
        let out = reqs.open(&mut host, &open_req("claude", "t1", Mode::Fork));
        assert_eq!(out, Ok(OpenResult { session_id: "new-1".to_string(), existing: false }));
        assert!(reqs.opening.is_empty());
        assert_eq!(host.seeds.len(), 1);
        let seed = &host.seeds[0];
        assert_eq!(seed.title, u16s("My title"));
        assert_eq!(seed.model, "m-1");
        assert_eq!(seed.slot, "claude");
        assert_eq!(seed.turns[0].id, "u1");
        assert_eq!(seed.turns[0].created_at, "");
        assert_eq!(seed.turns[1].imported_tools, Some(3.0));
        assert!(seed.turns[1].imported_clipped);
        assert_eq!(seed.bound, EngineThread { thread_id: "t1".to_string(), last_turn_id: Some("u2".to_string()), fork: true });
        assert!(seed.origin.omitted_before);
        assert_eq!(seed.origin.imported_at, "2026-01-01T00:00:00.000Z");
        // order: read, now, uuids, model_for, create
        let pos = |s: &str| host.trace.iter().position(|t| t == s);
        assert!(pos("read_external") < pos("now_iso"));
        assert!(pos("now_iso") < pos("uuid"));
        assert!(pos("uuid") < pos("model_for:claude"));
        assert!(pos("model_for:claude") < pos("create_imported"));
    }

    #[test]
    fn import_failure_mapping_and_key_release() {
        assert_eq!(wire_key(&import_failure(ImportError::Abort)), "wireWork.engineImport.timeout");
        assert_eq!(wire_key(&import_failure(ImportError::EngineMissing(Some("x".to_string())))), "wireWork.engineImport.engineMissing");
        assert_eq!(import_failure(ImportError::Other("z".to_string())), Fail::Passthrough("z".to_string()));
        let mut host = FakeHost::new();
        host.read_result = Err(ImportError::NotFound);
        let mut reqs = Requests::default();
        let out = reqs.open(&mut host, &open_req("claude", "t1", Mode::Fork));
        assert_eq!(out.as_ref().err().map(wire_key), Some("wireWork.engineImport.notFound"));
        assert!(reqs.opening.is_empty());
        assert_eq!(host.count("timer_start"), host.count("timer_clear"));
    }

    #[test]
    fn source_unknown_and_folder_errors() {
        let mut host = FakeHost::new();
        let out = Requests::default().list(&mut host, "nope", "/f");
        assert_eq!(out.as_ref().err().map(wire_key), Some("wireWork.engineImport.sourceUnknown"));
        assert_eq!(host.count("check_folder"), 0);
        host.folder = Err(Some("outside".to_string()));
        match Requests::default().list(&mut host, "claude", "/f") {
            Err(Fail::Wire(w)) => assert_eq!((w.key, w.message.as_str()), ("wireWork.engineImport.folder", "outside")),
            other => panic!("unexpected {:?}", other),
        }
        host.folder = Err(None);
        match Requests::default().list(&mut host, "claude", "/f") {
            Err(Fail::Wire(w)) => assert_eq!(w.message, "이 폴더는 쓸 수 없어."),
            other => panic!("unexpected {:?}", other),
        }
    }

    #[test]
    fn grok_verify_runs_only_when_the_copy_asks() {
        let mut host = FakeHost::new();
        host.sources = Ok(vec![home("grok", Engine::Grok)]);
        let out = Requests::default().open(&mut host, &open_req("grok", "t1", Mode::Fork));
        assert_eq!(out, Ok(OpenResult { session_id: "new-1".to_string(), existing: false }));
        assert_eq!(host.count("grok_load_check"), 0);

        let mut host2 = FakeHost::new();
        host2.sources = Ok(vec![home("grok", Engine::Grok)]);
        host2.copy_verify = Some("/h2".to_string());
        host2.check_result = Err(ImportError::ReadFailed);
        let mut reqs = Requests::default();
        let out2 = reqs.open(&mut host2, &open_req("grok", "t1", Mode::Fork));
        match out2 {
            Err(Fail::Wire(w)) => assert_eq!(w.key, "wireWork.engineImport.grokLoadFailed"),
            other => panic!("unexpected {:?}", other),
        }
        // The copy side cleaned its partial copy; the handler owns no path, so it removes nothing.
        assert!(host2.has("copy_cleanup:/h/copy-t1"));
        assert_eq!(host2.count("rm:"), 0);
        assert_eq!(host2.count("grok_load_check"), 1);
        assert!(reqs.opening.is_empty());
        assert_eq!(host2.count("timer_start"), host2.count("timer_clear"));
    }

    #[test]
    fn empty_copy_path_is_not_removed_and_empty_account_not_seeded() {
        let mut host = FakeHost::new();
        host.sources = Ok(vec![home("grok", Engine::Grok)]);
        host.copy_result = Ok(String::new());
        host.create_result = Err("boom".to_string());
        let _ = Requests::default().open(&mut host, &open_req("grok", "t1", Mode::Fork));
        assert_eq!(host.count("rm:"), 0);

        let mut host2 = FakeHost::new();
        let mut src = home("claude", Engine::Claude);
        src.account = Some(String::new());
        host2.sources = Ok(vec![src]);
        let out = Requests::default().open(&mut host2, &open_req("claude", "t1", Mode::Fork));
        assert!(out.is_ok());
        assert_eq!(host2.seeds[0].account, None);
    }
}
