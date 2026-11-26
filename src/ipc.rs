//! Application state and typed IPC command handlers bridging WebView2 <-> Rust engine.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::assets;
use crate::engine::{Document, EditPatch, RenderedBlock, SearchQuery};
use crate::exports::{self, PandocFormat};
use crate::latex;
use crate::settings::{self, Settings, ThemePreset, ViewMode};
use crate::storage::{self, FileStamp, Fingerprint, Session, SessionTab};
use crate::wallpaper::fx::Effect;
use crate::wallpaper::gallery::{self, BUILTINS};
use crate::wallpaper::store::{self, Prepared};
use crate::wallpaper::theme_tint::{Palette, ResolvedTheme};
use crate::workspace;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TabSummary {
    pub id: String,
    pub title: String,
    pub path: Option<String>,
    pub dirty: bool,
    pub large_mode: bool,
    pub tex_source: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileNode {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub children: Vec<FileNode>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WallpaperTile {
    pub kind: &'static str, // "builtin" | "recent"
    pub id: String,
    pub title: String,
    pub path: String,
    pub thumb_url: String,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WallpaperStatePayload {
    pub active_name: String,
    pub active_path: Option<String>,
    pub image_url: Option<String>,
    pub blurred_url: Option<String>,
    pub effect: Effect,
    pub hero_opacity: f32,
    pub fills_window: bool,
    pub theme_colors: bool,
    pub error: Option<String>,
    pub tiles: Vec<WallpaperTile>,
    pub theme: ResolvedTheme,
}

pub struct AppState {
    pub settings: Settings,
    pub docs: Vec<Document>,
    pub active_tab: Option<usize>,
    pub next_doc_num: u64,
    pub prepared_wp: Option<Prepared>,
    pub wp_error: Option<String>,
    pub folder_root: Option<PathBuf>,
    latex_mains: HashMap<PathBuf, PathBuf>,
    autosaves: HashMap<String, Instant>,
    pending_close: Option<CloseRequest>,
    disk: HashMap<String, TrackedFile>,
    recovery_at: Option<Instant>,
    scan_at: Instant,
    initialized: bool,
    session_path: Option<PathBuf>,
    _session_lease: Option<std::fs::File>,
}

struct TrackedFile {
    expected: Option<Fingerprint>,
    stamp: Option<FileStamp>,
    changed: Option<Option<Fingerprint>>,
    prompted: bool,
    manual_save: bool,
}

impl TrackedFile {
    fn new(expected: Option<Fingerprint>, stamp: Option<FileStamp>) -> Self {
        Self {
            expected,
            stamp,
            changed: None,
            prompted: false,
            manual_save: false,
        }
    }

    fn observe(&mut self, path: &Path, force: bool) -> std::io::Result<()> {
        let stamp = FileStamp::read(path)?;
        if force || self.stamp != stamp {
            let found = storage::disk_fingerprint(path)?;
            let change = (found != self.expected).then_some(found);
            if change != self.changed {
                self.prompted = false;
            }
            self.stamp = stamp;
            self.changed = change;
        }
        Ok(())
    }
}

enum CloseTarget {
    Tab(String),
    Window,
}

struct CloseRequest {
    target: CloseTarget,
    documents: VecDeque<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum CloseChoice {
    Save,
    Discard,
    Cancel,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum WorkspaceAction {
    NewDocument,
    NewFolder,
    Rename,
    Trash,
    Reveal,
}

impl AppState {
    pub fn new(initial_file: Option<PathBuf>) -> Self {
        let settings = settings::load();
        let folder_root = settings
            .last_folder
            .as_deref()
            .map(PathBuf::from)
            .filter(|p| p.is_dir());

        let (session_path, session_lease) = settings::config_dir()
            .and_then(|dir| storage::claim_session(&dir).ok())
            .unzip();
        let mut app = Self {
            settings,
            docs: Vec::new(),
            active_tab: None,
            next_doc_num: 1,
            prepared_wp: None,
            wp_error: None,
            folder_root,
            latex_mains: HashMap::new(),
            autosaves: HashMap::new(),
            pending_close: None,
            disk: HashMap::new(),
            recovery_at: None,
            scan_at: Instant::now(),
            initialized: false,
            session_path,
            _session_lease: session_lease,
        };

        // Prepare initial wallpaper synchronously on fast cache hit, or compute once
        app.sync_wallpaper_now();
        app.restore_recovery();

        if let Some(path) = initial_file {
            let _ = app.open_file_path(&path);
        }

        app
    }

    fn alloc_doc_id(&mut self) -> String {
        let id = format!("doc-{}", self.next_doc_num);
        self.next_doc_num += 1;
        id
    }

    pub fn active_doc(&self) -> Option<&Document> {
        self.active_tab.and_then(|i| self.docs.get(i))
    }

    pub fn active_latex_main(&self) -> Option<PathBuf> {
        let path = self.active_doc()?.path.as_deref()?;
        latex::project_source(path).then_some(())?;
        self.latex_mains
            .get(path)
            .and_then(|main| latex::main_file(main).ok())
            .or_else(|| latex::detect_root(path, self.folder_root.as_deref()))
            .or_else(|| {
                // A chapter may omit a root directive when the main document is already open.
                let scope = self
                    .docs
                    .iter()
                    .filter_map(|doc| doc.path.as_deref())
                    .filter(|source| *source != path && latex::project_source(source))
                    .filter_map(|source| {
                        self.latex_mains
                            .get(source)
                            .cloned()
                            .or_else(|| latex::detect_root(source, None))
                    })
                    .filter_map(|root| {
                        root.parent()
                            .filter(|dir| path.starts_with(dir))
                            .map(Path::to_path_buf)
                    })
                    .min_by_key(|dir| dir.components().count())?;
                latex::detect_root(path, Some(&scope))
            })
    }

    pub fn set_latex_main(&mut self, source: &Path, main: PathBuf) {
        if latex::project_source(source) {
            self.latex_mains.insert(source.to_path_buf(), main);
        }
    }

    pub fn active_doc_mut(&mut self) -> Option<&mut Document> {
        self.active_tab.and_then(|i| self.docs.get_mut(i))
    }

    pub fn tab_summaries(&self) -> Vec<TabSummary> {
        self.docs
            .iter()
            .map(|d| TabSummary {
                id: d.id.clone(),
                title: d.title.clone(),
                path: d.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
                dirty: d.dirty,
                large_mode: d.is_large(),
                tex_source: latex::project_source(
                    d.path.as_deref().unwrap_or_else(|| Path::new(&d.title)),
                ),
            })
            .collect()
    }

    pub fn open_file_path(&mut self, path: &Path) -> Result<usize, String> {
        let path = dunce::canonicalize(path).map_err(|e| format!("Cannot locate file: {e}"))?;
        // Focus existing tab if already open
        if let Some(idx) = self
            .docs
            .iter()
            .position(|d| d.path.as_deref() == Some(path.as_path()))
        {
            self.active_tab = Some(idx);
            self.schedule_recovery();
            return Ok(idx);
        }
        let (text, stamp) = storage::read_document(&path)
            .map_err(|e| format!("Cannot read UTF-8 document: {e}"))?;
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Untitled.md".to_string());
        let id = self.alloc_doc_id();
        self.disk.insert(
            id.clone(),
            TrackedFile::new(Some(storage::fingerprint(text.as_bytes())), Some(stamp)),
        );
        let doc = Document::new(id, title, Some(path.clone()), &text);
        self.docs.push(doc);
        let idx = self.docs.len() - 1;
        self.active_tab = Some(idx);
        self.settings.remember_recent_file(&path.to_string_lossy());
        let _ = settings::save(&self.settings);
        self.schedule_recovery();
        Ok(idx)
    }

    pub fn new_untitled_doc(&mut self) -> usize {
        let num = self.next_doc_num;
        let id = self.alloc_doc_id();
        let doc = Document::new(id, format!("Untitled-{num}.md"), None, "");
        self.docs.push(doc);
        let idx = self.docs.len() - 1;
        self.active_tab = Some(idx);
        self.schedule_recovery();
        idx
    }

    fn restore_recovery(&mut self) {
        let Some(path) = self.session_path.clone() else {
            return;
        };
        let read = |path: &Path| -> Option<Session> {
            let file = std::fs::File::open(path).ok()?;
            serde_json::from_reader(std::io::BufReader::new(file)).ok()
        };
        if let Some(session) = read(&path) {
            self.restore_session(session);
        }
        if path.file_name().and_then(|name| name.to_str()) != Some("session.json") {
            return;
        }
        let Some(directory) = path.parent() else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(directory) else {
            return;
        };
        let mut recovered = Vec::new();
        for entry in entries.flatten() {
            let candidate = entry.path();
            if candidate
                .extension()
                .and_then(|extension| extension.to_str())
                != Some("json")
                || !candidate
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("session-extra-"))
            {
                continue;
            }
            if let Ok(lease) = storage::lock_session(&candidate)
                && let Some(session) = read(&candidate)
            {
                let active = self.active_tab;
                self.restore_session(session);
                self.active_tab = active.or(self.active_tab);
                recovered.push((candidate, lease));
            }
        }
        if !recovered.is_empty() && self.persist_session(true).is_ok() {
            for (candidate, lease) in recovered {
                drop(lease);
                let _ = std::fs::remove_file(&candidate);
                let _ = std::fs::remove_file(candidate.with_extension("lock"));
            }
        }
    }

    fn restore_session(&mut self, session: Session) {
        for (index, mut tab) in session.tabs.into_iter().enumerate() {
            if tab.path.is_some() && self.docs.iter().any(|doc| doc.path == tab.path) {
                if tab.draft.is_none() {
                    continue;
                }
                tab.title = format!("Recovered — {}", tab.title);
                tab.path = None;
            }
            let dirty = tab.draft.is_some();
            let (text, tracked) = match (&tab.path, tab.draft) {
                (Some(path), Some(draft)) => {
                    let mut tracked = TrackedFile::new(tab.expected_disk, None);
                    tracked.manual_save = tab.manual_save;
                    let _ = tracked.observe(path, true);
                    (draft, Some(tracked))
                }
                (Some(path), None) => {
                    let Ok((text, stamp)) = storage::read_document(path) else {
                        continue;
                    };
                    let tracked =
                        TrackedFile::new(Some(storage::fingerprint(text.as_bytes())), Some(stamp));
                    (text, Some(tracked))
                }
                (None, draft) => (draft.unwrap_or_default(), None),
            };
            let id = self.alloc_doc_id();
            if let Some(tracked) = tracked {
                self.disk.insert(id.clone(), tracked);
            }
            let mut doc = Document::new(id, tab.title, tab.path, &text);
            doc.dirty = dirty;
            if session.active == Some(index) {
                self.active_tab = Some(self.docs.len());
            }
            self.docs.push(doc);
        }
        if self.active_tab.is_none() && !self.docs.is_empty() {
            self.active_tab = Some(0);
        }
    }

    fn schedule_recovery(&mut self) {
        self.recovery_at
            .get_or_insert_with(|| Instant::now() + Duration::from_millis(750));
    }

    fn session(&self, include_drafts: bool) -> Session {
        Session {
            active: self.active_tab,
            tabs: self
                .docs
                .iter()
                .map(|doc| {
                    let tracked = self.disk.get(&doc.id);
                    let draft = (doc.dirty && include_drafts).then(|| {
                        let mut bytes = Vec::new();
                        doc.write_text(&mut bytes)
                            .expect("Write document to memory");
                        String::from_utf8(bytes).expect("Document encodes as UTF-8")
                    });
                    SessionTab {
                        path: doc.path.clone(),
                        title: doc.title.clone(),
                        draft,
                        expected_disk: tracked.and_then(|file| file.expected),
                        manual_save: tracked.is_some_and(|file| file.manual_save),
                    }
                })
                .collect(),
        }
    }

    fn persist_session(&self, include_drafts: bool) -> std::io::Result<()> {
        if let Some(path) = &self.session_path {
            storage::atomic_json(path, &self.session(include_drafts))?;
        }
        Ok(())
    }

    pub fn next_maintenance(&self) -> Option<Instant> {
        [
            self.next_autosave(),
            self.recovery_at,
            (self.initialized && !self.disk.is_empty()).then_some(self.scan_at),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    pub fn run_maintenance(&mut self, now: Instant) -> Vec<Value> {
        let mut events = self.run_autosaves(now);
        if self.recovery_at.is_some_and(|at| at <= now) {
            self.recovery_at = None;
            if let Err(error) = self.persist_session(true) {
                events.push(error_toast(format!(
                    "Cannot save session recovery: {error}"
                )));
                self.recovery_at = Some(now + Duration::from_secs(5));
            }
        }
        if self.initialized && now >= self.scan_at {
            self.scan_at = now + Duration::from_secs(1);
            if let Some(change) = self.poll_external_change() {
                events.push(change);
            }
        }
        events
    }

    fn poll_external_change(&mut self) -> Option<Value> {
        if self.pending_close.is_some() {
            return None;
        }
        let doc = self.docs.get(self.active_tab?)?;
        let file = self.disk.get_mut(&doc.id)?;
        file.observe(doc.path.as_deref()?, false).ok()?;
        let change = file.changed?;
        if file.prompted {
            return None;
        }
        file.prompted = true;
        Some(
            json!({ "event": "externalChange", "docId": doc.id, "title": doc.title, "missing": change.is_none() }),
        )
    }

    fn resolve_external_change(&mut self, doc_id: &str, reload: bool) -> Result<Value, String> {
        let idx = self
            .active_tab
            .filter(|&idx| self.docs[idx].id == doc_id)
            .ok_or_else(|| "The document is no longer active".to_owned())?;
        let doc = &mut self.docs[idx];
        let path = doc
            .path
            .clone()
            .ok_or_else(|| "The document has no path".to_owned())?;
        let file = self
            .disk
            .get_mut(doc_id)
            .ok_or_else(|| "The document is no longer tracked".to_owned())?;
        if reload {
            let (text, stamp) = storage::read_document(&path)
                .map_err(|error| format!("Cannot reload document: {error}"))?;
            *file = TrackedFile::new(Some(storage::fingerprint(text.as_bytes())), Some(stamp));
            let mut loaded = Document::new(doc.id.clone(), doc.title.clone(), Some(path), &text);
            loaded.version = doc.version + 1;
            *doc = loaded;
        } else {
            file.observe(&path, true)
                .map_err(|error| format!("Cannot check disk version: {error}"))?;
            if let Some(found) = file.changed.take() {
                file.expected = found;
                file.manual_save = true;
                doc.dirty = true;
            }
            file.prompted = false;
        }
        self.autosaves.remove(doc_id);
        self.schedule_recovery();
        if reload {
            let doc = self.active_doc_snapshot(200);
            Ok(
                json!({ "event": "tabSwitched", "tabs": self.tab_summaries(), "activeTabId": doc_id, "doc": doc }),
            )
        } else {
            Ok(json!({ "event": "tabsUpdated", "tabs": self.tab_summaries() }))
        }
    }

    pub fn sync_wallpaper_now(&mut self) {
        self.wp_error = None;
        let Some(wp_str) = self.settings.wallpaper.as_deref() else {
            self.prepared_wp = None;
            return;
        };
        let path = Path::new(wp_str);
        if !path.is_file() {
            self.prepared_wp = None;
            return;
        }
        let Some(cfg) = settings::config_dir() else {
            return;
        };
        match store::prepare(
            path,
            self.settings.wallpaper_effect,
            self.settings.light,
            &cfg,
        ) {
            Ok(prep) => {
                self.prepared_wp = Some(prep);
            }
            Err(e) => {
                self.wp_error = Some(e);
                self.prepared_wp = None;
            }
        }
    }

    pub fn resolved_theme(&self) -> ResolvedTheme {
        let mut palette = Palette::base(self.settings.theme_preset, self.settings.light);
        if self.settings.wallpaper_theme_colors
            && let Some(accent) = self.prepared_wp.as_ref().and_then(|p| p.accent)
        {
            palette = palette.tinted(accent);
        }
        let backdrop = self.prepared_wp.as_ref().map(|p| p.backdrop.as_slice());
        palette.resolve(
            backdrop,
            self.settings.wallpaper_fills_window && self.prepared_wp.is_some(),
            self.settings.wallpaper_hero_opacity,
        )
    }

    pub fn wallpaper_payload(&self) -> WallpaperStatePayload {
        let cfg = settings::config_dir();
        let current = self.settings.wallpaper.as_deref();
        let mut tiles = Vec::new();

        for b in &BUILTINS {
            let b_path = cfg
                .as_ref()
                .map(|c| gallery::builtin_path(c, b))
                .unwrap_or_else(|| PathBuf::from(format!("builtin:{}", b.id)));
            let selected = current.is_some_and(|c| Path::new(c) == b_path);
            let thumb_url = cfg
                .as_ref()
                .and_then(|c| gallery::builtin_thumbnail(c, b).ok())
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .map(|f| format!("/wallpaper/thumb/{f}"))
                .unwrap_or_else(|| format!("/wallpaper/builtin/{}.jpg", b.id));
            tiles.push(WallpaperTile {
                kind: "builtin",
                id: b.id.to_string(),
                title: b.title.to_string(),
                path: b_path.to_string_lossy().into_owned(),
                thumb_url,
                selected,
            });
        }

        for recent in &self.settings.wallpaper_history {
            let rp = Path::new(recent);
            if !rp.is_file() {
                continue;
            }
            let title = rp
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| recent.clone());
            let thumb_url = cfg
                .as_ref()
                .and_then(|c| gallery::file_thumbnail(c, rp).ok())
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .map(|f| format!("/wallpaper/thumb/{f}"))
                .unwrap_or_default();
            tiles.push(WallpaperTile {
                kind: "recent",
                id: recent.clone(),
                title,
                path: recent.clone(),
                thumb_url,
                selected: current == Some(recent.as_str()),
            });
        }

        let active_name = match current {
            Some(p) if Path::new(p).is_file() => Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.to_owned()),
            Some(_) => "File not found".to_string(),
            None => "None".to_string(),
        };

        let image_url = self.prepared_wp.as_ref().and_then(|p| {
            p.image
                .file_name()
                .map(|n| format!("/wallpaper/cache/{}", n.to_string_lossy()))
        });
        let blurred_url = self.prepared_wp.as_ref().and_then(|p| {
            p.blurred
                .file_name()
                .map(|n| format!("/wallpaper/cache/{}", n.to_string_lossy()))
        });

        WallpaperStatePayload {
            active_name,
            active_path: current.map(str::to_owned),
            image_url,
            blurred_url,
            effect: self.settings.wallpaper_effect,
            hero_opacity: self.settings.wallpaper_hero_opacity,
            fills_window: self.settings.wallpaper_fills_window,
            theme_colors: self.settings.wallpaper_theme_colors,
            error: self.wp_error.clone(),
            tiles,
            theme: self.resolved_theme(),
        }
    }

    pub fn active_doc_snapshot(&mut self, max_initial_blocks: usize) -> Value {
        let split = self.settings.view_mode == ViewMode::Split;
        let tex_main = self.active_latex_main();
        let Some(doc) = self.active_doc_mut() else {
            return Value::Null;
        };
        let large = doc.is_large();
        let tex_source =
            latex::project_source(doc.path.as_deref().unwrap_or_else(|| Path::new(&doc.title)));
        let total_blocks = doc.blocks.len();
        let deferred = large && !split;
        let block_count = if deferred || tex_source {
            0
        } else if large {
            max_initial_blocks.min(total_blocks)
        } else {
            total_blocks
        };
        let blocks: Vec<RenderedBlock> = doc.render_range(0, block_count, true);
        let outline = doc.outline();
        let stats = doc.stats();
        let source = source_window(doc, 0, if large { 600 } else { doc.rope.len_lines() });
        let text = &source["text"];
        json!({
            "id": doc.id,
            "title": doc.title,
            "path": doc.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
            "dirty": doc.dirty,
            "version": doc.version,
            "largeMode": large,
            "windowStartLine": 0,
            "windowEndLine": source["endLine"],
            "contextLine": source["contextLine"],
            "text": text,
            "blocks": blocks,
            "previewDeferred": deferred,
            "texSource": tex_source,
            "texMain": tex_main,
            "totalBlocks": total_blocks,
            "outline": outline,
            "stats": stats,
        })
    }

    pub fn scan_folder_tree(&self) -> Option<FileNode> {
        let root = self.folder_root.as_deref()?;
        if !root.is_dir() {
            return None;
        }
        Some(build_file_tree(root, 0))
    }

    pub fn save_latex_sources(&mut self, main: &Path) -> Result<(), String> {
        let directory = main
            .parent()
            .ok_or_else(|| "The main file has no project folder".to_owned())?;
        for index in 0..self.docs.len() {
            let document = &self.docs[index];
            if document.dirty
                && document.path.as_deref().is_some_and(|path| {
                    path == main
                        || (latex::project_source(path)
                            && (path.starts_with(directory)
                                || latex::detect_root(path, self.folder_root.as_deref())
                                    .as_deref()
                                    == Some(main)))
                })
            {
                self.save_document(index, false, true)?;
                self.schedule_recovery();
            }
        }
        Ok(())
    }

    fn workspace_action(
        &mut self,
        action: WorkspaceAction,
        path: Option<PathBuf>,
        name: Option<String>,
    ) -> Result<Value, String> {
        let root = self
            .folder_root
            .as_deref()
            .ok_or_else(|| "Open a workspace folder first".to_owned())?;
        let root = dunce::canonicalize(root)
            .map_err(|error| format!("Cannot locate workspace: {error}"))?;
        let path = workspace::existing_inside(&root, path.as_deref().unwrap_or(&root))?;
        match action {
            WorkspaceAction::Reveal => {
                workspace::reveal(&path)?;
                return Ok(json!({ "event": "noop" }));
            }
            WorkspaceAction::NewDocument | WorkspaceAction::NewFolder => {
                let folder = if path.is_dir() {
                    path.as_path()
                } else {
                    path.parent().unwrap_or(&root)
                };
                let document = matches!(action, WorkspaceAction::NewDocument);
                let target =
                    workspace::child_path(folder, name.as_deref().unwrap_or_default(), document)?;
                if document {
                    workspace::create_document(&target)
                        .map_err(|error| format!("Cannot create document: {error}"))?;
                    self.open_file_path(&target)?;
                    let doc = self.active_doc_snapshot(200);
                    return Ok(
                        json!({"event":"tabSwitched","doc":doc,"tabs":self.tab_summaries(),
                        "activeTabId":self.active_doc().map(|doc|&doc.id),"fileTree":self.scan_folder_tree()}),
                    );
                }
                std::fs::create_dir(target)
                    .map_err(|error| format!("Cannot create folder: {error}"))?;
            }
            WorkspaceAction::Rename | WorkspaceAction::Trash => {
                let parent = path
                    .parent()
                    .filter(|_| path != root)
                    .ok_or_else(|| "The workspace root is open".to_owned())?;
                if matches!(action, WorkspaceAction::Trash) {
                    trash::delete(&path)
                        .map_err(|error| format!("Cannot move item to Trash: {error}"))?;
                    self.scan_at = Instant::now();
                } else {
                    let target = workspace::child_path(
                        parent,
                        name.as_deref().unwrap_or_default(),
                        path.is_file(),
                    )?;
                    if target != path {
                        workspace::rename_item(&path, &target)?;
                        for doc in &mut self.docs {
                            if let Some(relative) = doc
                                .path
                                .as_deref()
                                .and_then(|current| current.strip_prefix(&path).ok())
                            {
                                let moved = if relative.as_os_str().is_empty() {
                                    target.clone()
                                } else {
                                    target.join(relative)
                                };
                                doc.title = moved
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned();
                                doc.set_path(moved);
                                if let Some(file) = self.disk.get_mut(&doc.id) {
                                    file.stamp = None;
                                }
                            }
                        }
                        for recent in &mut self.settings.recent_files {
                            if let Ok(relative) = Path::new(recent).strip_prefix(&path) {
                                *recent = if relative.as_os_str().is_empty() {
                                    target.clone()
                                } else {
                                    target.join(relative)
                                }
                                .to_string_lossy()
                                .into_owned();
                            }
                        }
                        let _ = settings::save(&self.settings);
                    }
                }
            }
        }
        Ok(
            json!({ "event": "tabsUpdated", "tabs": self.tab_summaries(), "fileTree": self.scan_folder_tree() }),
        )
    }

    pub fn handle_command(&mut self, raw: &str) -> Value {
        let Ok(cmd) = serde_json::from_str::<IpcCommand>(raw) else {
            return json!({ "event": "error", "message": "Invalid IPC payload" });
        };

        let before = self.active_doc().map(|doc| (doc.id.clone(), doc.version));
        let response = self.handle(cmd);
        if matches!(
            response["event"].as_str(),
            Some("patch" | "tabSwitched" | "saved" | "tabsUpdated" | "clipboardImageSaved")
        ) {
            self.schedule_recovery();
        }
        if let Some((id, version)) = before
            && let Some(doc) = self.active_doc()
            && doc.id == id
            && doc.version != version
            && doc.path.is_some()
            && self.settings.auto_save
        {
            self.autosaves
                .insert(id, Instant::now() + Duration::from_millis(1500));
        }
        response
    }

    pub fn next_autosave(&self) -> Option<Instant> {
        self.autosaves
            .iter()
            .filter(|(id, _)| !self.closing_document(id))
            .map(|(_, at)| *at)
            .min()
    }

    pub fn run_autosaves(&mut self, now: Instant) -> Vec<Value> {
        let due: Vec<_> = self
            .autosaves
            .iter()
            .filter(|(id, at)| **at <= now && !self.closing_document(id))
            .map(|(id, _)| id.clone())
            .collect();
        let mut responses = Vec::new();
        for id in due {
            self.autosaves.remove(&id);
            if let Some(idx) = self
                .docs
                .iter()
                .position(|doc| doc.id == id && doc.dirty && doc.path.is_some())
            {
                match self.save_document(idx, false, false) {
                    Ok(Some(changed_path)) => {
                        responses.push(self.saved_response(idx, changed_path));
                        self.schedule_recovery();
                    }
                    Ok(None) => {}
                    Err(e) => responses.push(error_toast(e)),
                }
            }
        }
        responses
    }

    // Keep autosave from writing a document while the user is choosing whether to discard it.
    fn closing_document(&self, id: &str) -> bool {
        match self.pending_close.as_ref().map(|close| &close.target) {
            Some(CloseTarget::Window) => true,
            Some(CloseTarget::Tab(tab)) => tab == id,
            None => false,
        }
    }

    pub fn request_window_close(&mut self) -> Value {
        self.begin_close(CloseTarget::Window)
    }

    fn begin_close(&mut self, target: CloseTarget) -> Value {
        if self.pending_close.is_some() {
            return json!({ "event": "noop" });
        }
        let documents = self
            .docs
            .iter()
            .filter(|doc| {
                doc.dirty
                    && match &target {
                        CloseTarget::Tab(id) => &doc.id == id,
                        CloseTarget::Window => true,
                    }
            })
            .map(|doc| doc.id.clone())
            .collect();
        self.pending_close = Some(CloseRequest { target, documents });
        self.advance_close()
    }

    fn advance_close(&mut self) -> Value {
        let Some(request) = &mut self.pending_close else {
            return json!({ "event": "noop" });
        };
        while let Some(id) = request.documents.front() {
            if let Some(doc) = self.docs.iter().find(|doc| &doc.id == id && doc.dirty) {
                return json!({
                    "event": "confirmClose",
                    "docId": doc.id,
                    "title": doc.title,
                    "tabs": self.tab_summaries(),
                });
            }
            request.documents.pop_front();
        }
        match self.pending_close.take().unwrap().target {
            CloseTarget::Window => match self.persist_session(false) {
                Ok(()) => json!({ "event": "exitRequested" }),
                Err(error) => error_toast(format!("Cannot finalize session: {error}")),
            },
            CloseTarget::Tab(id) => self.finish_close_tab(&id),
        }
    }

    fn resolve_close(&mut self, id: &str, choice: CloseChoice) -> Value {
        let Some(next) = self
            .pending_close
            .as_ref()
            .and_then(|close| close.documents.front())
        else {
            return json!({ "event": "noop" });
        };
        if next != id {
            return json!({ "event": "noop" });
        }
        match choice {
            CloseChoice::Cancel => return self.cancel_close(None),
            CloseChoice::Save => {
                if let Some(idx) = self.docs.iter().position(|doc| doc.id == id && doc.dirty) {
                    match self.save_document(idx, false, true) {
                        Ok(Some(_)) => {}
                        Ok(None) => return self.cancel_close(None),
                        Err(error) => return self.cancel_close(Some(error)),
                    }
                }
            }
            CloseChoice::Discard => {}
        }
        self.pending_close.as_mut().unwrap().documents.pop_front();
        self.advance_close()
    }

    fn cancel_close(&mut self, message: Option<String>) -> Value {
        self.pending_close = None;
        json!({
            "event": "tabsUpdated",
            "tabs": self.tab_summaries(),
            "toast": message,
        })
    }

    fn finish_close_tab(&mut self, id: &str) -> Value {
        let Some(idx) = self.docs.iter().position(|doc| doc.id == id) else {
            return json!({ "event": "noop" });
        };
        let closing_active = self.active_tab == Some(idx);
        self.autosaves.remove(id);
        self.disk.remove(id);
        self.docs.remove(idx);
        self.active_tab = match self.active_tab {
            _ if self.docs.is_empty() => None,
            Some(active) if active < idx => Some(active),
            Some(active) if active > idx => Some(active - 1),
            _ => Some(idx.min(self.docs.len() - 1)),
        };
        if !closing_active {
            return json!({ "event": "tabsUpdated", "tabs": self.tab_summaries() });
        }
        let doc = self.active_doc_snapshot(200);
        json!({
            "event": "tabSwitched",
            "tabs": self.tab_summaries(),
            "activeTabId": self.active_doc().map(|d| d.id.clone()),
            "doc": doc,
        })
    }

    fn save_document(
        &mut self,
        idx: usize,
        save_as: bool,
        allow_dialog: bool,
    ) -> Result<Option<bool>, String> {
        let doc = &self.docs[idx];
        let target = if save_as || doc.path.is_none() {
            if !allow_dialog {
                return Ok(None);
            }
            let mut dialog = rfd::FileDialog::new().set_file_name(&doc.title).add_filter(
                "Markdown and LaTeX",
                &["md", "markdown", "tex", "ltx", "bib", "sty", "cls"],
            );
            if let Some(dir) = doc.path.as_deref().and_then(Path::parent) {
                dialog = dialog.set_directory(dir);
            }
            dialog.save_file()
        } else {
            doc.path.clone()
        };
        let Some(target) = target else {
            return Ok(None);
        };
        let target = dunce::canonicalize(&target).unwrap_or(target);
        if self
            .docs
            .iter()
            .enumerate()
            .any(|(i, doc)| i != idx && doc.path.as_ref() == Some(&target))
        {
            return Err("Save failed: another tab already has this file open".into());
        }
        let track = (doc.path.as_ref() == Some(&target))
            .then(|| self.disk.get(&doc.id))
            .flatten();
        if track.is_some_and(|file| file.manual_save) && !allow_dialog {
            return Ok(None);
        }
        let expected = track.map(|file| file.expected);
        let mut hasher = blake3::Hasher::new();
        doc.write_text(&mut hasher)
            .map_err(|error| format!("Cannot encode document: {error}"))?;
        let saved = Some(*hasher.finalize().as_bytes());
        let mut conflict = false;
        let write = storage::atomic_write_checked(
            &target,
            |file| doc.write_text(file),
            || {
                if let Some(expected) = expected
                    && storage::disk_fingerprint(&target)? != expected
                {
                    conflict = true;
                    return Err(std::io::Error::other("the file changed on disk"));
                }
                Ok(())
            },
        );
        if let Err(error) = write {
            if conflict {
                if let Some(tracked) = self.disk.get_mut(&doc.id) {
                    let _ = tracked.observe(&target, true);
                }
                self.scan_at = Instant::now();
            }
            return Err(format!("Save failed: {error}"));
        }
        let path = dunce::canonicalize(&target).unwrap_or(target);
        let doc = &mut self.docs[idx];
        self.disk.insert(
            doc.id.clone(),
            TrackedFile::new(saved, FileStamp::read(&path).ok().flatten()),
        );
        let changed_path = doc.set_path(path.clone());
        if let Some(name) = path.file_name() {
            doc.title = name.to_string_lossy().into_owned();
        }
        doc.dirty = false;
        self.autosaves.remove(&doc.id);
        if changed_path {
            self.settings.remember_recent_file(&path.to_string_lossy());
            let _ = settings::save(&self.settings);
        }
        Ok(Some(changed_path))
    }

    fn saved_response(&self, idx: usize, changed_path: bool) -> Value {
        let doc = &self.docs[idx];
        let mut response = json!({
            "event": "saved",
            "docId": doc.id,
            "tabs": self.tab_summaries(),
            "toast": format!("Saved {}", doc.title),
        });
        if changed_path {
            response["fileTree"] = json!(self.scan_folder_tree());
        }
        response
    }

    fn handle(&mut self, cmd: IpcCommand) -> Value {
        match cmd {
            IpcCommand::Init => {
                self.initialized = true;
                let doc = self.active_doc_snapshot(200);
                let tabs = self.tab_summaries();
                let wp = self.wallpaper_payload();
                let tree = self.scan_folder_tree();
                json!({
                    "event": "init",
                    "version": env!("CARGO_PKG_VERSION"),
                    "pandocAvailable": exports::pandoc().is_some(),
                    "platform": std::env::consts::OS,
                    "settings": self.settings,
                    "tabs": tabs,
                    "activeTabId": self.active_doc().map(|d| d.id.clone()),
                    "doc": doc,
                    "wallpaper": wp,
                    "fileTree": tree,
                })
            }

            IpcCommand::Edit {
                doc_id,
                base_line,
                start_char,
                end_char,
                text,
                source,
                utf16,
                defer_preview,
            } => {
                let Some(doc) = self.active_doc_mut().filter(|doc| doc_id == doc.id) else {
                    return resync_response(&doc_id);
                };
                let Ok(base_offset) = doc.rope.try_line_to_char(base_line) else {
                    return resync_response(&doc_id);
                };
                if start_char > end_char {
                    return resync_response(&doc_id);
                }
                let offset = |relative| {
                    if utf16 {
                        let units = doc
                            .rope
                            .char_to_utf16_cu(base_offset)
                            .checked_add(relative)?;
                        let point = doc.rope.try_utf16_cu_to_char(units).ok()?;
                        (doc.rope.char_to_utf16_cu(point) == units).then_some(point)
                    } else {
                        base_offset
                            .checked_add(relative)
                            .filter(|&point| point <= doc.rope.len_chars())
                    }
                };
                let (Some(start), Some(end)) = (offset(start_char), offset(end_char)) else {
                    return resync_response(&doc_id);
                };
                let defer_preview = defer_preview.unwrap_or(utf16);
                let patch = doc.apply_edit_with_preview(
                    start,
                    end,
                    &text,
                    (!defer_preview).then_some(true),
                );
                let mut response = patch_response(doc, patch, source);
                response["userEdit"] = json!(true);
                if defer_preview {
                    response["deferredPreview"] = json!(true);
                }
                response
            }

            IpcCommand::ToggleTask {
                doc_id,
                version,
                block_id,
                line_offset,
                source,
            } => {
                let Some(doc) = self
                    .active_doc_mut()
                    .filter(|doc| doc_id == doc.id && version == doc.version)
                else {
                    return resync_response(&doc_id);
                };
                let line = doc.blocks.iter().find(|b| b.id == block_id).and_then(|b| {
                    b.start_line
                        .checked_add(line_offset)
                        .filter(|&line| line < b.end_line)
                });
                match line.and_then(|line| doc.toggle_task_line(line)) {
                    Some(patch) => patch_response(doc, patch, Some(source)),
                    None => resync_response(&doc_id),
                }
            }

            IpcCommand::RenderLiveWidgets {
                doc_id,
                request,
                fragments,
            } => {
                let html = self
                    .active_doc()
                    .filter(|doc| doc.id == doc_id)
                    .map(|doc| {
                        fragments
                            .iter()
                            .take(16)
                            .map(|source| doc.render_live_fragment(source))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                json!({ "event": "liveWidgetsRendered", "request": request, "html": html })
            }

            IpcCommand::FetchBlocks {
                doc_id,
                start,
                count,
                request,
                refresh,
                around_line,
            } => {
                let Some(doc) = self.active_doc_mut().filter(|doc| doc_id == doc.id) else {
                    return resync_response(&doc_id);
                };
                let start = around_line.map_or(start, |line| {
                    doc.blocks
                        .partition_point(|block| block.end_line <= line)
                        .saturating_sub(35)
                });
                let blocks = doc.render_range(start, count, true);
                json!({
                    "event": "blocksLoaded",
                    "docId": doc.id,
                    "version": doc.version,
                    "request": request,
                    "refresh": refresh,
                    "start": start,
                    "blocks": blocks,
                })
            }

            IpcCommand::FetchLines {
                doc_id,
                start_line,
                count,
                request,
            } => {
                let Some(doc) = self.active_doc().filter(|doc| doc_id == doc.id) else {
                    return json!({ "event": "noop" });
                };
                json!({
                    "event": "linesLoaded",
                    "docId": doc.id,
                    "version": doc.version,
                    "request": request,
                    "source": source_window(doc, start_line, count.min(1000)),
                })
            }

            IpcCommand::NewTab => {
                self.new_untitled_doc();
                let doc = self.active_doc_snapshot(200);
                json!({
                    "event": "tabSwitched",
                    "tabs": self.tab_summaries(),
                    "activeTabId": self.active_doc().map(|d| d.id.clone()),
                    "doc": doc,
                })
            }

            IpcCommand::SwitchTab { id } => {
                if let Some(idx) = self.docs.iter().position(|d| d.id == id) {
                    self.active_tab = Some(idx);
                }
                let doc = self.active_doc_snapshot(200);
                json!({
                    "event": "tabSwitched",
                    "tabs": self.tab_summaries(),
                    "activeTabId": self.active_doc().map(|d| d.id.clone()),
                    "doc": doc,
                })
            }

            IpcCommand::CloseTab { id } => self.begin_close(CloseTarget::Tab(id)),

            IpcCommand::ResolveClose { doc_id, choice } => self.resolve_close(&doc_id, choice),

            IpcCommand::OpenFileDialog => {
                let picked = rfd::FileDialog::new()
                    .add_filter(
                        "Markdown and LaTeX",
                        &[
                            "md", "markdown", "mdown", "mkd", "txt", "tex", "ltx", "bib", "sty",
                            "cls", "bst", "bbl",
                        ],
                    )
                    .add_filter("Markdown", &["md", "markdown", "mdown", "mkd", "txt"])
                    .add_filter(
                        "LaTeX project files",
                        &["tex", "ltx", "bib", "sty", "cls", "bst", "bbl"],
                    )
                    .add_filter("All Files", &["*"])
                    .pick_file();
                if let Some(path) = picked {
                    let t0 = Instant::now();
                    match self.open_file_path(&path) {
                        Ok(_) => {
                            let ms = t0.elapsed().as_millis();
                            let doc = self.active_doc_snapshot(200);
                            json!({
                                "event": "tabSwitched",
                                "tabs": self.tab_summaries(),
                                "activeTabId": self.active_doc().map(|d| d.id.clone()),
                                "doc": doc,
                                "settings": self.settings,
                                "toast": format!("Opened {} in {} ms", path.file_name().unwrap_or_default().to_string_lossy(), ms),
                            })
                        }
                        Err(e) => json!({ "event": "toast", "message": e, "kind": "error" }),
                    }
                } else {
                    json!({ "event": "noop" })
                }
            }

            IpcCommand::OpenPath { path } => {
                let p = PathBuf::from(&path);
                let allowed = dunce::canonicalize(&p).ok().is_some_and(|canonical| {
                    self.folder_root
                        .as_deref()
                        .and_then(|root| dunce::canonicalize(root).ok())
                        .is_some_and(|root| canonical.starts_with(&root))
                        || self.docs.iter().any(|d| {
                            d.path.as_deref() == Some(canonical.as_path())
                                || d.path.as_deref().and_then(Path::parent).is_some_and(|dir| {
                                    latex::project_source(&canonical) && canonical.starts_with(dir)
                                })
                        })
                        || self.settings.recent_files.iter().any(|recent| {
                            dunce::canonicalize(recent)
                                .ok()
                                .is_some_and(|r| r == canonical)
                        })
                });
                if !allowed {
                    return json!({ "event": "toast", "message": "Cannot open file outside the workspace or recent files", "kind": "error" });
                }
                let t0 = Instant::now();
                match self.open_file_path(&p) {
                    Ok(_) => {
                        let ms = t0.elapsed().as_millis();
                        let doc = self.active_doc_snapshot(200);
                        json!({
                            "event": "tabSwitched",
                            "tabs": self.tab_summaries(),
                            "activeTabId": self.active_doc().map(|d| d.id.clone()),
                            "doc": doc,
                            "settings": self.settings,
                            "toast": format!("Opened in {} ms", ms),
                        })
                    }
                    Err(e) => json!({ "event": "toast", "message": e, "kind": "error" }),
                }
            }

            IpcCommand::OpenFolderDialog => {
                if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                    self.folder_root = Some(dir.clone());
                    self.settings.last_folder = Some(dir.to_string_lossy().into_owned());
                    let _ = settings::save(&self.settings);
                    let tree = self.scan_folder_tree();
                    json!({
                        "event": "fileTree",
                        "fileTree": tree,
                    })
                } else {
                    json!({ "event": "noop" })
                }
            }

            IpcCommand::WorkspaceAction { action, path, name } => self
                .workspace_action(action, path, name)
                .unwrap_or_else(error_toast),

            IpcCommand::SaveClipboardImage {
                doc_id,
                request,
                encoded,
            } => {
                let result = (|| {
                    let idx = self
                        .active_tab
                        .filter(|&idx| self.docs[idx].id == doc_id)
                        .ok_or_else(|| "The document is no longer active".to_owned())?;
                    if self.docs[idx].path.is_none()
                        && self.save_document(idx, false, true)?.is_none()
                    {
                        return Ok(None);
                    }
                    let folder = self.docs[idx]
                        .path
                        .as_deref()
                        .and_then(Path::parent)
                        .ok_or_else(|| "The document has no folder".to_owned())?;
                    assets::save_clipboard_image(folder, &encoded).map(Some)
                })();
                let (path, error) = match result {
                    Ok(path) => (path, None),
                    Err(error) => (None, Some(error)),
                };
                json!({ "event": "clipboardImageSaved", "docId": doc_id, "request": request,
                    "path": path, "error": error, "tabs": self.tab_summaries() })
            }

            IpcCommand::ResolveExternalChange { doc_id, reload } => {
                match self.resolve_external_change(&doc_id, reload) {
                    Ok(response) => response,
                    Err(error) => {
                        if let Some(file) = self.disk.get_mut(&doc_id) {
                            file.prompted = false;
                        }
                        error_toast(error)
                    }
                }
            }

            IpcCommand::Save { save_as } => {
                let Some(idx) = self.active_tab else {
                    return json!({ "event": "noop" });
                };
                match self.save_document(idx, save_as, true) {
                    Ok(Some(changed_path)) => self.saved_response(idx, changed_path),
                    Ok(None) => json!({ "event": "noop" }),
                    Err(e) => error_toast(e),
                }
            }

            IpcCommand::Print { doc_id } => {
                let Some(doc) = self.active_doc_mut().filter(|doc| doc_id == doc.id) else {
                    return resync_response(&doc_id);
                };
                json!({ "event": "printDocument", "docId": doc.id, "html": doc.render_full_html() })
            }

            IpcCommand::ExportHtml { inline_images } => {
                let Some(doc) = self.active_doc_mut() else {
                    return json!({ "event": "noop" });
                };
                let default_name = format!("{}.html", doc.title.trim_end_matches(".md"));
                let Some(dest) = rfd::FileDialog::new()
                    .set_file_name(&default_name)
                    .add_filter("HTML Document", &["html", "htm"])
                    .save_file()
                else {
                    return json!({ "event": "noop" });
                };
                let title = doc.title.clone();
                let body = doc.render_full_html();
                let base = dest
                    .parent()
                    .and_then(|path| dunce::canonicalize(path).ok());
                let link_base = (!inline_images.unwrap_or(true))
                    .then_some(base.as_deref())
                    .flatten();
                let html = assets::export_html(&title, &body, self.settings.light, link_base);
                match storage::atomic_write(&dest, |file| {
                    use std::io::Write;
                    file.write_all(html.as_bytes())
                }) {
                    Ok(()) => json!({
                        "event": "toast",
                        "message": format!("Exported HTML to {}", dest.file_name().unwrap_or_default().to_string_lossy()),
                    }),
                    Err(e) => {
                        json!({ "event": "toast", "message": format!("Export failed: {e}"), "kind": "error" })
                    }
                }
            }

            IpcCommand::ExportPandoc { format } => {
                let Some(doc) = self.active_doc() else {
                    return json!({"event":"noop"});
                };
                let Some(binary) = exports::pandoc() else {
                    return error_toast("Pandoc is not installed or is missing from PATH".into());
                };
                let extension = format.extension();
                let name = Path::new(&doc.title).with_extension(extension);
                let mut dialog = rfd::FileDialog::new()
                    .set_file_name(name.to_string_lossy())
                    .add_filter(extension, &[extension]);
                if let Some(folder) = doc.path.as_deref().and_then(Path::parent) {
                    dialog = dialog.set_directory(folder);
                }
                let Some(dest) = dialog.save_file() else {
                    return json!({"event":"noop"});
                };
                match exports::convert_document(&binary, doc, &dest, format) {
                    Ok(()) => {
                        json!({"event":"toast","message":format!("Exported {}",dest.file_name().unwrap_or_default().to_string_lossy())})
                    }
                    Err(error) => error_toast(error),
                }
            }

            IpcCommand::Search {
                doc_id,
                request,
                query,
                case_sensitive,
                use_regex,
            } => {
                let Some(doc) = self.active_doc().filter(|doc| doc_id == doc.id) else {
                    return json!({ "event": "noop" });
                };
                let matches = doc.search(&query, case_sensitive, use_regex, 500);
                json!({
                    "event": "searchResults",
                    "docId": doc.id,
                    "version": doc.version,
                    "request": request,
                    "matches": matches,
                })
            }

            IpcCommand::ReplaceOne {
                doc_id,
                version,
                query,
                replacement,
                case_sensitive,
                use_regex,
                start_char,
                end_char,
                source,
            } => {
                let Some(doc) = self
                    .active_doc_mut()
                    .filter(|doc| doc_id == doc.id && version == doc.version)
                else {
                    return resync_response(&doc_id);
                };
                let search = SearchQuery {
                    query: &query,
                    case_sensitive,
                    use_regex,
                };
                match doc.replace_match(start_char, end_char, &search, &replacement) {
                    Some(patch) => patch_response(doc, patch, Some(source)),
                    None => resync_response(&doc_id),
                }
            }

            IpcCommand::ReplaceAll {
                doc_id,
                query,
                replacement,
                case_sensitive,
                use_regex,
            } => {
                let Some(doc) = self.active_doc_mut().filter(|doc| doc_id == doc.id) else {
                    return resync_response(&doc_id);
                };
                let count = doc.replace_all(&query, &replacement, case_sensitive, use_regex);
                let snap = self.active_doc_snapshot(200);
                json!({
                    "event": "tabSwitched",
                    "tabs": self.tab_summaries(),
                    "activeTabId": self.active_doc().map(|d| d.id.clone()),
                    "doc": snap,
                    "toast": format!("Replaced {count} occurrences"),
                })
            }

            IpcCommand::UpdateSettings { patch } => {
                let mut need_wp = false;
                if let Some(v) = patch.language {
                    self.settings.language = v;
                }
                if let Some(v) = patch.light
                    && self.settings.light != v
                {
                    self.settings.light = v;
                    need_wp = true;
                }
                if let Some(v) = patch.theme_preset {
                    self.settings.theme_preset = v;
                }
                if let Some(v) = patch.view_mode {
                    self.settings.view_mode = v;
                }
                if let Some(v) = patch.font_size {
                    self.settings.font_size = v;
                }
                if let Some(v) = patch.preview_max_width {
                    self.settings.preview_max_width = v;
                }
                if let Some(v) = patch.line_height {
                    self.settings.line_height = v;
                }
                if let Some(v) = patch.sync_scroll {
                    self.settings.sync_scroll = v;
                }
                if let Some(v) = patch.focus_mode {
                    self.settings.focus_mode = v;
                }
                if let Some(v) = patch.typewriter_mode {
                    self.settings.typewriter_mode = v;
                }
                if let Some(v) = patch.auto_save {
                    self.settings.auto_save = v;
                    self.autosaves.clear();
                    if v {
                        for doc in &self.docs {
                            if doc.dirty && doc.path.is_some() {
                                self.autosaves.insert(
                                    doc.id.clone(),
                                    Instant::now() + Duration::from_millis(1500),
                                );
                            }
                        }
                    }
                }
                if let Some(v) = patch.show_line_numbers {
                    self.settings.show_line_numbers = v;
                }
                if let Some(v) = patch.reduce_motion {
                    self.settings.reduce_motion = v;
                }
                if let Some(v) = patch.sidebar_open {
                    self.settings.sidebar_open = v;
                }
                if let Some(v) = patch.sidebar_tab {
                    self.settings.sidebar_tab = v;
                }
                if let Some(v) = patch.split_ratio {
                    self.settings.split_ratio = v;
                }
                if let Some(v) = patch.wallpaper_effect
                    && self.settings.wallpaper_effect != v
                {
                    self.settings.wallpaper_effect = v;
                    need_wp = true;
                }
                if let Some(v) = patch.wallpaper_hero_opacity {
                    self.settings.wallpaper_hero_opacity = v;
                }
                if let Some(v) = patch.wallpaper_fills_window {
                    self.settings.wallpaper_fills_window = v;
                }
                if let Some(v) = patch.wallpaper_theme_colors {
                    self.settings.wallpaper_theme_colors = v;
                }

                self.settings.clamp();
                let _ = settings::save(&self.settings);
                if need_wp {
                    self.sync_wallpaper_now();
                }
                let wp = self.wallpaper_payload();
                json!({
                    "event": "settingsUpdated",
                    "settings": self.settings,
                    "wallpaper": wp,
                })
            }

            IpcCommand::SelectBuiltinWallpaper { id } => {
                if let (Some(b), Some(cfg)) = (gallery::find_builtin(&id), settings::config_dir()) {
                    match gallery::install_builtin(&cfg, b) {
                        Ok(path) => {
                            gallery::use_builtin(
                                &mut self.settings,
                                path.to_string_lossy().into_owned(),
                            );
                            let _ = settings::save(&self.settings);
                            self.sync_wallpaper_now();
                        }
                        Err(e) => self.wp_error = Some(e),
                    }
                }
                json!({
                    "event": "wallpaperUpdated",
                    "settings": self.settings,
                    "wallpaper": self.wallpaper_payload(),
                })
            }

            IpcCommand::SelectRecentWallpaper { path } => {
                let allowed = self.settings.wallpaper_history.contains(&path)
                    && settings::config_dir().is_some_and(|cfg| {
                        let store_root = store::store_dir(&cfg);
                        dunce::canonicalize(&store_root)
                            .ok()
                            .zip(dunce::canonicalize(&path).ok())
                            .is_some_and(|(root, target)| {
                                target.starts_with(&root) && target.is_file()
                            })
                    });
                if allowed {
                    gallery::use_picked(&mut self.settings, path);
                    if let Some(cfg) = settings::config_dir() {
                        store::prune(&cfg, &self.settings.wallpaper_history);
                    }
                    let _ = settings::save(&self.settings);
                    self.sync_wallpaper_now();
                }
                json!({
                    "event": "wallpaperUpdated",
                    "settings": self.settings,
                    "wallpaper": self.wallpaper_payload(),
                })
            }

            IpcCommand::ClearWallpaper => {
                self.settings.wallpaper = None;
                self.prepared_wp = None;
                self.wp_error = None;
                let _ = settings::save(&self.settings);
                json!({
                    "event": "wallpaperUpdated",
                    "settings": self.settings,
                    "wallpaper": self.wallpaper_payload(),
                })
            }

            IpcCommand::PickWallpaperDialog => {
                let picked = rfd::FileDialog::new()
                    .set_title("Choose wallpaper")
                    .add_filter("Images", &["png", "jpg", "jpeg", "webp"])
                    .pick_file();
                if let (Some(src), Some(cfg)) = (picked, settings::config_dir()) {
                    match store::import(&src, &cfg) {
                        Ok(imported) => {
                            gallery::use_picked(
                                &mut self.settings,
                                imported.to_string_lossy().into_owned(),
                            );
                            store::prune(&cfg, &self.settings.wallpaper_history);
                            let _ = settings::save(&self.settings);
                            self.sync_wallpaper_now();
                        }
                        Err(e) => self.wp_error = Some(e),
                    }
                    json!({
                        "event": "wallpaperUpdated",
                        "settings": self.settings,
                        "wallpaper": self.wallpaper_payload(),
                    })
                } else {
                    json!({ "event": "noop" })
                }
            }
        }
    }
}

fn error_toast(message: String) -> Value {
    json!({ "event": "toast", "message": message, "kind": "error" })
}

fn resync_response(doc_id: &str) -> Value {
    json!({ "event": "resyncSource", "docId": doc_id })
}

fn patch_response(doc: &Document, patch: EditPatch, source: Option<SourceRequest>) -> Value {
    let source = source.map(|req| {
        source_window(
            doc,
            req.start_line,
            req.count.unwrap_or(doc.rope.len_lines()),
        )
    });
    json!({ "event": "patch", "docId": doc.id, "dirty": doc.dirty, "patch": patch, "source": source })
}

fn source_window(doc: &Document, start: usize, count: usize) -> Value {
    let total = doc.rope.len_lines();
    let start = start.min(total - 1);
    let end = start.saturating_add(count).min(total);
    let text = if end == total {
        doc.rope.slice(doc.rope.line_to_char(start)..).to_string()
    } else {
        doc.slice_lines(start, end)
    };
    let context_line = doc
        .blocks
        .get(doc.blocks.partition_point(|b| b.end_line <= start))
        .filter(|b| {
            b.start_line < start
                && matches!(
                    b.kind,
                    crate::engine::BlockKind::CodeFence
                        | crate::engine::BlockKind::MathBlock
                        | crate::engine::BlockKind::FrontMatter
                )
        })
        .map(|b| doc.line_str(b.start_line));
    json!({ "startLine": start, "endLine": end, "totalLines": total, "text": text, "contextLine": context_line })
}

fn build_file_tree(dir: &Path, depth: usize) -> FileNode {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.to_string_lossy().into_owned());
    let mut children = Vec::new();
    if depth < 4
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        let mut dirs = Vec::new();
        let mut files = Vec::new();
        for entry in entries.flatten() {
            let p = entry.path();
            let fname = entry.file_name().to_string_lossy().into_owned();
            if fname.starts_with('.') || fname == "target" || fname == "node_modules" {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                dirs.push(build_file_tree(&p, depth + 1));
            } else if kind.is_file() && workspace::document_file(&p) {
                files.push(FileNode {
                    name: fname,
                    path: p.to_string_lossy().into_owned(),
                    is_dir: false,
                    children: Vec::new(),
                });
            }
        }
        dirs.sort_by_key(|a| a.name.to_lowercase());
        files.sort_by_key(|a| a.name.to_lowercase());
        children.extend(dirs);
        children.extend(files);
    }
    FileNode {
        name,
        path: dir.to_string_lossy().into_owned(),
        is_dir: true,
        children,
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "cmd", rename_all = "camelCase", rename_all_fields = "camelCase")]
enum IpcCommand {
    Init,
    Edit {
        doc_id: String,
        #[serde(default)]
        base_line: usize,
        start_char: usize,
        end_char: usize,
        text: String,
        source: Option<SourceRequest>,
        #[serde(default)]
        utf16: bool,
        defer_preview: Option<bool>,
    },
    ToggleTask {
        doc_id: String,
        version: u64,
        block_id: u64,
        line_offset: usize,
        source: SourceRequest,
    },
    RenderLiveWidgets {
        doc_id: String,
        request: u64,
        fragments: Vec<String>,
    },
    FetchBlocks {
        doc_id: String,
        start: usize,
        count: usize,
        request: u64,
        refresh: bool,
        around_line: Option<usize>,
    },
    FetchLines {
        doc_id: String,
        start_line: usize,
        count: usize,
        request: u64,
    },
    NewTab,
    SwitchTab {
        id: String,
    },
    CloseTab {
        id: String,
    },
    ResolveClose {
        doc_id: String,
        choice: CloseChoice,
    },
    OpenFileDialog,
    OpenPath {
        path: String,
    },
    OpenFolderDialog,
    WorkspaceAction {
        action: WorkspaceAction,
        path: Option<PathBuf>,
        name: Option<String>,
    },
    Save {
        #[serde(rename = "saveAs")]
        save_as: bool,
    },
    ResolveExternalChange {
        doc_id: String,
        reload: bool,
    },
    SaveClipboardImage {
        doc_id: String,
        request: u64,
        encoded: String,
    },
    ExportHtml {
        inline_images: Option<bool>,
    },
    ExportPandoc {
        format: PandocFormat,
    },
    Print {
        doc_id: String,
    },
    Search {
        doc_id: String,
        request: u64,
        query: String,
        #[serde(rename = "caseSensitive")]
        case_sensitive: bool,
        #[serde(rename = "useRegex")]
        use_regex: bool,
    },
    ReplaceOne {
        doc_id: String,
        version: u64,
        query: String,
        replacement: String,
        case_sensitive: bool,
        use_regex: bool,
        start_char: usize,
        end_char: usize,
        source: SourceRequest,
    },
    ReplaceAll {
        doc_id: String,
        query: String,
        replacement: String,
        #[serde(rename = "caseSensitive")]
        case_sensitive: bool,
        #[serde(rename = "useRegex")]
        use_regex: bool,
    },
    UpdateSettings {
        patch: SettingsPatch,
    },
    SelectBuiltinWallpaper {
        id: String,
    },
    SelectRecentWallpaper {
        path: String,
    },
    ClearWallpaper,
    PickWallpaperDialog,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceRequest {
    start_line: usize,
    count: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsPatch {
    language: Option<String>,
    light: Option<bool>,
    theme_preset: Option<ThemePreset>,
    view_mode: Option<ViewMode>,
    font_size: Option<f32>,
    preview_max_width: Option<u32>,
    line_height: Option<f32>,
    sync_scroll: Option<bool>,
    focus_mode: Option<bool>,
    typewriter_mode: Option<bool>,
    auto_save: Option<bool>,
    show_line_numbers: Option<bool>,
    reduce_motion: Option<bool>,
    sidebar_open: Option<bool>,
    sidebar_tab: Option<String>,
    split_ratio: Option<f32>,
    wallpaper_effect: Option<Effect>,
    wallpaper_hero_opacity: Option<f32>,
    wallpaper_fills_window: Option<bool>,
    wallpaper_theme_colors: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state(docs: Vec<Document>) -> AppState {
        let disk = docs
            .iter()
            .filter_map(|doc| {
                let path = doc.path.as_deref()?;
                let expected = storage::disk_fingerprint(path).ok()?;
                Some((
                    doc.id.clone(),
                    TrackedFile::new(expected, FileStamp::read(path).ok().flatten()),
                ))
            })
            .collect();
        AppState {
            settings: Settings {
                auto_save: true,
                ..Settings::default()
            },
            docs,
            active_tab: Some(0),
            next_doc_num: 3,
            prepared_wp: None,
            wp_error: None,
            folder_root: None,
            latex_mains: HashMap::new(),
            autosaves: HashMap::new(),
            pending_close: None,
            disk,
            recovery_at: None,
            scan_at: Instant::now(),
            initialized: true,
            session_path: None,
            _session_lease: None,
        }
    }

    fn saved_document(path: &Path) -> Document {
        std::fs::write(path, "original").unwrap();
        Document::new(
            "first".into(),
            "first.md".into(),
            Some(dunce::canonicalize(path).unwrap()),
            "original",
        )
    }

    #[test]
    fn resolves_the_main_file_for_each_latex_tab_in_a_shared_directory() {
        let dir = tempfile::tempdir().unwrap();
        let alpha = dir.path().join("alpha.tex");
        let beta = dir.path().join("beta.tex");
        let chapter = dir.path().join("chapter.tex");
        let mut docs = Vec::new();
        for (id, path, source) in [
            ("a", &alpha, "\\documentclass{article}\n"),
            ("b", &beta, "\\documentclass{article}\n"),
            ("chapter", &chapter, "Chapter text\n"),
        ] {
            std::fs::write(path, source).unwrap();
            docs.push(Document::new(
                id.into(),
                path.file_name().unwrap().to_string_lossy().into(),
                Some(dunce::canonicalize(path).unwrap()),
                source,
            ));
        }
        let mut state = test_state(docs);
        assert_eq!(
            state.active_latex_main(),
            Some(dunce::canonicalize(alpha).unwrap())
        );
        state.active_tab = Some(1);
        let beta = dunce::canonicalize(beta).unwrap();
        assert_eq!(
            state.active_doc_snapshot(0)["texMain"],
            beta.to_string_lossy().as_ref()
        );
        state.active_tab = Some(2);
        assert_eq!(state.active_latex_main(), None);
        state.set_latex_main(&dunce::canonicalize(chapter).unwrap(), beta.clone());
        assert_eq!(state.active_latex_main(), Some(beta));
    }

    #[test]
    fn finds_the_open_main_file_for_a_nested_chapter_without_a_root_directive() {
        let dir = tempfile::tempdir().unwrap();
        let main = dir.path().join("main.tex");
        let sections = dir.path().join("sections");
        std::fs::create_dir(&sections).unwrap();
        let mut main_doc = saved_document(&main);
        main_doc.id = "main".into();
        std::fs::write(&main, "\\documentclass{article}\n").unwrap();
        let mut state = test_state(vec![
            saved_document(&sections.join("chapter.tex")),
            main_doc,
        ]);
        assert_eq!(
            state.active_latex_main(),
            Some(dunce::canonicalize(main).unwrap())
        );
        state.docs.pop();
        assert_eq!(state.active_latex_main(), None);
    }

    #[test]
    fn a_changed_file_is_protected_until_explicit_manual_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("first.md");
        let mut state = test_state(vec![saved_document(&path)]);
        state.docs[0].apply_edit(0, 8, "my draft", false);
        std::fs::write(&path, "new disk").unwrap();

        assert!(
            state
                .save_document(0, false, false)
                .unwrap_err()
                .contains("changed on disk")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new disk");
        assert_eq!(state.poll_external_change().unwrap()["docId"], "first");
        state.resolve_external_change("first", false).unwrap();
        assert!(state.save_document(0, false, false).unwrap().is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new disk");
        assert_eq!(state.save_document(0, false, true).unwrap(), Some(false));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "my draft");
    }

    #[test]
    fn recovering_open_tabs_keeps_drafts_and_detects_a_changed_disk_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("first.md");
        let mut state = test_state(vec![saved_document(&path)]);
        state.session_path = Some(dir.path().join("session.json"));
        state.docs[0].apply_edit(0, 8, "working copy", false);
        state.new_untitled_doc();
        state
            .active_doc_mut()
            .unwrap()
            .apply_edit(0, 0, "unsaved note", false);
        state.persist_session(true).unwrap();
        std::fs::write(&path, "external edit").unwrap();

        let manifest = std::fs::read(state.session_path.as_ref().unwrap()).unwrap();
        let mut recovered = test_state(Vec::new());
        recovered.active_tab = None;
        recovered.restore_session(serde_json::from_slice(&manifest).unwrap());
        assert_eq!(recovered.active_doc().unwrap().full_text(), "unsaved note");
        recovered.active_tab = Some(0);
        assert_eq!(recovered.active_doc().unwrap().full_text(), "working copy");
        let doc_id = recovered.active_doc().unwrap().id.clone();
        assert_eq!(recovered.poll_external_change().unwrap()["docId"], doc_id);
        assert_eq!(
            recovered.resolve_external_change(&doc_id, true).unwrap()["event"],
            "tabSwitched"
        );
        assert_eq!(recovered.active_doc().unwrap().full_text(), "external edit");
        assert!(!recovered.active_doc().unwrap().dirty);
    }

    #[test]
    fn clipboard_images_get_distinct_relative_paths_and_invalid_data_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("first.md");
        let mut state = test_state(vec![saved_document(&path)]);
        let encoded = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aBCsAAAAASUVORK5CYII=";
        let save = json!({ "cmd": "saveClipboardImage", "docId": "first", "request": 1, "encoded": encoded });
        let first = state.handle_command(&save.to_string());
        let second = state.handle_command(&save.to_string());
        let first_path = first["path"].as_str().unwrap();
        assert!(first_path.starts_with("assets/clipboard-") && first_path.ends_with(".png"));
        assert_ne!(first["path"], second["path"]);
        assert!(dir.path().join(first_path).is_file());
        let mut invalid = save;
        invalid["encoded"] = json!("bm90IGFuIGltYWdl");
        assert!(state.handle_command(&invalid.to_string())["error"].is_string());
        assert_eq!(
            std::fs::read_dir(dir.path().join("assets"))
                .unwrap()
                .count(),
            2
        );
    }

    #[test]
    fn live_widgets_resolve_document_relative_images_and_references() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("first.md");
        std::fs::write(directory.path().join("chart.svg"), "<svg/>").unwrap();
        let doc = Document::new(
            "first".into(),
            "first.md".into(),
            Some(path),
            "[chart]: chart.svg\n[example]: https://example.com/\n",
        );
        let mut state = test_state(vec![doc]);
        let response = state.handle_command(
            r#"{"cmd":"renderLiveWidgets","docId":"first","request":10,"fragments":["![A chart][chart]","[Click here][example]"]}"#,
        );
        assert_eq!(response["event"], "liveWidgetsRendered");
        assert_eq!(response["request"], 10);
        assert!(
            response["html"][0]
                .as_str()
                .unwrap()
                .contains("/local-image/")
        );
        assert!(
            response["html"][1]
                .as_str()
                .unwrap()
                .contains("https://example.com/")
        );
    }

    #[test]
    fn large_live_snapshot_defers_split_blocks_until_requested() {
        let doc = Document::new(
            "first".into(),
            "first.md".into(),
            None,
            &"Paragraph\n\n".repeat(4000),
        );
        let mut state = test_state(vec![doc]);
        state.settings.view_mode = ViewMode::Live;
        let live = state.active_doc_snapshot(200);
        assert_eq!(live["previewDeferred"], true);
        assert_eq!(live["blocks"].as_array().unwrap().len(), 0);
        assert_eq!(live["windowEndLine"], 600);
        let split = state.handle_command(r#"{"cmd":"fetchBlocks","docId":"first","start":0,"count":200,"request":10,"refresh":true,"aroundLine":0}"#);
        assert_eq!(split["blocks"].as_array().unwrap().len(), 200);
    }

    #[test]
    fn autosave_writes_the_edited_tab_after_switching_to_another() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("first.md");
        let mut state = test_state(vec![saved_document(&path)]);
        let edit = json!({
            "cmd": "edit", "docId": "first", "baseLine": 0,
            "startChar": 0, "endChar": 8, "text": "saved",
            "source": {"startLine": 0, "count": null},
        });
        assert_eq!(state.handle_command(&edit.to_string())["event"], "patch");
        state.new_untitled_doc();
        let events = state.run_autosaves(Instant::now() + Duration::from_secs(2));
        assert_eq!(events[0]["docId"], "first");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "saved");
        assert!(!state.docs[0].dirty);
        assert!(state.active_doc().unwrap().path.is_none());
    }

    #[test]
    fn closing_a_dirty_tab_requires_a_decision_for_that_document() {
        let mut doc = Document::new("first".into(), "first.md".into(), None, "original");
        doc.apply_edit(0, 8, "edited", false);
        let mut state = test_state(vec![doc]);
        let close = r#"{"cmd":"closeTab","id":"first"}"#;
        assert_eq!(state.handle_command(close)["event"], "confirmClose");
        assert_eq!(
            state.resolve_close("other", CloseChoice::Discard)["event"],
            "noop"
        );
        assert_eq!(state.active_doc().unwrap().full_text(), "edited");
        assert_eq!(
            state.resolve_close("first", CloseChoice::Cancel)["event"],
            "tabsUpdated"
        );
        assert_eq!(state.docs.len(), 1);
        assert_eq!(state.handle_command(close)["docId"], "first");
        assert_eq!(
            state.resolve_close("first", CloseChoice::Discard)["event"],
            "tabSwitched"
        );
        assert!(state.docs.is_empty());
    }

    #[test]
    fn closing_the_window_pauses_autosave_and_cancelling_keeps_all_tabs() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("first.md");
        let mut first = saved_document(&path);
        first.apply_edit(0, 8, "saved changes", false);
        let mut second = Document::new("second".into(), "second.md".into(), None, "");
        second.apply_edit(0, 0, "another change", false);
        let mut state = test_state(vec![first, second]);
        let now = Instant::now();
        state.autosaves.insert("first".into(), now);

        assert_eq!(state.request_window_close()["docId"], "first");
        assert_eq!(
            state.resolve_close("first", CloseChoice::Discard)["docId"],
            "second"
        );
        assert!(state.next_autosave().is_none());
        assert!(state.run_autosaves(now).is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        assert_eq!(
            state.resolve_close("second", CloseChoice::Cancel)["event"],
            "tabsUpdated"
        );
        assert_eq!(state.docs.len(), 2);
        assert!(state.docs.iter().all(|doc| doc.dirty));

        assert_eq!(state.request_window_close()["docId"], "first");
        assert_eq!(
            state.resolve_close("first", CloseChoice::Save)["docId"],
            "second"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "saved changes");
        assert_eq!(
            state.resolve_close("second", CloseChoice::Discard)["event"],
            "exitRequested"
        );
    }
}
