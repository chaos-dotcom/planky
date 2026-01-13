// tui.rs

use crate::app::{App, InputMode};
use chrono::{
    Datelike, Duration as Dur, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike,
    Weekday,
};
use crossterm::event::{self, Event as CEvent, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Terminal,
    backend::Backend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Line},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};
use std::{io, time::Duration};
use std::io::Write;
use std::process::{Command, Stdio};
use textwrap::wrap;


pub fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> io::Result<()>
where
    std::io::Error: From<<B as Backend>::Error>,
{
    app.start_background_sync();

    // Eagerly resolve lists for the current project so [w] is shown immediately
    if app.planka_lists_by_board.get(&app.current_project).is_none() {
        if let Ok(client) = app.ensure_planka_client() {
            // Ensure boards are cached so the header can show "Project - Board"
            if app.planka_boards.is_empty() {
                if let Ok(boards) = client.fetch_boards() {
                    app.planka_boards = boards;
                }
            }
            if let Ok(lists) = client.resolve_lists(&app.current_project) {
                app.planka_lists_by_board
                    .insert(app.current_project.clone(), lists.clone());
                app.planka_lists = Some(lists);
            }
        }
    }

    loop {
        // process inbound updates and retry queued outbound ops
        app.drain_inbound();
        app.process_pending_ops_tick();
        terminal.draw(|f| ui(f, app))?;

        if crossterm::event::poll(Duration::from_millis(100))? {
            if let CEvent::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match app.input_mode {
                    InputMode::Normal => match key.code {
                        KeyCode::Char('q') => return Ok(()),
                        KeyCode::Char('a') => {
                            app.input_mode = InputMode::EditingDescription;
                            app.input_description.clear();
                            app.input_due_date.clear();
                            app.error_message = None;
                        }
                        KeyCode::Char('e') => {
                            app.begin_edit_selected();
                        }
                        // Delete selected todo (Shift+R only)
                        KeyCode::Char('r') | KeyCode::Char('R') if key.modifiers.contains(KeyModifiers::SHIFT) => {
                            app.delete_todo();
                        }
                        KeyCode::Char('d') => app.mark_done(),
                        KeyCode::Char('w') => {
                            app.mark_doing();
                        }
                        KeyCode::Char('?') => {
                            app.input_mode = InputMode::Searching;
                            app.search_query.clear();
                        }
                        KeyCode::Down => {
                            if app.selected < filtered_todos(app).len().saturating_sub(1) {
                                app.selected += 1;
                            }
                        }
                        KeyCode::Up => {
                            if app.selected > 0 {
                                app.selected -= 1;
                            }
                        }
                        KeyCode::Enter => {
                            app.open_selected_card();
                        }
                        KeyCode::Char('c') => {
                            let list = filtered_todos(app);
                            if let Some(todo) = list.get(app.selected) {
                                if let Err(e) = copy_to_clipboard(&todo.description) {
                                    app.error_message = Some(format!("Copy failed: {}", e));
                                } else {
                                    app.error_message = Some("Copied task to clipboard".to_string());
                                }
                            }
                        }
                        KeyCode::Char('p') => {
                            match paste_from_clipboard() {
                                Ok(mut text) => {
                                    // trim trailing newlines from clipboard content
                                    while text.ends_with('\n') || text.ends_with('\r') {
                                        text.pop();
                                    }
                                    app.input_description = text;
                                    app.input_mode = InputMode::EditingDescription;
                                    app.error_message = None;
                                }
                                Err(e) => {
                                    app.error_message = Some(format!("Paste failed: {}", e));
                                }
                            }
                        }
                        KeyCode::Char(']') => {
                            app.next_project();
                            app.selected = 0;
                        }
                        KeyCode::Char('[') => {
                            app.prev_project();
                            app.selected = 0;
                        }
                        KeyCode::Char('l') => {
                            app.input_mode = InputMode::EditingProject;
                            app.input_project = app.current_project.clone();
                            app.error_message = None;
                        }
                        KeyCode::Char('S') => {
                            app.sync_all_projects_from_planka();
                        }
                        KeyCode::Char('L') => {
                            app.start_planka_setup();
                        }
                        KeyCode::Tab => {
                            app.input_mode = InputMode::ControlCenter;
                            app.error_message = None;
                        }
                        _ => {}
                    },
                    InputMode::EditingDescription => {
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && matches!(key.code, KeyCode::Char('v') | KeyCode::Char('V'))
                        {
                            match paste_from_clipboard() {
                                Ok(mut text) => {
                                    while text.ends_with('\n') || text.ends_with('\r') {
                                        text.pop();
                                    }
                                    app.input_description.push_str(&text);
                                }
                                Err(e) => app.error_message = Some(format!("Paste failed: {}", e)),
                            }
                            continue;
                        }
                        match key.code {
                        KeyCode::Enter => {
                            app.input_mode = InputMode::EditingDueDate;
                        }
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Char(c) => {
                            app.input_description.push(c);
                        }
                        KeyCode::Backspace => {
                            app.input_description.pop();
                        }
                        _ => {}
                    }},
                    InputMode::EditingDueDate => match key.code {
                        KeyCode::Enter => {
                            if app.editing_index.is_some() {
                                match app.save_edit() {
                                    Ok(_) => app.input_mode = InputMode::Normal,
                                    Err(e) => app.error_message = Some(e),
                                }
                            } else {
                                match app.add_todo() {
                                    Ok(_) => app.input_mode = InputMode::Normal,
                                    Err(e) => app.error_message = Some(e),
                                }
                            }
                        }
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Char(c) => {
                            app.input_due_date.push(c);
                        }
                        KeyCode::Backspace => {
                            app.input_due_date.pop();
                        }
                        _ => {}
                    },
                    InputMode::EditingProject => match key.code {
                        KeyCode::Enter => {
                            let name = app.input_project.clone();
                            app.set_current_project(name);
                            app.selected = 0;
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Char(c) => {
                            app.input_project.push(c);
                        }
                        KeyCode::Backspace => {
                            app.input_project.pop();
                        }
                        _ => {}
                    },
                    InputMode::EditingPlanka => match key.code {
                        KeyCode::Enter => {
                            app.submit_planka_setup();
                        }
                        KeyCode::Esc => {
                            app.planka_setup = None;
                            app.input_planka.clear();
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Char(c) => {
                            app.input_planka.push(c);
                        }
                        KeyCode::Backspace => {
                            app.input_planka.pop();
                        }
                        _ => {}
                    },
                    InputMode::CreatingBoard => match key.code {
                        KeyCode::Enter => {
                            match app.submit_create_board() {
                                Ok(_) => {}
                                Err(e) => app.error_message = Some(e),
                            }
                        }
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Left | KeyCode::Char('[') => {
                            if !app.create_board_projects.is_empty() {
                                if app.create_board_project_index == 0 {
                                    app.create_board_project_index = app.create_board_projects.len() - 1;
                                } else {
                                    app.create_board_project_index -= 1;
                                }
                            }
                        }
                        KeyCode::Right | KeyCode::Char(']') => {
                            if !app.create_board_projects.is_empty() {
                                app.create_board_project_index = (app.create_board_project_index + 1) % app.create_board_projects.len();
                            }
                        }
                        KeyCode::Char(c) => {
                            app.input_board.push(c);
                        }
                        KeyCode::Backspace => {
                            app.input_board.pop();
                        }
                        _ => {}
                    },
                    InputMode::CreatingProject => match key.code {
                        KeyCode::Enter => {
                            match app.submit_create_project() {
                                Ok(_) => {}
                                Err(e) => app.error_message = Some(e),
                            }
                        }
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Char(c) => {
                            app.input_project.push(c);
                        }
                        KeyCode::Backspace => {
                            app.input_project.pop();
                        }
                        _ => {}
                    },
                    InputMode::ViewingCard => match key.code {
                        KeyCode::Esc => {
                            app.close_view();
                        }
                        KeyCode::Up => {
                            app.view_scroll = app.view_scroll.saturating_sub(1);
                        }
                        KeyCode::Down => {
                            app.view_scroll = app.view_scroll.saturating_add(1);
                        }
                        KeyCode::PageUp => {
                            app.view_scroll = app.view_scroll.saturating_sub(10);
                        }
                        KeyCode::PageDown => {
                            app.view_scroll = app.view_scroll.saturating_add(10);
                        }
                        _ => {}
                    },
                    InputMode::ControlCenter => match key.code {
                        KeyCode::Esc | KeyCode::Tab => {
                            app.input_mode = InputMode::Normal;
                        }
                        KeyCode::Down => {
                            if app.control_center_index < 4 { app.control_center_index += 1; }
                        }
                        KeyCode::Up => {
                            if app.control_center_index > 0 { app.control_center_index -= 1; }
                        }
                        KeyCode::Enter => {
                            match app.control_center_index {
                                0 => { app.begin_create_board(); }      // New board
                                1 => { app.begin_create_project(); }    // New project
                                2 => { app.start_planka_setup(); app.input_mode = InputMode::EditingPlanka; }
                                3 => { app.sync_all_projects_from_planka(); app.input_mode = InputMode::Normal; }
                                4 => { app.input_mode = InputMode::Normal; }
                                _ => {}
                            }
                        }
                        _ => {}
                    },
                    InputMode::Searching => match key.code {
                        KeyCode::Esc => {
                            app.input_mode = InputMode::Normal;
                            app.search_query.clear();
                        }
                        KeyCode::Char(c) => {
                            app.search_query.push(c);
                            app.selected = 0;
                        }
                        KeyCode::Backspace => {
                            app.search_query.pop();
                            app.selected = 0;
                        }
                        KeyCode::Down => {
                            if app.selected < filtered_todos(app).len().saturating_sub(1) {
                                app.selected += 1;
                            }
                        }
                        KeyCode::Up => {
                            if app.selected > 0 {
                                app.selected -= 1;
                            }
                        }
                        _ => {}
                    },
                }
            }
        }
    }
}

fn filtered_todos(app: &App) -> Vec<&crate::todo::Todo> {
    let base = app
        .todos
        .iter()
        .filter(|t| t.project == app.current_project)
        .filter(|t| {
            if app.search_query.is_empty() {
                true
            } else {
                let q = app.search_query.to_lowercase();
                t.description.to_lowercase().contains(&q)
                    || t.due_date
                        .as_ref()
                        .map(|d| d.to_lowercase().contains(&q))
                        .unwrap_or(false)
            }
        });

    let doing_id = app
        .planka_lists_by_board
        .get(&app.current_project)
        .map(|l| l.doing_list_id.as_str());
    let done_id = app
        .planka_lists_by_board
        .get(&app.current_project)
        .map(|l| l.done_list_id.as_str());

    let mut doing: Vec<&crate::todo::Todo> = Vec::new();
    let mut todo: Vec<&crate::todo::Todo> = Vec::new();
    let mut done: Vec<&crate::todo::Todo> = Vec::new();

    for t in base {
        let in_doing = doing_id
            .map(|id| t.planka_list_id.as_deref() == Some(id))
            .unwrap_or(false);
        let in_done = done_id
            .map(|id| t.planka_list_id.as_deref() == Some(id))
            .unwrap_or(false);

        if !t.done && in_doing {
            doing.push(t);
        } else if t.done || in_done {
            done.push(t);
        } else {
            todo.push(t);
        }
    }

    doing.into_iter().chain(todo).chain(done).collect()
}

fn ui(f: &mut ratatui::Frame<'_>, app: &App) {
    let size = f.area();

    if matches!(app.input_mode, InputMode::ViewingCard) {
        let area = f.area();
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(area);

        // Simple tabs header (Tasks active)
        let tasks_style = Style::default().fg(Color::White).bg(Color::Blue).add_modifier(Modifier::BOLD);
        let tools_style = Style::default();
        let tabs_line = Line::from(vec![
            Span::styled(" Tasks ", tasks_style),
            Span::raw(" "),
            Span::styled(" Tools ", tools_style),
        ]);
        let tabs = Paragraph::new(tabs_line).alignment(Alignment::Left);
        f.render_widget(tabs, rows[0]);

        let mut lines: Vec<Line> = Vec::new();
        if let Some(d) = app.view_card.as_ref() {
            lines.push(Line::from(Span::styled(&d.name, Style::default().add_modifier(Modifier::BOLD))));
            if let Some(ref ln) = d.list_name {
                lines.push(Line::from(format!("List: {}", ln)));
            }
            if let Some(ref due) = d.due {
                lines.push(Line::from(format!("Due: {}", due)));
            }
            if let Some(c) = d.is_due_completed {
                lines.push(Line::from(format!("Due completed: {}", if c { "yes" } else { "no" })));
            }
            if let Some(ref c) = d.created {
                lines.push(Line::from(format!("Created: {}", c)));
            }
            if let Some(ref u) = d.updated {
                lines.push(Line::from(format!("Updated: {}", u)));
            }
            if !d.labels.is_empty() {
                lines.push(Line::from(format!("Labels: {}", d.labels.join(", "))));
            }
            if !d.attachments.is_empty() {
                lines.push(Line::from("Attachments:"));
                for a in &d.attachments {
                    lines.push(Line::from(format!("  • {}", a)));
                }
            }
            if !d.tasks.is_empty() {
                lines.push(Line::from("Checklist:"));
                for (name, done) in &d.tasks {
                    let mark = if *done { "[x]" } else { "[ ]" };
                    lines.push(Line::from(format!("  {} {}", mark, name)));
                }
            }
            if let Some(ref desc) = d.description {
                lines.push(Line::from("")); // spacer
                lines.push(Line::from("Description:"));
                for l in textwrap::wrap(desc, rows[1].width.saturating_sub(4) as usize) {
                    lines.push(Line::from(format!("  {}", l)));
                }
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "Esc close • Up/Down/PageUp/PageDown scroll",
                Style::default().fg(Color::Blue),
            )));
        } else {
            lines.push(Line::from("No card loaded"));
        }

        let details = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title("Card"))
            .wrap(Wrap { trim: true })
            .scroll((app.view_scroll, 0));
        f.render_widget(details, rows[1]);
        return;
    }

    if matches!(app.input_mode, InputMode::ControlCenter) {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(size);

        // Tabs row
        let tasks_style = Style::default();
        let tools_style = Style::default().fg(Color::White).bg(Color::Blue).add_modifier(Modifier::BOLD);
        let tabs_line = Line::from(vec![
            Span::styled(" Tasks ", tasks_style),
            Span::raw(" "),
            Span::styled(" Tools ", tools_style),
        ]);
        let tabs = Paragraph::new(tabs_line).alignment(Alignment::Left);
        f.render_widget(tabs, rows[0]);

        // Tools list
        let items = ["New board", "New project", "Login/setup", "Sync all projects", "Back to tasks"];
        let list_items: Vec<ListItem> = items.iter().enumerate().map(|(i, label)| {
            let style = if i == app.control_center_index {
                Style::default().fg(Color::White).bg(Color::Blue).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(*label, style)))
        }).collect();
        let mut state = ratatui::widgets::ListState::default();
        state.select(Some(app.control_center_index));
        let list = List::new(list_items)
            .block(Block::default().borders(Borders::ALL).title("Tools"))
            .highlight_symbol(">> ");
        f.render_stateful_widget(list, rows[1], &mut state);
        return;
    }

    let mut constraints = vec![
        Constraint::Length(1), // tabs
        Constraint::Length(3), // title
        Constraint::Length(3), // help
        Constraint::Min(1),    // todo list
    ];
    let needs_input = matches!(
        app.input_mode,
        InputMode::EditingDescription
            | InputMode::EditingDueDate
            | InputMode::EditingProject
            | InputMode::EditingPlanka
            | InputMode::CreatingBoard
            | InputMode::CreatingProject
            | InputMode::Searching
    );
    if needs_input {
        constraints.push(Constraint::Length(3)); // one input line only
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints(constraints)
        .split(size);

    let tasks_style = Style::default().fg(Color::White).bg(Color::Blue).add_modifier(Modifier::BOLD);
    let tools_style = Style::default();
    let tabs_line = Line::from(vec![
        Span::styled(" Tasks ", tasks_style),
        Span::raw(" "),
        Span::styled(" Tools ", tools_style),
    ]);
    let tabs = Paragraph::new(tabs_line).alignment(Alignment::Left);
    f.render_widget(tabs, chunks[0]);

    let board_name = &app.current_project;
    let project_name = app
        .planka_boards
        .iter()
        .find(|b| b.name == *board_name)
        .and_then(|b| b.project_name.as_deref())
        .map(|s| s.to_string());

    let mut title_text = if let Some(pn) = project_name {
        format!("🌈 {} - {} 🥰", pn, board_name)
    } else {
        format!("🌈 {} 🥰", board_name)
    };
    if app.pending_ops_len() > 0 {
        title_text = format!("{}🏴‍☠️ ⇅{}", title_text, app.pending_ops_len());
    }
    let title = Paragraph::new(Line::from(Span::styled(
        title_text,
        Style::default().add_modifier(Modifier::BOLD),
    )))
    .alignment(Alignment::Center);
    f.render_widget(title, chunks[1]);

    let b = Style::default().add_modifier(Modifier::BOLD);
    let help = Paragraph::new(vec![
        Line::from(vec![
            Span::raw("Press "),
            Span::styled("a", b), Span::raw(" add, "),
            Span::styled("e", b), Span::raw(" edit, "),
            Span::styled("Enter", b), Span::raw(" open, "),
            Span::styled("d", b), Span::raw(" done, "),
            Span::styled("w", b), Span::raw(" doing, "),
            Span::raw("Shift+"), Span::styled("R", b), Span::raw(" delete, "),
            Span::styled("c", b), Span::raw(" copy, "),
            Span::styled("p", b), Span::raw(" paste"),
        ]),
        Line::from(vec![
            Span::styled("?", b), Span::raw(" search, "),
            Span::styled("[", b), Span::raw(" prev, "),
            Span::styled("]", b), Span::raw(" next project, "),
            Span::styled("l", b), Span::raw(" set project, "),
            Span::styled("S", b), Span::raw(" sync, "),
            Span::styled("L", b), Span::raw(" login, "),
            Span::styled("Tab", b), Span::raw(" tools, "),
            Span::styled("q", b), Span::raw(" quit"),
        ]),
    ])
    .alignment(Alignment::Center);
    f.render_widget(help, chunks[2]);

    let list_area = chunks[3];
    let inner_width = list_area.width.saturating_sub(2) as usize; // minus left/right borders

    let todos: Vec<ListItem> = filtered_todos(app)
        .iter()
        .map(|t| {
            let due_opt = t.due_date.as_ref();
            let is_doing = !t.done
                && app
                    .planka_lists_by_board
                    .get(&app.current_project)
                    .map(|lists| t.planka_list_id.as_deref() == Some(lists.doing_list_id.as_str()))
                    .unwrap_or(false);

            let status = if t.done {
                "[d]"
            } else if is_doing {
                "[w]"
            } else {
                "[ ]"
            };

            let mut desc_color = if t.done {
                Color::Green
            } else if is_doing {
                Color::Cyan
            } else if due_opt.map(|s| is_overdue(s)).unwrap_or(false) {
                Color::Red
            } else {
                Color::Yellow
            };
            // Build a single visible string, then soft-wrap to list width
            let mut text = format!("{} {}", status, t.description);
            if let Some(due) = due_opt {
                text.push_str(&format!(" (Due: {})", due));
            }
            text.push_str(&format!(" [Created: {}]", t.created_date));

            let wrapped = wrap(&text, inner_width);
            let lines: Vec<Line> = wrapped
                .iter()
                .map(|w| Line::from(Span::styled(
                    w.to_string(),
                    Style::default().fg(desc_color),
                )))
                .collect();
            ListItem::new(lines)
        })
        .collect();

    let mut list_state = ratatui::widgets::ListState::default();
    if !todos.is_empty() {
        list_state.select(Some(app.selected.min(todos.len() - 1)));
    }

    let todos_list = List::new(todos)
        .block(Block::default().borders(Borders::ALL).title("Todos"))
        .highlight_style(
            Style::default()
                .bg(Color::Blue)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");

    f.render_stateful_widget(todos_list, chunks[3], &mut list_state);

    // Optional single-line input at bottom (only when editing)
    if needs_input {
        let last = chunks.len() - 1;
        let caret = "|";
        if matches!(app.input_mode, InputMode::EditingPlanka) {
            let style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
            let text = if app.input_planka.is_empty() { caret.to_string() } else { format!("{}{}", app.input_planka, caret) };
            let title = match app.planka_setup {
                Some(crate::app::PlankaSetupStep::Url) => "Planka URL",
                Some(crate::app::PlankaSetupStep::Username) => "Planka Username or Email",
                Some(crate::app::PlankaSetupStep::Password) => "Planka Password",
                _ => "Planka Setup",
            };
            let widget = Paragraph::new(text)
                .block(Block::default().borders(Borders::ALL).title(title))
                .style(style)
                .wrap(Wrap { trim: true });
            f.render_widget(widget, chunks[last]);
        } else if matches!(app.input_mode, InputMode::EditingProject) {
            let style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
            let text = if app.input_project.is_empty() { caret.to_string() } else { format!("{}{}", app.input_project, caret) };
            let widget = Paragraph::new(text)
                .block(Block::default().borders(Borders::ALL).title("Project"))
                .style(style)
                .wrap(Wrap { trim: true });
            f.render_widget(widget, chunks[last]);
        } else if matches!(app.input_mode, InputMode::EditingDescription) {
            let style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
            let text = if app.input_description.is_empty() { caret.to_string() } else { format!("{}{}", app.input_description, caret) };
            let widget = Paragraph::new(text)
                .block(Block::default().borders(Borders::ALL).title("Description"))
                .style(style)
                .wrap(Wrap { trim: true });
            f.render_widget(widget, chunks[last]);
        } else if matches!(app.input_mode, InputMode::EditingDueDate) {
            let style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
            let text = if app.input_due_date.is_empty() { caret.to_string() } else { format!("{}{}", app.input_due_date, caret) };
            let widget = Paragraph::new(text)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Due") // shortened title
                )
                .style(style)
                .wrap(Wrap { trim: true });
            f.render_widget(widget, chunks[last]);
        } else if matches!(app.input_mode, InputMode::CreatingBoard) {
            let style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
            let text = if app.input_board.is_empty() { caret.to_string() } else { format!("{}{}", app.input_board, caret) };
            let proj_name = app
                .create_board_projects
                .get(app.create_board_project_index)
                .map(|(_, name)| name.as_str())
                .unwrap_or("Select project");
            let widget = Paragraph::new(text)
                .block(Block::default().borders(Borders::ALL).title(format!("New Board (Project: {})", proj_name)))
                .style(style)
                .wrap(Wrap { trim: true });
            f.render_widget(widget, chunks[last]);
        } else if matches!(app.input_mode, InputMode::CreatingProject) {
            let style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
            let text = if app.input_project.is_empty() { caret.to_string() } else { format!("{}{}", app.input_project, caret) };
            let widget = Paragraph::new(text)
                .block(Block::default().borders(Borders::ALL).title("New Project Name"))
                .style(style)
                .wrap(Wrap { trim: true });
            f.render_widget(widget, chunks[last]);
        } else if matches!(app.input_mode, InputMode::Searching) {
            let style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
            let text = if app.search_query.is_empty() { caret.to_string() } else { format!("{}{}", app.search_query, caret) };
            let widget = Paragraph::new(text)
                .block(Block::default().borders(Borders::ALL).title("Search"))
                .style(style)
                .wrap(Wrap { trim: true });
            f.render_widget(widget, chunks[last]);
        }
    }



    // Show error message if any
    if let Some(ref msg) = app.error_message {
        let error = Paragraph::new(msg.as_str())
            .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
            .alignment(Alignment::Center);
        let area = ratatui::layout::Rect {
            x: size.x,
            y: size.height.saturating_sub(2),
            width: size.width,
            height: 1,
        };
        f.render_widget(error, area);
    }
}

