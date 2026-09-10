//! Pane state and key handling. Drawing lives in [`crate::view`].
//!
//! Mutations go through [`claimdag::WorkGraph`], the same calls the command
//! line makes. This pane does not reimplement claim, complete or unlink, and
//! it does not relax their guards: an action a stranger may not take fails
//! here with the message the graph gives it.

use std::path::{Path, PathBuf};

use claimdag::{WorkGraph, WorkId, WorkStatus};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind};

use crate::forest::{self, Row};
use crate::handles::Handles;

/// What the event loop does after a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Stay in the event loop.
    Continue,
    /// Leave the event loop.
    Quit,
}

/// The bindings, as drawn in the footer.
pub const HELP: &str =
    "c claim  d done  R reclaim  a archive  u unlink  h done?  A archived?  r reload  q quit";

/// The work graph pane.
#[derive(Debug)]
pub struct App {
    dir: PathBuf,
    actor: WorkId,
    handles: Handles,
    /// Rows for the current filter, depth first.
    pub rows: Vec<Row>,
    /// Index into [`Self::rows`].
    pub selected: usize,
    /// Whether finished work is drawn.
    pub show_terminal: bool,
    /// Whether archived work is drawn.
    pub show_archived: bool,
    /// The last thing that happened, drawn in the status line.
    pub message: String,
    /// What the files looked like at the last reload.
    stamp: u128,
    /// How long a claim may go quiet before the pane will hand it back.
    ///
    /// A pane's lease is not the graph's: the graph has none, it takes one per
    /// call. This is the number the operator is acting on when they press the
    /// key, so it lives beside the key rather than in the library.
    pub lease: u64,
}

/// The lease the pane offers, in seconds, matching the command line's default.
pub const DEFAULT_LEASE: u64 = 900;

impl App {
    /// Open the pane on a directory.
    #[must_use]
    pub fn open(dir: PathBuf, actor: WorkId) -> Self {
        let handles = Handles::load(&dir);
        let mut app = Self {
            dir,
            actor,
            handles,
            rows: Vec::new(),
            selected: 0,
            show_terminal: false,
            show_archived: false,
            message: String::new(),
            stamp: 0,
            lease: DEFAULT_LEASE,
        };
        app.reload();
        app
    }

    /// The directory this pane is open on.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The id under the cursor, if any.
    #[must_use]
    pub fn selected_id(&self) -> Option<WorkId> {
        self.rows
            .get(self.selected)
            .filter(|row| !row.cycle)
            .map(|row| row.id)
    }

    /// Re-read the graph and rebuild the rows, keeping the cursor on its node.
    pub fn reload(&mut self) {
        let held = self.selected_id();
        self.handles = Handles::load(&self.dir);
        let graph = WorkGraph::load_dir(&self.dir);
        let nodes = graph.list_view(self.show_terminal, self.show_archived);
        self.rows = forest::rows(&nodes, &self.handles);
        self.stamp = self.snapshot_stamp();
        self.selected = held
            .and_then(|id| self.rows.iter().position(|row| row.id == id))
            .unwrap_or_else(|| self.selected.min(self.rows.len().saturating_sub(1)));
    }

    /// Reload when the files changed under us, so a claim made elsewhere shows
    /// up without anyone pressing a key.
    pub fn poll(&mut self) {
        if self.snapshot_stamp() != self.stamp {
            self.reload();
        }
    }

    /// The newest write time across the files the pane reads.
    fn snapshot_stamp(&self) -> u128 {
        ["work.bin", "handles.json"]
            .iter()
            .filter_map(|name| std::fs::metadata(self.dir.join(name)).ok())
            .filter_map(|meta| meta.modified().ok())
            .filter_map(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|since| since.as_nanos())
            .max()
            .unwrap_or(0)
    }

    /// Load, mutate, save. The graph is the only writer of its own rules.
    fn mutate(&mut self, done: &str, change: impl FnOnce(&mut WorkGraph) -> Result<(), String>) {
        let mut graph = WorkGraph::load_dir(&self.dir);
        self.message = match change(&mut graph) {
            Err(why) => why,
            Ok(()) => match graph.save_dir(&self.dir) {
                Err(why) => why,
                Ok(()) => done.to_string(),
            },
        };
        self.reload();
    }

