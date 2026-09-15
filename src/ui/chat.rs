use super::nick_color;
use crate::app::{App, LineKind, Mark};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

pub fn draw(f: &mut Frame, app: &mut App, area: Rect) {
    let win = &app.windows[app.active];
    let dim = Style::default().fg(Color::DarkGray);
    let lines: Vec<Line> = win
        .lines
        .iter()
        .map(|l| {
            let mut spans = vec![Span::styled(format!("{} ", l.time), dim)];
            match &l.kind {
                LineKind::Msg { nick, own, mark } => {
                    let style = if *own {
                        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(nick_color(nick))
                    };
                    spans.push(Span::styled(format!("<{nick}> "), style));
                    spans.push(Span::raw(l.text.clone()));
                    let green = Style::default().fg(Color::Green);
                    match mark {
                        Some(Mark::Pending) => spans.push(Span::styled(" ○", dim)),
                        Some(Mark::Heard) => spans.push(Span::styled(" ●", green)),
                        Some(Mark::Acked) => spans.push(Span::styled(" ●●", green)),
                        Some(Mark::Failed) => spans.push(Span::styled(" ●", Style::default().fg(Color::Red))),
                        None => {}
                    }
                }
                LineKind::Notice => {
                    spans.push(Span::styled("-!- ", Style::default().fg(Color::Blue)));
                    spans.push(Span::styled(l.text.clone(), Style::default().fg(Color::Gray)));
                }
                LineKind::Error => {
                    spans.push(Span::styled("-!- ", Style::default().fg(Color::Red)));
                    spans.push(Span::styled(l.text.clone(), Style::default().fg(Color::Red)));
                }
                LineKind::History => spans.push(Span::styled(l.text.clone(), dim)),
            }
            Line::from(spans)
        })
        .collect();

    let para = Paragraph::new(lines).wrap(Wrap { trim: false });
    let total = para.line_count(area.width);
    let height = area.height as usize;
    let bottom = total.saturating_sub(height);
    let offset = bottom.saturating_sub(win.scroll.min(bottom));
    f.render_widget(para.scroll((offset as u16, 0)), area);
}