// Check if a due date string represents an overdue task
fn is_overdue(due_date_str: &str) -> bool {
    if due_date_str == "No due date" {
        return false;
    }

    let now = Local::now();

    // Try to parse as datetime first
    if let Ok(dt) = NaiveDateTime::parse_from_str(due_date_str, "%Y-%m-%d %H:%M") {
        if let Some(dt_local) = Local.from_local_datetime(&dt).single() {
            return dt_local < now;
        }
    }

    // Try to parse as date only
    if let Ok(date) = NaiveDate::parse_from_str(due_date_str, "%Y-%m-%d") {
        let today = now.date_naive();
        return date < today;
    }

    false
}

pub fn parse_due_date(input: &str) -> Result<String, String> {
    let input = input.trim().to_lowercase();
    let now = Local::now();
    let today = now.date_naive();

    // Handle empty input
    if input.is_empty() {
        return Err("Please enter a due date".to_string());
    }

    let words: Vec<&str> = input.split_whitespace().collect();

    let raw_date = match words.as_slice() {
        // Immediate times
        ["now"] => Ok(now.format("%Y-%m-%d %H:%M").to_string()),

        // Relative days
        ["today"] => Ok(today.format("%Y-%m-%d").to_string()),
        ["tomorrow"] | ["tmr"] => Ok((today + Dur::days(1)).format("%Y-%m-%d").to_string()),
        ["yesterday"] => Ok((today - Dur::days(1)).format("%Y-%m-%d").to_string()),

        // Day of week (this week or next week)
        [day] if is_weekday(day) => parse_weekday(day, today),

        // Next/this + day
        ["next", day] if is_weekday(day) => parse_next_weekday(day, today),
        ["this", day] if is_weekday(day) => parse_this_weekday(day, today),

        // Relative periods
        ["week"] | ["next", "week"] => Ok((today + Dur::days(7)).format("%Y-%m-%d").to_string()),
        ["month"] | ["next", "month"] => Ok((today + Dur::days(30)).format("%Y-%m-%d").to_string()),
        ["year"] | ["next", "year"] => Ok((today + Dur::days(365)).format("%Y-%m-%d").to_string()),

        // "in X unit" patterns
        ["in", num, unit] => parse_offset(num, unit, &now),

        // "in X unit Y unit" patterns (e.g., "in 1 day 3 hours")
        ["in", num1, unit1, num2, unit2] => parse_compound_offset(num1, unit1, num2, unit2, &now),

        // Date + time (YYYY-MM-DD HH:MM)
        [date_str, time_str] if looks_like_date(date_str) && looks_like_time(time_str) => {
            parse_date_time_combo(date_str, time_str)
        },

        // Weekday + time (e.g., "friday 15:30") — also before [num, unit]
        [day, time] if is_weekday(day) => parse_weekday_time(day, time, today),

        // "X unit" patterns (e.g., "3 days", "2 hours")
        [num, unit] => parse_offset(num, unit, &now),

        // "X unit Y unit" patterns (e.g., "1 day 3 hours")
        [num1, unit1, num2, unit2] => parse_compound_offset(num1, unit1, num2, unit2, &now),

        // Full date or time
        [date_or_time] => try_parse_date_or_time(date_or_time, today, now),


        // "next/this weekday time"
        ["next", day, time] if is_weekday(day) => parse_next_weekday_time(day, time, today),
        ["this", day, time] if is_weekday(day) => parse_this_weekday_time(day, time, today),

        _ => Err("Unrecognized due date format".to_string()),
    };

    raw_date.and_then(|date| validate_not_past(&date))
}

