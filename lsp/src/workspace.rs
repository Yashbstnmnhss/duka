//! Document store and analysis snapshots
//!
//! Every language server request reads the same snapshot instead of running the
//! pipeline again, so hover, completion and diagnostics can never disagree. A
//! change bumps the workspace revision, which invalidates every snapshot; the
//! next request re-analyzes once and the result is reused until the next edit.
//! Text is synced in full, incremental parsing is a later step.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, MutexGuard},
};

use tower_lsp::lsp_types::Url;

use crate::compile::{self, DocAnalysis};

pub struct Document {
    pub text: String,
    pub version: i32,
    /// revision the snapshot was produced at
    pub revision: u64,
    pub analysis: Option<DocAnalysis>,
}

impl Document {
    fn new(text: String, version: i32) -> Self {
        Self {
            text,
            version,
            revision: 0,
            analysis: None,
        }
    }
}

#[derive(Default)]
pub struct Workspace {
    revision: u64,
    docs: HashMap<Url, Document>,
    /// module analysis caches, one per entry document
    build_caches: HashMap<String, duka_frontend::analyzer::modules::ModuleBuildCache>,
}

impl Workspace {
    pub fn open(&mut self, uri: Url, text: String, version: i32) {
        self.revision += 1;
        self.docs.insert(uri, Document::new(text, version));
    }

    pub fn change(&mut self, uri: &Url, text: String, version: i32) {
        self.revision += 1;
        let revision = self.revision;
        match self.docs.get_mut(uri) {
            Some(doc) => {
                doc.text = text;
                doc.version = version;
                // the snapshot is stale, the next request rebuilds it
                doc.revision = 0;
                doc.analysis = None;
            }
            None => {
                let mut doc = Document::new(text, version);
                doc.revision = revision;
                self.docs.insert(uri.clone(), doc);
            }
        }
    }

    pub fn close(&mut self, uri: &Url) {
        self.revision += 1;
        self.docs.remove(uri);
    }

    pub fn text(&self, uri: &Url) -> Option<String> {
        self.docs.get(uri).map(|doc| doc.text.clone())
    }

    pub fn version(&self, uri: &Url) -> Option<i32> {
        self.docs.get(uri).map(|doc| doc.version)
    }

    pub fn open_uris(&self) -> Vec<Url> {
        self.docs.keys().cloned().collect()
    }

    /// Paths of the open documents, a module that is being edited must be read
    /// from the editor instead of from disk, exactly like the lua and typescript
    /// plugins do for unsaved buffers
    fn open_paths(&self) -> HashMap<PathBuf, String> {
        let mut out = HashMap::new();
        for (uri, doc) in &self.docs {
            if let Ok(path) = uri.to_file_path() {
                out.insert(path, doc.text.clone());
            }
        }
        out
    }

    /// The shared analysis of a document, produced on demand
    pub fn analysis(&mut self, uri: &Url) -> Option<&DocAnalysis> {
        let revision = self.revision;
        let text = self.docs.get(uri)?.text.clone();
        let version = self.docs.get(uri).map(|doc| doc.version).unwrap_or(0);
        let open = self.open_paths();
        let cache = self.build_caches.entry(uri.to_string()).or_default();
        let file_path = uri.to_file_path().ok();
        let analysis = compile::analyze(&text, uri.as_str(), file_path.as_deref(), &open, cache);
        let doc = self.docs.get_mut(uri)?;
        if doc.revision != revision || doc.version != version {
            doc.revision = revision;
            doc.analysis = Some(analysis);
        }
        doc.analysis.as_ref()
    }
}

pub type SharedWorkspace = Mutex<Workspace>;

pub fn lock<'a>(workspace: &'a SharedWorkspace) -> MutexGuard<'a, Workspace> {
    workspace.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use duka_shared::types::SourceName;
    use tower_lsp::lsp_types::Url;

    fn uri(path: &str) -> Url {
        Url::parse(&format!("file:///{path}")).unwrap()
    }

    #[test]
    fn analysis_is_reused_until_a_change() {
        let mut workspace = Workspace::default();
        let entry = uri("C:/proj/main.duka");
        workspace.open(entry.clone(), "local x: int = 1".to_owned(), 1);
        assert!(workspace.analysis(&entry).unwrap().errors.is_empty());

        workspace.change(&entry, "local x: int = \"no\"".to_owned(), 2);
        let errors = &workspace.analysis(&entry).unwrap().errors;
        assert!(
            !errors.is_empty(),
            "the snapshot must be rebuilt after an edit, got {errors:?}"
        );
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
        let clean = workspace.analysis(&entry).unwrap().errors.len();
        assert!(clean > 0, "the disk module should be type checked");

        // the edited module returns a string, the same entry becomes valid
        let module_uri = Url::from_file_path(&module).unwrap();
        workspace.open(
            module_uri,
            "export function f(): string return \"s\" end".to_owned(),
            1,
        );
        workspace.change(&entry, entry_text.to_owned(), 2);
        let dirty = workspace.analysis(&entry).unwrap().errors.len();
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
        let analysis = workspace.analysis(&entry).unwrap();
        let foreign: Vec<_> = analysis
            .errors
            .iter()
            .filter(|e| matches!(&e.source_info.name, SourceName::File(..)))
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
}