    /// Handle one key.
    pub fn handle_key(&mut self, key: KeyEvent) -> Action {
        if key.kind == KeyEventKind::Release {
            return Action::Continue;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Action::Quit,
            KeyCode::Char('j') | KeyCode::Down => self.move_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_by(-1),
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = self.rows.len().saturating_sub(1),
            KeyCode::Char('r') => self.reload(),
            KeyCode::Char('h') => {
                self.show_terminal = !self.show_terminal;
                self.message = if self.show_terminal {
                    "showing finished work".into()
                } else {
                    "hiding finished work".into()
                };
                self.reload();
            }
            KeyCode::Char('A') => {
                self.show_archived = !self.show_archived;
                self.message = if self.show_archived {
                    "showing archived work".into()
                } else {
                    "hiding archived work".into()
                };
                self.reload();
            }
            KeyCode::Char('c') => self.claim(),
            KeyCode::Char('d') => self.complete(),
            KeyCode::Char('a') => self.archive(),
            KeyCode::Char('u') => self.unlink(),
            KeyCode::Char('R') => self.reclaim(),
            _ => {}
        }
        Action::Continue
    }

    fn move_by(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() - 1;
        self.selected = match delta {
            d if d < 0 => self.selected.saturating_sub(d.unsigned_abs()),
            d => (self.selected + d.unsigned_abs()).min(last),
        };
    }

    fn claim(&mut self) {
        let Some(id) = self.selected_id() else {
            self.message = "select a node".into();
            return;
        };
        let actor = self.actor;
        // No generation: the pane claims what it is looking at, and the graph
        // still refuses one already held by somebody else.
        self.mutate("claimed", |graph| graph.claim(id, actor, None).map(|_| ()));
    }

    /// Hand back every claim quiet longer than the lease.
    ///
    /// The pane already shows how long a held node has been quiet, and an
    /// operator watching a stalled claim had to leave the pane to act on what
    /// the pane was telling them.
    ///
    /// Every stale claim rather than the selected one, because the lease is
    /// the rule and applying it to one node is a judgement the lease already
    /// made. What is selected has nothing to do with which claims went quiet.
    fn reclaim(&mut self) {
        // Not through `mutate`, because the count has to come back out and a
        // closure that owns it cannot hand it over.
        let mut graph = WorkGraph::load_dir(&self.dir);
        let handed = graph.reclaim(self.lease);
        self.message = if handed.is_empty() {
            format!("nothing has been quiet for {}s", self.lease)
        } else {
            match graph.save_dir(&self.dir) {
                Err(why) => why,
                Ok(()) => match handed.len() {
                    1 => "handed back 1 claim".to_string(),
                    n => format!("handed back {n} claims"),
                },
            }
        };
        self.reload();
    }

    fn complete(&mut self) {
        let Some(id) = self.selected_id() else {
            self.message = "select a node".into();
            return;
        };
        let actor = self.actor;
        self.mutate("done", |graph| {
            graph.complete(id, WorkStatus::Done, "", actor, None)
        });
    }

    fn archive(&mut self) {
        let Some(id) = self.selected_id() else {
            self.message = "select a node".into();
            return;
        };
        let actor = self.actor;
        self.mutate("archived", |graph| graph.archive(id, actor));
    }

