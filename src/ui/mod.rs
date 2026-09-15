mod chat;
mod sidebar;
mod statusbar;

use crate::app::{App, Focus};
use ratatui::layout::{Constraint, Direction, Layout, Position};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn draw(f: &mut Frame, app: &mut App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1), Constraint::Length(1)])
        .split(f.area());
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(sidebar::WIDTH)])
        .split(rows[1]);

    app.page_height = cols[0].height as usize;
    statusbar::draw_top(f, app, rows[0]);
    chat::draw(f, app, cols[0]);
    sidebar::draw(f, app, cols[1]);
    statusbar::draw_bottom(f, app, rows[2]);
    draw_input(f, app, rows[3]);
}

fn draw_input(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let prompt = match app.focus {
        Focus::Input => format!("[{}] ", app.windows[app.active].name()),
        Focus::Chat => format!("[{}|scroll] ", app.windows[app.active].name()),
        Focus::Contacts => format!("[{}|contacts] ", app.windows[app.active].name()),
    };
    let width = area.width.saturating_sub(prompt.chars().count() as u16 + 1) as usize;
    let scroll = app.input.visual_scroll(width);
    let prompt_style = if app.focus == Focus::Input { Style::default().fg(Color::Cyan) } else { Style::default().fg(Color::Yellow) };
    let line = Line::from(vec![
        Span::styled(prompt.clone(), prompt_style),
        Span::raw(app.input.value()),
    ]);
    f.render_widget(Paragraph::new(line).scroll((0, scroll as u16)), area);
    if app.focus == Focus::Input {
        let x = area.x + prompt.chars().count() as u16 + (app.input.visual_cursor().saturating_sub(scroll)) as u16;
        f.set_cursor_position(Position::new(x.min(area.right().saturating_sub(1)), area.y));
    }
}

pub fn nick_color(nick: &str) -> Color {
    const PALETTE: [Color; 8] = [
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::LightGreen,
        Color::LightBlue,
        Color::LightMagenta,
    ];
    let h = nick.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    PALETTE[(h % PALETTE.len() as u32) as usize]
}
