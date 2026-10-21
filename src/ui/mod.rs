mod chat;
mod sidebar;
mod statusbar;

use crate::app::{App, Focus};
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

pub fn draw(f: &mut Frame, app: &mut App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3), Constraint::Length(1), Constraint::Length(1)])
        .split(f.area());
    let sidebar_w = if app.show_sidebar { sidebar::WIDTH } else { 0 };
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(20), Constraint::Length(sidebar_w)])
        .split(rows[1]);

    app.page_height = cols[0].height as usize;
    statusbar::draw_top(f, app, rows[0]);
    chat::draw(f, app, cols[0]);
    if app.show_sidebar {
        sidebar::draw(f, app, cols[1]);
    }
    statusbar::draw_bottom(f, app, rows[2]);
    draw_input(f, app, rows[3]);
    draw_mentions(f, app, cols[0], rows[3]);
    draw_confirm_remove(f, app, rows[1]);
}

/// "Remove <name> from the contact list? [y/N]" dialog, centred over the chat + sidebar.
fn draw_confirm_remove(f: &mut Frame, app: &App, area: Rect) {
    let Some(name) = app.confirm_remove.and_then(|k| app.contacts.by_prefix(&k)).map(|c| c.adv_name.clone()) else { return };
    let text = vec![
        Line::from(vec![Span::raw("Remove "), Span::styled(name, Style::default().fg(Color::Yellow)), Span::raw(" from the contact list?")]),
        Line::from(""),
        Line::from(vec![Span::styled("y", Style::default().fg(Color::Green)), Span::raw(" remove   any other key: cancel")]),
    ];
    let width = (text.iter().map(|l| l.width()).max().unwrap_or(0) as u16 + 4).min(area.width);
    let height = 5.min(area.height);
    let popup = Rect::new(area.x + (area.width - width) / 2, area.y + (area.height - height) / 2, width, height);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .title(" Remove contact ");
    f.render_widget(Clear, popup);
    f.render_widget(Paragraph::new(text).block(block).wrap(Wrap { trim: false }), popup);
}

const MENTION_ROWS: usize = 8;

/// Popup with the contacts matching the `@` mention being typed, anchored
/// above the `@` in the input line.
fn draw_mentions(f: &mut Frame, app: &App, chat: Rect, input: Rect) {
    let Some(m) = app.mention else { return };
    let cands = app.mention_candidates();
    if cands.is_empty() || chat.height == 0 {
        return;
    }
    let height = cands.len().min(MENTION_ROWS).min(chat.height as usize);
    let first = m.selected.saturating_sub(height - 1).min(cands.len() - height);
    let width = cands.iter().map(|c| c.chars().count()).max().unwrap_or(0) as u16 + 2;
    let width = width.min(chat.width).max(1);
    let prompt_w = prompt(app).chars().count();
    let input_w = input.width.saturating_sub(prompt_w as u16 + 1) as usize;
    let scroll = app.input.visual_scroll(input_w);
    let at: usize = app.input.value().chars().take(m.start).map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)).sum();
    let x = (input.x as usize + prompt_w + at.saturating_sub(scroll)) as u16;
    let x = x.min(chat.right().saturating_sub(width)).max(chat.x);
    let area = Rect::new(x, chat.bottom() - height as u16, width, height as u16);
    let lines: Vec<Line> = cands
        .iter()
        .enumerate()
        .skip(first)
        .take(height)
        .map(|(i, c)| {
            let style = if i == m.selected {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::White).bg(Color::DarkGray)
            };
            Line::from(Span::styled(format!(" {c:<w$} ", w = width as usize - 2), style))
        })
        .collect();
    f.render_widget(Clear, area);
    f.render_widget(Paragraph::new(lines), area);
}

fn prompt(app: &App) -> String {
    match app.focus {
        Focus::Input => format!("[{}] ", app.windows[app.active].name()),
        Focus::Chat => format!("[{}|scroll] ", app.windows[app.active].name()),
        Focus::Contacts => format!("[{}|contacts] ", app.windows[app.active].name()),
    }
}

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    let prompt = prompt(app);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use meshcore_rs::events::Contact;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn mention_popup_renders_above_input() {
        let dir = std::env::temp_dir().join(format!("meshirc-ui-test-{}", std::process::id()));
        let cfg = Config { port: String::new(), baud: 0, log_dir: dir.join("logs"), data_dir: dir, history_lines: 0, save_private: true, http_server: false, http_private_windows: false, http_bind: String::new(), auto_join: vec![] };
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        std::mem::forget(rx);
        let mut app = App::new(cfg, tx).unwrap();
        for (i, name) in ["Alice Home", "Alice Work", "Bob"].iter().enumerate() {
            let mut public_key = [0u8; 32];
            public_key[0] = i as u8 + 1;
            app.contacts.upsert(Contact {
                public_key,
                contact_type: 1,
                flags: 0,
                path_len: -1,
                out_path: vec![],
                adv_name: name.to_string(),
                last_advert: 0,
                adv_lat: 0,
                adv_lon: 0,
                last_modification_timestamp: 0,
            });
        }
        for c in "hi @al".chars() {
            app.handle(crate::event::AppEvent::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
        }
        app.handle(crate::event::AppEvent::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));

        let mut term = Terminal::new(TestBackend::new(60, 10)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer();
        let row = |y: u16| -> String { (0..60).map(|x| buf[(x, y)].symbol().to_string()).collect() };
        // popup sits on the last two chat rows, aligned with the '@' (prompt "[status] " + "hi ")
        assert!(row(6).starts_with("             Alice Home "), "{:?}", row(6));
        assert!(row(7).starts_with("             Alice Work "), "{:?}", row(7));
        assert_eq!(buf[(12, 7)].bg, Color::Cyan, "selected row highlighted");
        assert_eq!(buf[(12, 6)].bg, Color::DarkGray);
        assert!(row(9).starts_with("[status] hi @al"), "{:?}", row(9));

        let mut bob = [0u8; 32];
        bob[0] = 3;
        app.confirm_remove = Some(bob);
        let mut term = Terminal::new(TestBackend::new(80, 12)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer();
        let screen: Vec<String> = (0..12).map(|y| (0..80).map(|x| buf[(x, y)].symbol().to_string()).collect()).collect();
        assert!(screen.iter().any(|r| r.contains("Remove contact")), "{screen:#?}");
        assert!(screen.iter().any(|r| r.contains("Remove Bob from the contact list?")), "{screen:#?}");
    }
}