    /// Drop the first dependency the selected node waits on.
    ///
    /// The first rather than a chosen one, because the pane draws a count and
    /// not the edges; a node with several is unblocked one press at a time.
    fn unlink(&mut self) {
        let Some(id) = self.selected_id() else {
            self.message = "select a node".into();
            return;
        };
        let graph = WorkGraph::load_dir(&self.dir);
        let Some(dep) = graph.get(id).and_then(|node| node.deps.first().copied()) else {
            self.message = "no edge".into();
            return;
        };
        let actor = self.actor;
        self.mutate("unlinked", |graph| graph.unlink_dep(dep, id, actor));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claimdag::{WorkFields, WorkKind, WorkRole};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, ratatui::crossterm::event::KeyModifiers::NONE)
    }

    fn seeded(dir: &Path) -> WorkId {
        let mut graph = WorkGraph::default();
        let id = graph.mint_id(WorkKind::Task, WorkId::ZERO, "land the adapter");
        graph
            .upsert(
                id,
                WorkFields {
                    kind: WorkKind::Task,
                    status: WorkStatus::Todo,
                    role: WorkRole::Unset,
                    parent: WorkId::ZERO,
                    actor: WorkId::ZERO,
                    summary: "land the adapter",
                },
            )
            .unwrap();
        graph.save_dir(dir).unwrap();
        id
    }

    fn actor() -> WorkId {
        WorkId::from_hex("0123456789abcdef0123456789abcdef").unwrap()
    }

    #[test]
    fn the_pane_opens_on_what_is_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let id = seeded(dir.path());
        let app = App::open(dir.path().to_path_buf(), actor());
        assert_eq!(app.rows.len(), 1);
        assert_eq!(app.selected_id(), Some(id));
    }

    #[test]
    fn claiming_writes_through_the_graph_and_shows_up_on_reload() {
        let dir = tempfile::tempdir().unwrap();
        let id = seeded(dir.path());
        let mut app = App::open(dir.path().to_path_buf(), actor());
        app.handle_key(key(KeyCode::Char('c')));
        assert_eq!(app.message, "claimed", "{}", app.message);
        let graph = WorkGraph::load_dir(dir.path());
        assert_eq!(graph.get(id).unwrap().assignee, actor());
    }

    /// The pane must not be a way around the guard the command line obeys.
    #[test]
    fn a_stranger_cannot_complete_somebody_elses_work() {
        let dir = tempfile::tempdir().unwrap();
        let id = seeded(dir.path());
        let mut holder = App::open(dir.path().to_path_buf(), actor());
        holder.handle_key(key(KeyCode::Char('c')));

        let other = WorkId::from_hex("fedcba9876543210fedcba9876543210").unwrap();
        let mut stranger = App::open(dir.path().to_path_buf(), other);
        stranger.handle_key(key(KeyCode::Char('d')));
        assert!(
            stranger.message.contains("not assignee"),
            "{}",
            stranger.message
        );
        assert!(!WorkGraph::load_dir(dir.path())
            .get(id)
            .unwrap()
            .status
            .is_terminal());
    }

    #[test]
    fn finished_work_is_hidden_until_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        seeded(dir.path());
        let mut app = App::open(dir.path().to_path_buf(), actor());
        app.handle_key(key(KeyCode::Char('c')));
        app.handle_key(key(KeyCode::Char('d')));
        assert!(app.rows.is_empty());
        app.handle_key(key(KeyCode::Char('h')));
        assert_eq!(app.rows.len(), 1);
    }

    #[test]
    fn a_write_from_elsewhere_arrives_without_a_key_press() {
        let dir = tempfile::tempdir().unwrap();
        seeded(dir.path());
        let mut app = App::open(dir.path().to_path_buf(), actor());
        assert_eq!(app.rows.len(), 1);

        let mut graph = WorkGraph::load_dir(dir.path());
        let extra = graph.mint_id(WorkKind::Task, WorkId::ZERO, "write the tree");
        graph
            .upsert(
                extra,
                WorkFields {
                    kind: WorkKind::Task,
                    status: WorkStatus::Todo,
                    role: WorkRole::Unset,
                    parent: WorkId::ZERO,
                    actor: WorkId::ZERO,
                    summary: "write the tree",
                },
            )
            .unwrap();
        graph.save_dir(dir.path()).unwrap();

        app.poll();
        assert_eq!(app.rows.len(), 2);
    }

    #[test]
    fn the_cursor_stays_on_its_node_across_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let first = seeded(dir.path());
        let mut graph = WorkGraph::load_dir(dir.path());
        let second = graph.mint_id(WorkKind::Task, WorkId::ZERO, "write the tree");
        graph
            .upsert(
                second,
                WorkFields {
                    kind: WorkKind::Task,
                    status: WorkStatus::Todo,
                    role: WorkRole::Unset,
                    parent: WorkId::ZERO,
                    actor: WorkId::ZERO,
                    summary: "write the tree",
                },
            )
            .unwrap();
        graph.save_dir(dir.path()).unwrap();

        let mut app = App::open(dir.path().to_path_buf(), actor());
        let held = app.rows.iter().position(|row| row.id != first).unwrap();
        app.selected = held;
        let id = app.selected_id();
        app.reload();
        assert_eq!(app.selected_id(), id);
    }

    #[test]
    fn unlinking_with_no_edge_says_so_rather_than_writing() {
        let dir = tempfile::tempdir().unwrap();
        seeded(dir.path());
        let mut app = App::open(dir.path().to_path_buf(), actor());
        app.handle_key(key(KeyCode::Char('u')));
        assert_eq!(app.message, "no edge");
    }
}

