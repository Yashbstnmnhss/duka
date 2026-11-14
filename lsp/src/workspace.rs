//! Document store and analysis snapshots
//!
//! Every language server request reads the same snapshot instead of running the
//! pipeline again, so hover, completion and diagnostics can never disagree. A
//! change bumps the workspace revision, which invalidates every snapshot; the
//! next request re-analyzes once and the result is reused until the next edit.
//! The pipeline runs outside every lock: its inputs are captured under the
//! lock, the run happens on its own thread, and the result is committed only
//! if no edit landed while it ran, so an edit supersedes the run instead of
//! waiting for it.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use duka_lib::duka_frontend::{analyzer::modules::ModuleBuildCache, parser::IncrementalParseCache};
use duka_lib::duka_shared::{config::DukaParsingConfig, constants::SOURCE_SUFFIX};
use duka_lib::kao::{self, Kao};
use tower_lsp::lsp_types::{TextDocumentContentChangeEvent, Url};

use crate::{
    compile::{self, DocAnalysis},
    convert::LineIndex,
};

/// The kao project a document belongs to.
///
/// A document without a manifest above it is a file on its own: it resolves
/// modules beside itself and nothing more. A document inside a project instead
/// inherits the edition, features and strictness the manifest names, and the
/// files under the project are what `workspace/symbol` and a rename across the
/// project search.
#[derive(Debug, Clone)]
pub struct Project {
    pub kao: Kao,
    pub config: DukaParsingConfig,
}

impl Project {
    #[cfg(test)]
    pub fn modules_dir(&self) -> PathBuf {
        self.kao.root().join(self.kao.modules_dir())
    }

    /// Every `.duka` file under the project root. The build directory is
    /// skipped by `collect_sources`, so a compiled copy of a file is never
    /// indexed as a second declaration of it.
    pub fn source_files(&self) -> Vec<PathBuf> {
        let Ok(files) = kao::collect_sources(&self.kao, self.kao.root()) else {
            return vec![];
        };
        let mut out: Vec<PathBuf> = files
            .into_iter()
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some(SOURCE_SUFFIX))
            .collect();
        out.sort();
        out.dedup();
        out
    }
}

/// The manifest above `start`, if there is one.
///
/// `find_kao` answers with the starting directory when nothing was found, so
/// the manifest -- not the call succeeding -- is what says a project exists.
pub fn find_project(start: &Path) -> Option<Project> {
    let kao = kao::find_kao(start).ok()?;
    let manifest = kao.manifest()?;
    Some(Project {
        config: kao::project_config(manifest),
        kao,
    })
}

/// Everything a request needs, captured at one revision. The `Arc`s let a
/// snapshot outlive the workspace lock it was taken under, so two requests
/// read the same analysis without serialising on the lock.
pub struct Snapshot {
    pub text: Arc<String>,
    pub analysis: Arc<DocAnalysis>,
    pub line_index: Arc<LineIndex>,
}

/// What one run of the pipeline needs, taken while the lock is held so the run
/// itself never has to touch the workspace again.
pub struct Pending {
    uri: Url,
    pub(crate) text: Arc<String>,
    pub revision: u64,
    file_path: Option<PathBuf>,
    open: HashMap<PathBuf, Arc<String>>,
    build_cache: Arc<Mutex<ModuleBuildCache>>,
    parse_cache: Option<IncrementalParseCache>,
    edit_at_char: Option<u32>,
    project: Option<Project>,
}

impl Pending {
    /// Runs the pipeline. The module cache is a lock of its own rather than
    /// the workspace's, so a second run of the same entry waits for the first
    /// here instead of holding every request hostage.
    pub fn run(self) -> DocAnalysis {
        let config = self
            .project
            .as_ref()
            .map(|project| project.config.clone())
            .unwrap_or_default();
        let mut cache = self.build_cache.lock().unwrap_or_else(|e| e.into_inner());
        compile::analyze_incremental(
            self.text.as_str(),
            self.uri.as_str(),
            self.file_path.as_deref(),
            &self.open,
            &mut cache,
            self.parse_cache,
            self.edit_at_char,
            &config,
        )
    }
}