fn is_weekday(s: &str) -> bool {
    matches!(
        s,
        "monday"
            | "mon"
            | "tuesday"
            | "tue"
            | "wednesday"
            | "wed"
            | "thursday"
            | "thu"
            | "friday"
            | "fri"
            | "saturday"
            | "sat"
            | "sunday"
            | "sun"
    )
}

fn parse_weekday_name(s: &str) -> Option<Weekday> {
    match s {
        "monday" | "mon" => Some(Weekday::Mon),
        "tuesday" | "tue" => Some(Weekday::Tue),
        "wednesday" | "wed" => Some(Weekday::Wed),
        "thursday" | "thu" => Some(Weekday::Thu),
        "friday" | "fri" => Some(Weekday::Fri),
        "saturday" | "sat" => Some(Weekday::Sat),
        "sunday" | "sun" => Some(Weekday::Sun),
        _ => None,
    }
}

fn parse_weekday(day: &str, today: NaiveDate) -> Result<String, String> {
    let target_weekday = parse_weekday_name(day).ok_or("Invalid weekday")?;

    let days_until = days_until_weekday(today, target_weekday);
    let target_date = today + Dur::days(days_until);

    Ok(target_date.format("%Y-%m-%d").to_string())
}

fn parse_next_weekday(day: &str, today: NaiveDate) -> Result<String, String> {
    let target_weekday = parse_weekday_name(day).ok_or("Invalid weekday")?;

    let days_until = days_until_next_weekday(today, target_weekday);
    let target_date = today + Dur::days(days_until);

    Ok(target_date.format("%Y-%m-%d").to_string())
}

