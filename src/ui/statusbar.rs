use crate::app::App;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn draw_bottom(f: &mut Frame, app: &App, area: Rect) {
    let base = Style::default().bg(Color::Blue).fg(Color::White);
    let mut spans = vec![Span::styled(
        format!(" {} ", chrono::Local::now().format("%H:%M")),
        base,
    )];
    let right = if app.connected { String::new() } else { "[disconnected] ".to_string() };
    let names: Vec<String> = app.windows.iter().map(|w| w.name()).collect();
    let fixed = spans[0].content.chars().count() + right.chars().count();
    let width_with = |max: usize| -> usize {
        names.iter().enumerate().map(|(i, n)| format!("{i}:{} ", shorten(n, max)).chars().count()).sum::<usize>() + fixed
    };
    let longest = names.iter().map(|n| n.chars().count()).max().unwrap_or(0);
    let mut max = longest;
    while max > 3 && width_with(max) > area.width as usize {
        max -= 1;
    }
    // unread windows turn red (bold for new messages, plain for notices)
    for (i, (n, w)) in names.iter().zip(&app.windows).enumerate() {
        let style = if i == app.active {
            base.add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else if w.activity >= 2 {
            base.fg(Color::LightRed).add_modifier(Modifier::BOLD)
        } else if w.activity > 0 {
            base.fg(Color::LightRed)
        } else {
            base
        };
        spans.push(Span::styled(format!("{i}:{}", shorten(n, max)), style));
        spans.push(Span::styled(" ", base));
    }
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    let pad = (area.width as usize).saturating_sub(used + right.chars().count());
    spans.push(Span::styled(" ".repeat(pad), base));
    spans.push(Span::styled(right, if app.connected { base } else { base.fg(Color::Red) }));
    f.render_widget(Paragraph::new(Line::from(spans)).style(base), area);
}

fn shorten(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        name.to_string()
    } else {
        let mut s: String = name.chars().take(max.saturating_sub(1)).collect();
        s.push('…');
        s
    }
}

pub fn draw_top(f: &mut Frame, app: &App, area: Rect) {
    let base = Style::default().bg(Color::Blue).fg(Color::White);
    let key = base.fg(Color::LightCyan);
    let mut spans = vec![Span::styled(" MeshIRC ", key.add_modifier(Modifier::BOLD))];
    let mut item = |label: &str, value: String| {
        if !label.is_empty() {
            spans.push(Span::styled(format!("{label} "), key));
        }
        spans.push(Span::styled(format!("{value}  "), base));
    };
    match &app.me {
        Some(me) => {
            item("", me.name.clone());
            item("@", app.cfg.port.clone());
            item(
                "radio",
                format!(
                    "{:.3} sf{} bw{}k cr{} {}dBm",
                    me.radio_freq as f64 / 1000.0,
                    me.sf,
                    me.radio_bw as f64 / 1000.0,
                    me.cr,
                    me.tx_power
                ),
            );
            let gps = if me.adv_lat == 0 && me.adv_lon == 0 {
                "none".to_string()
            } else {
                format!("{:.5},{:.5}", me.adv_lat as f64 / 1_000_000.0, me.adv_lon as f64 / 1_000_000.0)
            };
            item("gps", gps);
        }
        None => item("@", format!("{} (connecting)", app.cfg.port)),
    }
    if let Some((mv, pct)) = app.battery {
        item("bat", format!("{pct}% {:.2}V", mv as f32 / 1000.0));
    }
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    spans.push(Span::styled(" ".repeat((area.width as usize).saturating_sub(used)), base));
    f.render_widget(Paragraph::new(Line::from(spans)).style(base), area);
}