struct Document {
    text: Arc<String>,
    version: i32,
    /// revision the snapshot was produced at, `0` while stale
    revision: u64,
    analysis: Option<Arc<DocAnalysis>>,
    line_index: Option<Arc<LineIndex>>,
    parse_cache: Option<IncrementalParseCache>,
    parse_cache_text: Option<Arc<String>>,
    edit_at_char: Option<u32>,
    /// Serialises analysis of this document without holding the workspace:
    /// two requests for one file share a run instead of starting two.
    gate: Arc<tokio::sync::Mutex<()>>,
}

impl Document {
    fn new(text: Arc<String>, version: i32) -> Self {
        Self {
            text,
            version,
            revision: 0,
            analysis: None,
            line_index: None,
            parse_cache: None,
            parse_cache_text: None,
            edit_at_char: None,
            gate: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
}

/// A file that is not open in the editor but was analyzed for a project-wide
/// question. It is dropped when the file itself changes, so an edit in another
/// buffer does not throw away the whole project index.
struct DiskAnalysis {
    text: Arc<String>,
    analysis: Arc<DocAnalysis>,
    line_index: Arc<LineIndex>,
}

#[derive(Default)]
pub struct Workspace {
    revision: u64,
    docs: HashMap<Url, Document>,
    /// module analysis caches, one per entry document, shared with every run
    /// of that entry so a repeated analysis reuses what the first one built
    build_caches: HashMap<String, Arc<Mutex<ModuleBuildCache>>>,
    /// Project discovery is a walk and a small TOML read, cached by directory
    /// until a `kao.toml` changes.
    projects: HashMap<PathBuf, Option<Project>>,
    /// Files analyzed on demand for project-wide searches.
    disk: HashMap<PathBuf, DiskAnalysis>,
}

impl Workspace {
    pub fn open(&mut self, uri: Url, text: String, version: i32) {
        self.revision += 1;
        self.docs
            .insert(uri, Document::new(Arc::new(text), version));
    }

    /// Applies a full-text change.
    #[cfg(test)]
    pub fn change(&mut self, uri: &Url, text: String, version: i32) -> bool {
        self.apply_change(
            uri,
            &[TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text,
            }],
            version,
        )
    }

    /// Applies the changes one did_change carries, in order: a change without
    /// a range replaces the text, the rest are edits at the positions the
    /// client named. A version that does not move forward is a replay or an
    /// event that arrived out of order: applying it would take the text back
    /// into the past, so it is dropped and the caller is told.
    pub fn apply_change(
        &mut self,
        uri: &Url,
        changes: &[TextDocumentContentChangeEvent],
        version: i32,
    ) -> bool {
        match self.docs.get_mut(uri) {
            Some(doc) => {
                if version <= doc.version {
                    return false;
                }
                self.revision += 1;
                let mut text = (*doc.text).clone();
                for change in changes {
                    crate::convert::apply_change(&mut text, change);
                }
                let old_text = doc
                    .parse_cache_text
                    .as_ref()
                    .map(|t| t.as_str())
                    .unwrap_or(doc.text.as_str());
                doc.edit_at_char = Some(Self::first_changed_char(old_text, &text));
                doc.text = Arc::new(text);
                doc.version = version;
                // the snapshot is stale, the next request rebuilds it
                doc.revision = 0;
                doc.analysis = None;
                doc.line_index = None;
                true
            }
            None => {
                self.revision += 1;
                let mut text = String::new();
                for change in changes {
                    crate::convert::apply_change(&mut text, change);
                }
                let mut doc = Document::new(Arc::new(text), version);
                doc.edit_at_char = Some(0);
                self.docs.insert(uri.clone(), doc);
                true
            }
        }
    }

    pub fn close(&mut self, uri: &Url) {
        self.revision += 1;
        self.docs.remove(uri);
        // a kept build cache holds a whole module map for a file the reader no
        // longer has open; the next open rebuilds it for less than a session
        // of remembering every file they once looked at
        self.build_caches.remove(&uri.to_string());
    }

    pub fn open_uris(&self) -> Vec<Url> {
        self.docs.keys().cloned().collect()
    }

    /// Invalidates every snapshot without touching a document. A file that
    /// changed on disk is not an edit in any buffer, but it can still change
    /// what a `require` resolves to, so the next request has to run again.
    pub fn touch(&mut self) {
        self.revision += 1;
    }

    /// Paths of the open documents, a module that is being edited must be read
    /// from the editor instead of from disk, exactly like the lua and
    /// typescript plugins do for unsaved buffers
    fn open_paths(&self) -> HashMap<PathBuf, Arc<String>> {
        self.docs
            .iter()
            .filter_map(|(uri, doc)| {
                uri.to_file_path()
                    .ok()
                    .map(|path| (path, Arc::clone(&doc.text)))
            })
            .collect()
    }

    /// The snapshot of a document whose analysis matches the revision the
    /// workspace is at now. A snapshot that is already current is the whole
    /// point of keeping one: running the pipeline again on every hover made
    /// each request pay for a full analysis of the file.
    pub fn snapshot(&self, uri: &Url) -> Option<Snapshot> {
        let doc = self.docs.get(uri)?;
        if doc.revision != self.revision {
            return None;
        }
        Some(Snapshot {
            text: Arc::clone(&doc.text),
            analysis: Arc::clone(doc.analysis.as_ref()?),
            line_index: Arc::clone(doc.line_index.as_ref()?),
        })
    }

    /// The gate that serialises analysis of one open document. A request that
    /// holds it re-checks the snapshot after the run it waited for commits, so
    /// a burst of requests costs one analysis rather than one each.
    pub fn analysis_gate(&self, uri: &Url) -> Option<Arc<tokio::sync::Mutex<()>>> {
        self.docs.get(uri).map(|doc| Arc::clone(&doc.gate))
    }

    /// The state one run of the pipeline needs, captured before the lock is
    /// let go. The finished run comes back through `commit`.
    pub fn pending_analysis(&mut self, uri: &Url) -> Option<Pending> {
        let (text, parse_cache, edit_at_char) = {
            let doc = self.docs.get_mut(uri)?;
            (
                Arc::clone(&doc.text),
                doc.parse_cache.take(),
                doc.edit_at_char.take(),
            )
        };
        let revision = self.revision;
        let open = self.open_paths();
        let file_path = uri.to_file_path().ok();
        let project = self.project_for(file_path.as_deref());
        let build_cache = Arc::clone(self.build_caches.entry(uri.to_string()).or_default());
        Some(Pending {
            uri: uri.clone(),
            text,
            revision,
            file_path,
            open,
            build_cache,
            parse_cache,
            edit_at_char,
            project,
        })
    }

    /// The project a file path belongs to, discovered once per directory and
    /// remembered until a manifest changes.
    pub fn project_for(&mut self, file_path: Option<&Path>) -> Option<Project> {
        let dir = file_path.and_then(Path::parent)?;
        self.project_for_dir(dir)
    }

    /// The project a directory belongs to. A workspace folder is a directory
    /// rather than a file, so it asks this directly.
    pub fn project_for_dir(&mut self, dir: &Path) -> Option<Project> {
        let dir = dir.to_path_buf();
        if let Some(cached) = self.projects.get(&dir) {
            return cached.clone();
        }
        let project = find_project(&dir);
        self.projects.insert(dir, project.clone());
        project
    }

    /// Forgets every discovered manifest, so the next analysis re-reads them.
    pub fn invalidate_projects(&mut self) {
        self.projects.clear();
        self.disk.clear();
        self.build_caches.clear();
    }

    /// A file on disk with no editor buffer behind it, ready to be analyzed.
    ///
    /// This is the other half of an open document: `workspace/symbol` and a
    /// rename across a project ask about files the reader never opened, and the
    /// answer has to come from the same pipeline, with the open buffers still
    /// winning over the disk where the two overlap.
    pub fn pending_disk(&mut self, path: &Path) -> Option<Pending> {
        if self
            .docs
            .keys()
            .filter_map(|uri| uri.to_file_path().ok())
            .any(|open| open == path)
        {
            return None;
        }
        let text = Arc::new(std::fs::read_to_string(path).ok()?);
        let uri = Url::from_file_path(path).ok()?;
        let revision = self.revision;
        let open = self.open_paths();
        let project = self.project_for(Some(path));
        let build_cache = Arc::clone(self.build_caches.entry(uri.to_string()).or_default());
        Some(Pending {
            uri,
            text,
            revision,
            file_path: Some(path.to_path_buf()),
            open,
            build_cache,
            parse_cache: None,
            edit_at_char: None,
            project,
        })
    }

    pub fn commit_disk(&mut self, path: PathBuf, text: Arc<String>, analysis: DocAnalysis) {
        let line_index = Arc::new(LineIndex::new(&text));
        self.disk.insert(
            path,
            DiskAnalysis {
                text,
                analysis: Arc::new(analysis),
                line_index,
            },
        );
    }

    /// The uri an open document was registered under, by its path. A client
    /// may spell the same file differently, so the comparison is on the path
    /// the uri decodes to rather than on the text of the uri.
    pub fn open_uri_for_path(&self, path: &Path) -> Option<Url> {
        self.docs
            .keys()
            .find(|uri| uri.to_file_path().ok().as_deref() == Some(path))
            .cloned()
    }

    pub fn disk_snapshot(&self, path: &Path) -> Option<Snapshot> {
        let entry = self.disk.get(path)?;
        Some(Snapshot {
            text: Arc::clone(&entry.text),
            analysis: Arc::clone(&entry.analysis),
            line_index: Arc::clone(&entry.line_index),
        })
    }

    pub fn invalidate_disk(&mut self, path: &Path) {
        self.disk.remove(path);
    }

    /// Commits a finished run. `true` when it is the snapshot the document is
    /// waiting for; `false` when an edit superseded it. A superseded run
    /// leaves its parse cache behind when it is for the text the document
    /// pairs its cache with, so the next run of that text resumes instead of
    /// parsing from scratch.
    pub fn commit(
        &mut self,
        uri: &Url,
        revision: u64,
        text: &Arc<String>,
        analysis: DocAnalysis,
    ) -> (bool, Arc<DocAnalysis>) {
        let mut analysis = Arc::new(analysis);
        let parse_cache =
            Arc::get_mut(&mut analysis).and_then(|analysis| analysis.parse_cache.take());
        let Some(doc) = self.docs.get_mut(uri) else {
            return (false, analysis);
        };
        if self.revision != revision {
            if doc
                .parse_cache_text
                .as_ref()
                .is_some_and(|known| Arc::ptr_eq(known, text))
            {
                doc.parse_cache = parse_cache;
            }
            return (false, analysis);
        }
        doc.revision = revision;
        doc.analysis = Some(Arc::clone(&analysis));
        doc.line_index = Some(Arc::new(LineIndex::new(text)));
        doc.parse_cache = parse_cache;
        doc.parse_cache_text = Some(Arc::clone(text));
        doc.edit_at_char = None;
        (true, analysis)
    }

    fn first_changed_char(old: &str, new: &str) -> u32 {
        let mut common = old
            .bytes()
            .zip(new.bytes())
            .take_while(|(left, right)| left == right)
            .count();
        while !old.is_char_boundary(common) || !new.is_char_boundary(common) {
            common -= 1;
        }
        old[..common].chars().count() as u32
    }
}

pub type SharedWorkspace = Mutex<Workspace>;

pub fn lock<'a>(workspace: &'a SharedWorkspace) -> MutexGuard<'a, Workspace> {
    workspace.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