fn parse_this_weekday(day: &str, today: NaiveDate) -> Result<String, String> {
    let target_weekday = parse_weekday_name(day).ok_or("Invalid weekday")?;

    let days_until = days_until_this_week(today, target_weekday);
    let target_date = today + Dur::days(days_until);

    Ok(target_date.format("%Y-%m-%d").to_string())
}

fn parse_weekday_time(day: &str, time: &str, today: NaiveDate) -> Result<String, String> {
    let target_weekday = parse_weekday_name(day).ok_or("Invalid weekday")?;

    let target_time =
        NaiveTime::parse_from_str(time, "%H:%M").map_err(|_| "Invalid time format. Use HH:MM")?;

    let days_until = days_until_weekday(today, target_weekday);
    let target_date = today + Dur::days(days_until);
    let target_datetime = NaiveDateTime::new(target_date, target_time);

    Ok(target_datetime.format("%Y-%m-%d %H:%M").to_string())
}

fn parse_next_weekday_time(day: &str, time: &str, today: NaiveDate) -> Result<String, String> {
    let target_weekday = parse_weekday_name(day).ok_or("Invalid weekday")?;

    let target_time =
        NaiveTime::parse_from_str(time, "%H:%M").map_err(|_| "Invalid time format. Use HH:MM")?;

    let days_until = days_until_next_weekday(today, target_weekday);
    let target_date = today + Dur::days(days_until);
    let target_datetime = NaiveDateTime::new(target_date, target_time);

    Ok(target_datetime.format("%Y-%m-%d %H:%M").to_string())
}

