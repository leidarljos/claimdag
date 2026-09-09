//! Short names for actor ids, so a claim says who rather than what.
//!
//! Optional and advisory: an id with no handle draws as its first eight hex
//! characters, which is what the command line prints. The file is read fresh
//! on every reload, so adding a name does not need a restart.

use std::collections::HashMap;
use std::path::Path;

use claimdag::WorkId;

/// Hex actor id to the name a person recognises.
#[derive(Debug, Clone, Default)]
pub struct Handles {
    names: HashMap<WorkId, String>,
}

impl Handles {
    /// Read `CLAIMDAG_HANDLES`, then `<dir>/handles.json`.
    ///
    /// Both are optional and a malformed one is ignored rather than fatal: a
    /// pane that will not open because a cosmetic file has a stray comma is
    /// worse than a pane that shows hex.
    #[must_use]
    pub fn load(dir: &Path) -> Self {
        let mut names = HashMap::new();
        let named = std::env::var_os("CLAIMDAG_HANDLES")
            .filter(|raw| !raw.is_empty())
            .map(std::path::PathBuf::from);
        for path in named
            .into_iter()
            .chain(std::iter::once(dir.join("handles.json")))
        {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Ok(serde_json::Value::Object(map)) = serde_json::from_str(&text) else {
                continue;
            };
            for (key, value) in map {
                let Some(id) = WorkId::from_hex(&key) else {
                    continue;
                };
                let Some(name) = value.as_str().map(str::trim).filter(|n| !n.is_empty()) else {
                    continue;
                };
                names.insert(id, name.to_string());
            }
        }
        Self { names }
    }

    /// The name for an id, or its first eight hex characters.
    #[must_use]
    pub fn name_for(&self, id: WorkId) -> String {
        self.names
            .get(&id)
            .cloned()
            .unwrap_or_else(|| id.to_hex()[..8].to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_id_draws_as_hex() {
        let id = WorkId::from_hex("0123456789abcdef0123456789abcdef").unwrap();
        assert_eq!(Handles::default().name_for(id), "01234567");
    }

    #[test]
    fn a_named_id_draws_as_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let hex = "0123456789abcdef0123456789abcdef";
        std::fs::write(
            dir.path().join("handles.json"),
            format!("{{\"{hex}\": \"reviewer\"}}"),
        )
        .unwrap();
        let handles = Handles::load(dir.path());
        assert_eq!(handles.name_for(WorkId::from_hex(hex).unwrap()), "reviewer");
    }

    #[test]
    fn a_malformed_file_leaves_the_pane_working() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("handles.json"), "{not json").unwrap();
        let id = WorkId::from_hex("0123456789abcdef0123456789abcdef").unwrap();
        assert_eq!(Handles::load(dir.path()).name_for(id), "01234567");
    }
}
