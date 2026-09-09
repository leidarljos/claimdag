//! Draw the pane. No graph logic.
//!
//! Nothing here names a colour. The terminal's own palette is the seat's
//! theme, so the pane matches whatever else is on the screen and there is no
//! second place for light and dark to disagree.

use std::io::{self, stdout, Stdout};

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::{Frame, Terminal};

use crate::app::{App, HELP};

/// Crossterm terminal used by [`install`].
pub type CrosstermTerm = Terminal<CrosstermBackend<Stdout>>;

/// Enter raw mode and the alternate screen.
///
/// # Errors
///
/// Returns an error if the terminal cannot enter raw mode, cannot switch to
/// the alternate screen, or cannot be wrapped as a ratatui backend.
pub fn install() -> io::Result<CrosstermTerm> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(out))
}

/// Leave raw mode and the alternate screen.
///
/// # Errors
///
/// Returns an error if the terminal cannot leave raw mode or the alternate
/// screen.
pub fn restore() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    Ok(())
}

/// Draw the title, the forest, and the status line.
pub fn draw(frame: &mut Frame, app: &App) {
    let [head, body, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    frame.render_widget(
        Paragraph::new(Line::from(format!("work  {}", app.dir().display()))),
        head,
    );

    let items: Vec<ListItem> = if app.rows.is_empty() {
        vec![ListItem::new("(empty)")]
    } else {
        app.rows
            .iter()
            .map(|row| ListItem::new(format!("{}{}", "  ".repeat(row.depth), row.label)))
            .collect()
    };
    let mut state = ListState::default();
    if !app.rows.is_empty() {
        state.select(Some(app.selected.min(app.rows.len() - 1)));
    }
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        body,
        &mut state,
    );

    let line = if app.message.is_empty() {
        HELP.to_string()
    } else {
        format!("{}  |  {HELP}", app.message)
    };
    frame.render_widget(Paragraph::new(Line::from(line)), status);
}

#[cfg(test)]
mod tests {
    use super::*;
    use claimdag::{WorkFields, WorkGraph, WorkId, WorkKind, WorkRole, WorkStatus};
    use ratatui::backend::TestBackend;

    fn rendered(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn an_empty_graph_draws_the_bindings_and_says_it_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let app = App::open(dir.path().to_path_buf(), WorkId::ZERO);
        let screen = rendered(&app, 90, 10);
        assert!(screen.contains("(empty)"), "{screen}");
        assert!(screen.contains("c claim"), "{screen}");
    }

    #[test]
    fn a_child_draws_indented_under_its_parent() {
        let dir = tempfile::tempdir().unwrap();
        let mut graph = WorkGraph::default();
        let root = graph.mint_id(WorkKind::Task, WorkId::ZERO, "land the adapter");
        let fields = |parent, summary| WorkFields {
            kind: WorkKind::Task,
            status: WorkStatus::Todo,
            role: WorkRole::Unset,
            parent,
            actor: WorkId::ZERO,
            summary,
        };
        graph
            .upsert(root, fields(WorkId::ZERO, "land the adapter"))
            .unwrap();
        let kid = graph.mint_id(WorkKind::Task, root, "write the tree");
        graph.upsert(kid, fields(root, "write the tree")).unwrap();
        graph.save_dir(dir.path()).unwrap();

        let app = App::open(dir.path().to_path_buf(), WorkId::ZERO);
        let screen = rendered(&app, 90, 10);
        let parent_line = screen
            .lines()
            .find(|line| line.contains("land the adapter"))
            .unwrap();
        let child_line = screen
            .lines()
            .find(|line| line.contains("write the tree"))
            .unwrap();
        let column = |line: &str, needle: &str| line.find(needle).unwrap();
        assert_eq!(
            column(child_line, "write the tree") - column(parent_line, "land the adapter"),
            2,
            "{screen}"
        );
    }
}
