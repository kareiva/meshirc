use crate::app::{App, Focus};
use crate::contacts::{age, type_icon, TYPE_CHAT, TYPE_REPEATER, TYPE_ROOM};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use unicode_width::UnicodeWidthChar;

pub const WIDTH: u16 = 24;

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let now = app.now_unix();
    let inner_w = area.width.saturating_sub(1) as usize;
    let height = area.height.saturating_sub(1) as usize;
    app.sidebar_height = height;
    let focused = app.focus == Focus::Contacts;
    let sorted = app.contacts.sorted();
    let sel = app.selected_contact.and_then(|k| sorted.iter().position(|c| c.public_key == k));
    let start = match sel {
        Some(i) if height > 0 => i.saturating_sub(height / 2).min(sorted.len().saturating_sub(height)),
        _ => 0,
    };
    let mut lines = Vec::new();
    for (i, c) in sorted.iter().enumerate().skip(start).take(height.max(1)) {
        let a = age(now, app.contacts.last_advert(c));
        let icon_style = match c.contact_type {
            TYPE_CHAT => Style::default().fg(Color::Green),
            TYPE_REPEATER => Style::default().fg(Color::Yellow),
            TYPE_ROOM => Style::default().fg(Color::Magenta),
            _ => Style::default(),
        };
        let name_w = inner_w.saturating_sub(2 + a.len() + 1);
        let mut name = String::new();
        let mut w = 0;
        for ch in c.adv_name.chars() {
            let cw = ch.width().unwrap_or(0);
            if w + cw > name_w {
                break;
            }
            name.push(ch);
            w += cw;
        }
        let pad = name_w.saturating_sub(w);
        let mut line = Line::from(vec![
            Span::styled(format!("{} ", type_icon(c.contact_type)), icon_style),
            Span::raw(name),
            Span::raw(" ".repeat(pad + 1)),
            Span::styled(a, Style::default().fg(Color::DarkGray)),
        ]);
        if Some(i) == sel {
            line = line.style(if focused {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default().add_modifier(Modifier::BOLD)
            });
        }
        lines.push(line);
    }
    let title = format!(" {} contacts ", app.contacts.len());
    let title_style = if focused { Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD) } else { Style::default() };
    let block = Block::default().borders(Borders::LEFT).title(Span::styled(title, title_style));
    f.render_widget(Paragraph::new(lines).block(block), area);
}
