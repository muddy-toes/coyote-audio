//! Terminal UI module using ratatui

mod app;
mod ui;

pub use app::{App, AppEvent, InputMode, OutputValues, Panel, ParameterSelection};
pub use ui::draw;
