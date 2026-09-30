//! Finished jobs, kept across sessions in the data directory
//! (`~/.local/share/collider/history.json`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Oldest records are dropped beyond this many.
const LIMIT: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Completed,
    Failed,
    /// Refused because the stream uses DRM.
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub name: String,
    pub url: String,
    pub outcome: Outcome,
    /// Saved files (empty unless completed).
    pub files: Vec<PathBuf>,
    pub bytes: u64,
    pub live: bool,
    /// Unix time in seconds.
    pub finished: i64,
    /// The error, for failed jobs.
    #[serde(default)]
    pub detail: String,
}

#[derive(Debug, Default)]
pub struct History {
    /// Newest last.
    pub records: Vec<Record>,
    path: Option<PathBuf>,
}

impl History {
    pub fn default_path() -> Option<PathBuf> {
        dirs::data_dir().map(|d| d.join("collider").join("history.json"))
    }

    pub fn load_from(path: &Path) -> Self {
        let records = std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self {
            records,
            path: Some(path.to_path_buf()),
        }
    }

    pub fn push(&mut self, record: Record) {
        self.records.push(record);
        let excess = self.records.len().saturating_sub(LIMIT);
        self.records.drain(..excess);
        self.save();
    }

    pub fn remove(&mut self, index: usize) {
        if index < self.records.len() {
            self.records.remove(index);
            self.save();
        }
    }

    pub fn clear(&mut self) {
        self.records.clear();
        self.save();
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        let write = || -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(&self.records)?)?;
            std::fs::rename(&tmp, path)
        };
        if let Err(e) = write() {
            eprintln!("collider: cannot save history to {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(name: &str) -> Record {
        Record {
            name: name.into(),
            url: format!("https://example.com/{name}.m3u8"),
            outcome: Outcome::Completed,
            files: vec![PathBuf::from(format!("/tmp/{name}.mp4"))],
            bytes: 1024,
            live: false,
            finished: 1_790_000_000,
            detail: String::new(),
        }
    }

    #[test]
    fn persists_and_caps() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("history.json");
        let mut h = History::load_from(&path);
        assert!(h.records.is_empty());
        h.push(record("a"));
        h.push(record("b"));
        let mut again = History::load_from(&path);
        assert_eq!(again.records, [record("a"), record("b")]);
        again.remove(0);
        assert_eq!(History::load_from(&path).records, [record("b")]);
        for i in 0..LIMIT + 5 {
            again.push(record(&i.to_string()));
        }
        assert_eq!(again.records.len(), LIMIT);
        assert_eq!(again.records[0].name, "5");
    }
}
