use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Tabs},
    Frame,
};

use super::app::{App, AppMode};

/// Main render function called every frame
pub fn render(f: &mut Frame, app: &mut App) {
    let size = f.area();

    // Overall layout: header / tabs / table / statusbar
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Length(3), // tabs
            Constraint::Min(5),    // node table
            Constraint::Length(3), // status / keys
        ])
        .split(size);

    render_header(f, app, chunks[0]);
    render_tabs(f, app, chunks[1]);
    render_node_table(f, app, chunks[2]);
    render_statusbar(f, app, chunks[3]);
}

// ─── Header ───────────────────────────────────────────────────────────────────

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let (status_text, status_color) = if app.state.running {
        ("● RUNNING", Color::Green)
    } else {
        ("○ STOPPED", Color::Red)
    };

    let mode_indicator = match app.mode {
        AppMode::Testing => "  [测速中...]",
        AppMode::AutoBest => "  [自动选优...]",
        AppMode::Normal => "",
    };

    let title_line = Line::from(vec![
        Span::styled(status_text, Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
        Span::raw(format!(
            "  HTTP:{}  SOCKS:{}  API:{}",
            app.state.http_port, app.state.socks_port, app.state.control_port
        )),
        Span::styled(mode_indicator, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Ladder v2 ");
    let paragraph = Paragraph::new(title_line)
        .block(block)
        .alignment(Alignment::Left);
    f.render_widget(paragraph, area);
}

// ─── Tabs ─────────────────────────────────────────────────────────────────────

fn render_tabs(f: &mut Frame, app: &App, area: Rect) {
    let tab_titles: Vec<Line> = app
        .tabs
        .iter()
        .map(|t| Line::from(t.label()))
        .collect();

    let tabs = Tabs::new(tab_titles)
        .block(Block::default().borders(Borders::ALL).title(" 订阅分组 "))
        .select(app.active_tab)
        .style(Style::default().fg(Color::Gray))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        );
    f.render_widget(tabs, area);
}

// ─── Node Table ───────────────────────────────────────────────────────────────

fn render_node_table(f: &mut Frame, app: &mut App, area: Rect) {
    let visible = app.visible_nodes();

    let header = Row::new(vec![
        Cell::from(" ").style(Style::default().fg(Color::DarkGray)),
        Cell::from("节点名称").style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Cell::from("延迟").style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Cell::from("订阅").style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
    ])
    .height(1)
    .bottom_margin(1);

    let rows: Vec<Row> = visible
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let is_selected = i == app.cursor;

            // Status icon
            let icon = if node.is_current {
                "▶"
            } else {
                " "
            };

            // Delay display
            let (delay_str, delay_color) = if node.delay == 0 {
                ("timeout".to_string(), Color::Red)
            } else if node.delay < 100 {
                (format!("{}ms", node.delay), Color::Green)
            } else if node.delay < 300 {
                (format!("{}ms", node.delay), Color::Yellow)
            } else {
                (format!("{}ms", node.delay), Color::Red)
            };

            let row_style = if is_selected {
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else if node.is_current {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            };

            Row::new(vec![
                Cell::from(icon).style(Style::default().fg(if node.is_current { Color::Green } else { Color::DarkGray })),
                Cell::from(node.name.as_str()),
                Cell::from(delay_str).style(Style::default().fg(delay_color)),
                Cell::from(node.subscription.as_str()).style(Style::default().fg(Color::DarkGray)),
            ])
            .style(row_style)
            .height(1)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),
            Constraint::Min(20),
            Constraint::Length(10),
            Constraint::Length(15),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" 节点列表 ({}) ", visible.len())),
    )
    .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut table_state = TableState::default().with_selected(Some(app.cursor));
    f.render_stateful_widget(table, area, &mut table_state);
}

// ─── Status Bar ───────────────────────────────────────────────────────────────

fn render_statusbar(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(30), Constraint::Length(60)])
        .split(area);

    // Left: status message
    let status_text = if let Some(msg) = &app.status_msg {
        msg.as_str()
    } else if let Some(node) = &app.current_node {
        &format!("当前节点: {}", node)
    } else {
        "无当前节点"
    };

    let status_line = app.status_msg.as_deref().map(|_| {
        Paragraph::new(status_text)
            .style(Style::default().fg(Color::Yellow))
            .block(Block::default().borders(Borders::ALL))
    }).unwrap_or_else(|| {
        Paragraph::new(status_text)
            .style(Style::default().fg(Color::Gray))
            .block(Block::default().borders(Borders::ALL))
    });
    f.render_widget(status_line, chunks[0]);

    // Right: key hints
    let keys = Line::from(vec![
        key_hint("↑↓", "移动"),
        Span::raw(" "),
        key_hint("Enter", "切换"),
        Span::raw(" "),
        key_hint("Tab", "分组"),
        Span::raw(" "),
        key_hint("T", "测速"),
        Span::raw(" "),
        key_hint("A", "自动最优"),
        Span::raw(" "),
        key_hint("U", "更新订阅"),
        Span::raw(" "),
        key_hint("Q", "退出"),
    ]);
    let keys_para = Paragraph::new(keys)
        .block(Block::default().borders(Borders::ALL))
        .alignment(Alignment::Center);
    f.render_widget(keys_para, chunks[1]);
}

fn key_hint(key: &str, desc: &str) -> Span<'static> {
    // We need owned strings here
    let text = format!("[{}]{}", key, desc);
    Span::styled(
        text,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
}