fn parse_this_weekday_time(day: &str, time: &str, today: NaiveDate) -> Result<String, String> {
    let target_weekday = parse_weekday_name(day).ok_or("Invalid weekday")?;

    let target_time =
        NaiveTime::parse_from_str(time, "%H:%M").map_err(|_| "Invalid time format. Use HH:MM")?;

    let days_until = days_until_this_week(today, target_weekday);
    let target_date = today + Dur::days(days_until);
    let target_datetime = NaiveDateTime::new(target_date, target_time);

    Ok(target_datetime.format("%Y-%m-%d %H:%M").to_string())
}

fn days_until_weekday(from: NaiveDate, target: Weekday) -> i64 {
    let current_weekday = from.weekday();
    let days =
        (target.num_days_from_monday() as i64) - (current_weekday.num_days_from_monday() as i64);

    if days <= 0 {
        days + 7 // Next week
    } else {
        days // This week
    }
}

fn days_until_next_weekday(from: NaiveDate, target: Weekday) -> i64 {
    let current_weekday = from.weekday();
    let days =
        (target.num_days_from_monday() as i64) - (current_weekday.num_days_from_monday() as i64);

    if days <= 0 {
        days + 7 // Next week
    } else {
        days + 7 // Force next week
    }
}

fn days_until_this_week(from: NaiveDate, target: Weekday) -> i64 {
    let current_weekday = from.weekday();
    let days =
        (target.num_days_from_monday() as i64) - (current_weekday.num_days_from_monday() as i64);

    if days < 0 {
        0 // If the day has passed this week, return today
    } else {
        days
    }
}