impl Workspace {
    /// The synchronous path tests run on: capture, run and commit with no
    /// thread in between, so a test walks the async path's own steps.
    fn analyze_now(&mut self, uri: &Url) -> Option<Arc<DocAnalysis>> {
        if let Some(snapshot) = self.snapshot(uri) {
            return Some(snapshot.analysis);
        }
        let pending = self.pending_analysis(uri)?;
        let revision = pending.revision;
        let text = Arc::clone(&pending.text);
        let analysis = pending.run();
        let _ = self.commit(uri, revision, &text, analysis);
        self.snapshot(uri).map(|snapshot| snapshot.analysis)
    }

    /// The revision a document was last analyzed at, `None` when it never was
    pub fn analyzed_revision(&self, uri: &Url) -> Option<u64> {
        self.docs
            .get(uri)
            .filter(|doc| doc.analysis.is_some())
            .map(|doc| doc.revision)
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use duka_lib::duka_shared::types::SourceName;

    fn uri(path: &str) -> Url {
        Url::parse(&format!("file:///{path}")).unwrap()
    }

    #[test]
    fn analysis_is_reused_until_a_change() {
        let mut workspace = Workspace::default();
        let entry = uri("C:/proj/main.duka");
        workspace.open(entry.clone(), "local x: int = 1".to_owned(), 1);
        assert!(workspace.analyze_now(&entry).unwrap().errors.is_empty());

        workspace.change(&entry, "local x: int = \"no\"".to_owned(), 2);
        let errors = &workspace.analyze_now(&entry).unwrap().errors;
        assert!(
            !errors.is_empty(),
            "the snapshot must be rebuilt after an edit, got {errors:?}"
        );
    }

    #[test]
    fn analysis_is_computed_once_until_something_changes() {
        let mut workspace = Workspace::default();
        let entry = uri("C:/proj/once.duka");
        workspace.open(entry.clone(), "local x: int = 1".to_owned(), 1);

        workspace.analyze_now(&entry).expect("first analysis");
        assert_eq!(
            workspace.analyzed_revision(&entry),
            Some(workspace.revision())
        );

        let before = compile::analysis_count();
        for _ in 0..5 {
            workspace.analyze_now(&entry).expect("cached analysis");
        }
        assert_eq!(
            compile::analysis_count(),
            before,
            "a current snapshot must not run the pipeline again"
        );

        workspace.change(&entry, "local x: int = 2".to_owned(), 2);
        assert_ne!(
            workspace.analyzed_revision(&entry),
            Some(workspace.revision()),
            "an edit must invalidate the snapshot"
        );
        workspace
            .analyze_now(&entry)
            .expect("analysis after the edit");
        assert_eq!(
            workspace.analyzed_revision(&entry),
            Some(workspace.revision())
        );
    }

    #[test]
    fn a_second_document_invalidates_the_first() {
        let mut workspace = Workspace::default();
        let a = uri("C:/proj/a.duka");
        let b = uri("C:/proj/b.duka");
        workspace.open(a.clone(), "local x: int = 1".to_owned(), 1);
        workspace.analyze_now(&a).expect("a");
        let revision = workspace.revision();
        workspace.open(b.clone(), "local y: int = 1".to_owned(), 1);
        workspace.analyze_now(&b).expect("b");
        assert_ne!(
            workspace.analyzed_revision(&a),
            Some(workspace.revision()),
            "a module can reach any other, so a revision is workspace wide"
        );
        workspace.analyze_now(&a).expect("a again");
        assert!(workspace.analyzed_revision(&a).unwrap() > revision);
    }

    #[test]
    fn open_module_wins_over_disk() {
        let dir = std::env::temp_dir().join("duka-lsp-ws-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let module = dir.join("m.duka");
        std::fs::write(&module, "export function f(): int return 1 end").unwrap();

        let entry_path = dir.join("main.duka");
        let entry_text = "local m = require(\"./m\") local n: string = m.f()";
        let entry = Url::from_file_path(&entry_path).unwrap();

        let mut workspace = Workspace::default();
        workspace.open(entry.clone(), entry_text.to_owned(), 1);
        // the disk version returns an int, so assigning it to a string fails
        let clean = workspace.analyze_now(&entry).unwrap().errors.len();
        assert!(clean > 0, "the disk module should be type checked");

        // the edited module returns a string, the same entry becomes valid
        let module_uri = Url::from_file_path(&module).unwrap();
        workspace.open(
            module_uri,
            "export function f(): string return \"s\" end".to_owned(),
            1,
        );
        workspace.change(&entry, entry_text.to_owned(), 2);
        let dirty = workspace.analyze_now(&entry).unwrap().errors.len();
        assert_eq!(
            dirty, 0,
            "an edited module must be analyzed instead of the file on disk"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn module_errors_are_reported_on_the_module() {
        let dir = std::env::temp_dir().join("duka-lsp-diag-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let module = dir.join("bad.duka");
        std::fs::write(&module, "local n: int = \"no\"").unwrap();
        let entry_path = dir.join("main.duka");
        let entry = Url::from_file_path(&entry_path).unwrap();

        let mut workspace = Workspace::default();
        workspace.open(entry.clone(), "local m = require(\"./bad\")".to_owned(), 1);
        let analysis = workspace.analyze_now(&entry).unwrap();
        let foreign: Vec<_> = analysis
            .errors
            .iter()
            .filter(|e| matches!(&e.source_info().name, SourceName::File(..)))
            .filter(|e| {
                crate::convert::error_uri(e, &entry) != Url::from_file_path(&entry_path).unwrap()
            })
            .collect();
        assert!(
            !foreign.is_empty(),
            "a module error must point at the module uri, got {:?}",
            analysis.errors
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_ranged_change_reaches_the_text() {
        use tower_lsp::lsp_types::{Position, Range, TextDocumentContentChangeEvent};

        let mut workspace = Workspace::default();
        let entry = uri("C:/proj/ranged.duka");
        workspace.open(entry.clone(), "local x: int = 1\nreturn x\n".to_owned(), 1);
        workspace.apply_change(
            &entry,
            &[TextDocumentContentChangeEvent {
                range: Some(Range::new(Position::new(0, 15), Position::new(0, 16))),
                range_length: None,
                text: "\"no\"".to_owned(),
            }],
            2,
        );
        let snapshot = workspace.analyze_now(&entry).expect("analyzed");
        assert!(
            !snapshot.errors.is_empty(),
            "the edit between the positions the client named must reach the text: {:?}",
            snapshot.errors
        );
    }

    #[test]
    fn a_change_that_does_not_move_the_version_is_dropped() {
        let mut workspace = Workspace::default();
        let entry = uri("C:/proj/version.duka");
        workspace.open(entry.clone(), "local x: int = 1".to_owned(), 5);
        assert!(
            !workspace.change(&entry, "local x: int = \"no\"".to_owned(), 5),
            "the same version is a replay"
        );
        assert!(
            !workspace.change(&entry, "local x: int = \"no\"".to_owned(), 4),
            "an older version is out of order"
        );
        let snapshot = workspace.analyze_now(&entry).expect("analyzed");
        assert!(
            snapshot.errors.is_empty(),
            "a dropped change must not reach the text, got {:?}",
            snapshot.errors
        );

        assert!(workspace.change(&entry, "local x: int = \"no\"".to_owned(), 6));
        let snapshot = workspace.analyze_now(&entry).expect("analyzed");
        assert!(
            !snapshot.errors.is_empty(),
            "a change whose version moves forward applies"
        );
    }

    #[test]
    fn a_run_superseded_by_an_edit_leaves_no_snapshot() {
        let mut workspace = Workspace::default();
        let entry = uri("C:/proj/superseded.duka");
        workspace.open(entry.clone(), "local x = 1".to_owned(), 1);
        let pending = workspace.pending_analysis(&entry).expect("pending");
        let revision = pending.revision;
        let text = Arc::clone(&pending.text);
        let analysis = pending.run();
        workspace.change(&entry, "local y = 2".to_owned(), 2);
        assert!(
            !workspace.commit(&entry, revision, &text, analysis).0,
            "an edit that landed while the run went on supersedes it"
        );
        assert!(
            workspace.snapshot(&entry).is_none(),
            "the superseded run must not be served as the snapshot"
        );
    }

    #[test]
    fn a_run_committed_at_the_revision_it_captured_is_the_snapshot() {
        let mut workspace = Workspace::default();
        let entry = uri("C:/proj/committed.duka");
        workspace.open(entry.clone(), "local x = 1".to_owned(), 1);
        let pending = workspace.pending_analysis(&entry).expect("pending");
        let revision = pending.revision;
        let text = Arc::clone(&pending.text);
        let analysis = pending.run();
        assert!(workspace.commit(&entry, revision, &text, analysis).0);
        assert!(
            workspace.snapshot(&entry).is_some(),
            "the run's own snapshot is served"
        );
        assert_eq!(
            workspace.analyzed_revision(&entry),
            Some(workspace.revision())
        );
    }

    #[test]
    fn closing_a_document_forgets_its_build_cache() {
        let mut workspace = Workspace::default();
        let entry = uri("C:/proj/cache.duka");
        workspace.open(entry.clone(), "local x = 1".to_owned(), 1);
        workspace.analyze_now(&entry).expect("analyzed");
        assert!(workspace.build_caches.contains_key(&entry.to_string()));
        workspace.close(&entry);
        assert!(workspace.build_caches.is_empty());
    }

    #[test]
    fn a_manifest_is_found_above_the_file_and_carries_its_config() {
        let dir = std::env::temp_dir().join("duka-lsp-project-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("kao.toml"),
            "[kao]\nname = \"demo\"\nedition = \"0.2\"\n\n[build]\nmodules_dir = \"libs\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("src").join("main.duka"), "return 1\n").unwrap();

        let project = find_project(&dir.join("src")).expect("a project");
        assert_eq!(project.config.analyzer.edition.as_deref(), Some("0.2"));
        assert_eq!(project.modules_dir(), dir.join("libs"));

        let files = project.source_files();
        assert!(files.iter().any(|p| p.ends_with("main.duka")), "{files:?}");
    }

    #[test]
    fn project_discovery_is_cached_until_a_manifest_changes() {
        let dir = std::env::temp_dir().join("duka-lsp-project-cache");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut workspace = Workspace::default();
        assert!(workspace.project_for_dir(&dir).is_none());

        std::fs::write(dir.join("kao.toml"), "[kao]\nname = \"late\"\n").unwrap();
        assert!(
            workspace.project_for_dir(&dir).is_none(),
            "the answer was cached before the manifest existed"
        );
        workspace.invalidate_projects();
        assert!(workspace.project_for_dir(&dir).is_some());
    }

    #[test]
    fn a_file_on_disk_is_analyzed_without_being_opened() {
        let dir = std::env::temp_dir().join("duka-lsp-disk-analysis");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("loose.duka");
        std::fs::write(&path, "local x: int = 1\nreturn x\n").unwrap();

        let mut workspace = Workspace::default();
        let pending = workspace.pending_disk(&path).expect("a pending run");
        let text = Arc::clone(&pending.text);
        let analysis = pending.run();
        workspace.commit_disk(path.clone(), text, analysis);
        assert!(workspace.disk_snapshot(&path).is_some());
        assert!(workspace.open_uri_for_path(&path).is_none());

        workspace.touch();
        assert!(
            workspace.disk_snapshot(&path).is_some(),
            "an edit in another buffer leaves this file's analysis alone"
        );
        workspace.invalidate_disk(&path);
        assert!(workspace.disk_snapshot(&path).is_none());
    }
}