#[cfg(test)]
mod reclaim_tests {
    use super::*;
    use claimdag::{WorkFields, WorkKind, WorkRole};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// The pane can act on the stall it is showing, without leaving the pane.
    #[test]
    fn the_pane_hands_back_a_claim_it_shows_as_quiet() {
        let dir = tempfile::tempdir().expect("tempdir");
        let holder = WorkId { hi: 7, lo: 7 };
        let mut graph = WorkGraph::default();
        let node = graph
            .upsert(
                WorkId::ZERO,
                WorkFields {
                    kind: WorkKind::Task,
                    status: claimdag::WorkStatus::Ready,
                    role: WorkRole::Implementor,
                    parent: WorkId::ZERO,
                    actor: holder,
                    summary: "the work",
                },
            )
            .expect("upsert");
        graph.claim(node, holder, None).expect("claim");
        graph.save_dir(dir.path()).expect("save");

        let mut app = App::open(dir.path().to_path_buf(), holder);
        // A live lease is not reclaimed, and the pane says so rather than
        // reporting a count of zero as if something happened.
        app.handle_key(key(KeyCode::Char('R')));
        assert!(
            app.message.contains("nothing has been quiet"),
            "{}",
            app.message
        );
        assert_eq!(
            WorkGraph::load_dir(dir.path())
                .get(node)
                .expect("node")
                .status,
            claimdag::WorkStatus::Claimed
        );

        // A zero lease is every claim past it, which is how this is tested
        // without waiting.
        app.lease = 0;
        app.handle_key(key(KeyCode::Char('R')));
        assert!(app.message.contains("handed back 1"), "{}", app.message);
        assert_eq!(
            WorkGraph::load_dir(dir.path())
                .get(node)
                .expect("node")
                .status,
            claimdag::WorkStatus::Ready
        );
    }

    /// A held node says how long it has been quiet, because the decision to
    /// hand it back cannot be made from a status that reads the same either
    /// way.
    #[test]
    fn a_held_node_shows_how_long_it_has_been_quiet() {
        let mut graph = WorkGraph::default();
        let holder = WorkId { hi: 3, lo: 3 };
        let node = graph
            .upsert(
                WorkId::ZERO,
                WorkFields {
                    kind: WorkKind::Task,
                    status: claimdag::WorkStatus::Ready,
                    role: WorkRole::Implementor,
                    parent: WorkId::ZERO,
                    actor: holder,
                    summary: "the work",
                },
            )
            .expect("upsert");
        let ready = graph.get(node).expect("node").clone();
        assert!(
            crate::forest::quiet_for(&ready).is_none(),
            "unheld work is not quiet"
        );

        graph.claim(node, holder, None).expect("claim");
        let held = graph.get(node).expect("node").clone();
        let quiet = crate::forest::quiet_for(&held).expect("a held node is quiet for a time");
        assert!(quiet.ends_with('s'), "{quiet}");
    }
}