fn parse_duration_component(num_str: &str, unit: &str) -> Result<chrono::TimeDelta, String> {
    let num: i64 = num_str.parse().map_err(|_| "Invalid number")?;

    if num < 0 {
        return Err("Duration cannot be negative".to_string());
    }

    match unit {
        "second" | "seconds" | "sec" | "s" => Ok(Dur::seconds(num)),
        "minute" | "minutes" | "min" | "m" => Ok(Dur::minutes(num)),
        "hour" | "hours" | "hr" | "h" => Ok(Dur::hours(num)),
        "day" | "days" | "d" => Ok(Dur::days(num)),
        "week" | "weeks" | "w" => Ok(Dur::days(num * 7)),
        "month" | "months" => Ok(Dur::days(num * 30)),
        "year" | "years" => Ok(Dur::days(num * 365)),
        _ => Err(format!("Unsupported time unit '{}'", unit)),
    }
}

fn parse_offset(num: &str, unit: &str, now: &chrono::DateTime<Local>) -> Result<String, String> {
    let duration = parse_duration_component(num, unit)?;
    Ok(((*now) + duration).format("%Y-%m-%d %H:%M").to_string())
}

fn parse_compound_offset(
    num1: &str,
    unit1: &str,
    num2: &str,
    unit2: &str,
    now: &chrono::DateTime<Local>,
) -> Result<String, String> {
    let delta1 = parse_duration_component(num1, unit1)?;
    let delta2 = parse_duration_component(num2, unit2)?;
    Ok(((*now) + delta1 + delta2)
        .format("%Y-%m-%d %H:%M")
        .to_string())
}

