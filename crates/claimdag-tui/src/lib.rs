//! Terminal work graph over a claimdag `work.bin`.
//!
//! The graph is a DAG over dependencies and a forest over parents. This pane
//! draws the forest, because that is the shape a person navigates, and it
//! mutates through [`claimdag::WorkGraph`] so the guards the command line
//! obeys are the guards here.

pub mod app;
pub mod forest;
pub mod handles;
pub mod seat;
pub mod view;

pub use app::{Action, App};
pub use forest::Row;
pub use handles::Handles;

/// Draw the pane until the reader quits.
///
/// # Errors
///
/// Returns an error if the terminal cannot be put into raw mode or drawn to.
pub fn run(dir: std::path::PathBuf) -> std::io::Result<()> {
    let mut app = App::open(dir, seat::actor());
    let mut terminal = view::install()?;
    let result = (|| {
        loop {
            terminal.draw(|frame| view::draw(frame, &app))?;
            if ratatui::crossterm::event::poll(std::time::Duration::from_millis(250))? {
                if let ratatui::crossterm::event::Event::Key(key) =
                    ratatui::crossterm::event::read()?
                {
                    if app.handle_key(key) == Action::Quit {
                        break;
                    }
                }
            } else {
                // Nothing pressed: a claim made from another seat shows up
                // without anybody having to know to press r.
                app.poll();
            }
        }
        Ok(())
    })();
    view::restore()?;
    result
}