fn try_parse_date_or_time(
    input: &str,
    today: NaiveDate,
    _now: chrono::DateTime<Local>,
) -> Result<String, String> {
    // Try full date (YYYY-MM-DD)
    if let Ok(date) = NaiveDate::parse_from_str(input, "%Y-%m-%d") {
        return Ok(date.format("%Y-%m-%d").to_string());
    }

    // Try date without year (MM-DD)
    if let Ok(parsed) =
        NaiveDate::parse_from_str(&format!("{}-{}", today.year(), input), "%Y-%m-%d")
    {
        return Ok(parsed.format("%Y-%m-%d").to_string());
    }

    // Try time for today (HH:MM)
    if let Ok(time) = NaiveTime::parse_from_str(input, "%H:%M") {
        let dt = NaiveDateTime::new(today, time);
        return Ok(dt.format("%Y-%m-%d %H:%M").to_string());
    }

    // Try 12-hour format (HH:MM AM/PM)
    if input.ends_with("am") || input.ends_with("pm") {
        let is_pm = input.ends_with("pm");
        let time_part = input.trim_end_matches("am").trim_end_matches("pm").trim();

        if let Ok(mut time) = NaiveTime::parse_from_str(time_part, "%H:%M") {
            if is_pm && time.hour() < 12 {
                time = time + Dur::hours(12);
            } else if !is_pm && time.hour() == 12 {
                time = time - Dur::hours(12);
            }
            let dt = NaiveDateTime::new(today, time);
            return Ok(dt.format("%Y-%m-%d %H:%M").to_string());
        }
    }

    Err("Invalid date or time format".to_string())
}

fn looks_like_date(s: &str) -> bool {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}
fn looks_like_time(s: &str) -> bool {
    NaiveTime::parse_from_str(s, "%H:%M").is_ok()
}

fn parse_date_time_combo(date_str: &str, time_str: &str) -> Result<String, String> {
    let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .map_err(|_| "Invalid date format. Use YYYY-MM-DD")?;

    let time = NaiveTime::parse_from_str(time_str, "%H:%M")
        .map_err(|_| "Invalid time format. Use HH:MM")?;

    let datetime = NaiveDateTime::new(date, time);
    Ok(datetime.format("%Y-%m-%d %H:%M").to_string())
}

fn validate_not_past(s: &str) -> Result<String, String> {
    let now = Local::now();

    // Try to parse as date+time first
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M") {
        let dt_local = Local
            .from_local_datetime(&dt)
            .single()
            .ok_or("Failed to convert due date to local time")?;

        if dt_local < now {
            return Err("Due date cannot be in the past".to_string());
        }
        return Ok(s.to_string());
    }

    // Try to parse as date only
    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let today = now.date_naive();
        if date < today {
            return Err("Due date cannot be in the past".to_string());
        }
        return Ok(s.to_string());
    }

    Err("Failed to parse due date".to_string())
}

#[cfg(target_os = "macos")]
fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start pbcopy: {}", e))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("Failed to write to pbcopy: {}", e))?;
    }
    let status = child.wait().map_err(|e| format!("pbcopy wait failed: {}", e))?;
    if status.success() { Ok(()) } else { Err("pbcopy failed".into()) }
}

#[cfg(target_os = "linux")]
fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut child = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start wl-copy: {}", e))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("Failed to write to wl-copy: {}", e))?;
    }
    let status = child.wait().map_err(|e| format!("wl-copy wait failed: {}", e))?;
    if status.success() { Ok(()) } else { Err("wl-copy failed".into()) }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn copy_to_clipboard(_text: &str) -> Result<(), String> {
    Err("Clipboard copy not supported on this OS".into())
}

#[cfg(target_os = "macos")]
fn paste_from_clipboard() -> Result<String, String> {
    let out = Command::new("pbpaste")
        .output()
        .map_err(|e| format!("Failed to start pbpaste: {}", e))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        Err("pbpaste failed".into())
    }
}

#[cfg(target_os = "linux")]
fn paste_from_clipboard() -> Result<String, String> {
    let out = Command::new("wl-paste")
        .output()
        .map_err(|e| format!("Failed to start wl-paste: {}", e))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        Err("wl-paste failed".into())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn paste_from_clipboard() -> Result<String, String> {
    Err("Clipboard paste not supported on this OS".into())
}
