// app.rs
use crate::todo::Todo;
use crate::tui::parse_due_date;
use chrono::Local;
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;
use crate::planka::{self, PlankaBoard, PlankaClient, PlankaConfig, PlankaLists, PlankaCard, PlankaCardDetails, PlankaComment};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PlankaSetupStep {
    Url,
    Username,
    Password,
}
#[derive(Serialize, Deserialize, Clone)]
pub enum PendingOpKind { Create, Move, Update, Delete }

#[derive(Serialize, Deserialize, Clone)]
pub struct PendingOp {
    pub kind: PendingOpKind,
    pub project: String,
    pub card_id: Option<String>,
    pub list_id: Option<String>,
    pub name: Option<String>,
    pub due: Option<String>,
    pub ts: i64,
}

#[derive(Clone, Debug)]
pub enum Delta {
    Upsert { project: String, id: String, name: String, due: Option<String>, created: Option<String>, done: bool, list_id: String },
    // Delete could be added later when we compute removals in the poller
}
fn default_projects() -> Vec<String> { vec!["Inbox".to_string()] }
fn default_current_project() -> String { "Inbox".to_string() }

pub fn get_data_file_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|p| p.join(".config"))
                .unwrap_or_else(|| PathBuf::from("."))
        });
    let dir = base.join("Planky");
    std::fs::create_dir_all(&dir).ok();
    dir.join("todos.json")
}

#[derive(PartialEq, Deserialize, Serialize)]
pub enum InputMode {
    Normal,
    EditingDescription,
    EditingDueDate,
    Searching,      // search mode
    EditingProject, // project name editing
    EditingPlanka,  // Planka setup flow
    CreatingBoard,
    CreatingProject,
    ViewingCard,
    CreatingComment,
    ControlCenter,
}

#[derive(Serialize, Deserialize)]
pub struct App {
    pub todos: Vec<Todo>,

    #[serde(default = "default_projects")]
    pub projects: Vec<String>,
    #[serde(default = "default_current_project")]
    pub current_project: String,
    #[serde(skip)]
    pub input_project: String,
    #[serde(skip)]
    pub input_board: String,

    #[serde(skip)]
    pub create_board_projects: Vec<(String, String)>, // (project_id, project_name)
    #[serde(skip)]
    pub create_board_project_index: usize,

    #[serde(skip)]
    pub input_mode: InputMode,
    #[serde(skip)]
    pub input_description: String,
    #[serde(skip)]
    pub input_due_date: String,
    #[serde(skip)]
    pub selected: usize,
    #[serde(skip)]
    pub error_message: Option<String>,
    #[serde(skip)]
    pub search_query: String, // Added for search
    #[serde(skip)]
    pub planka_config: Option<PlankaConfig>,
    #[serde(skip)]
    pub planka_lists: Option<PlankaLists>,
    #[serde(skip)]
    pub planka_lists_by_board: HashMap<String, PlankaLists>,
    #[serde(skip)]
    pub planka_boards: Vec<PlankaBoard>,
    #[serde(skip)]
    pub input_planka: String,
    #[serde(skip)]
    pub planka_setup: Option<PlankaSetupStep>,
    #[serde(skip)]
    pub pending_ops: Vec<PendingOp>,
    #[serde(skip)]
    pub inbound_rx: Option<Receiver<Delta>>,
    #[serde(skip)]
    pub control_center_index: usize,
    #[serde(skip)]
    pub editing_index: Option<usize>,
    #[serde(skip)]
    pub view_card: Option<PlankaCardDetails>,
    #[serde(skip)]
    pub view_comments: Vec<PlankaComment>,
    #[serde(skip)]
    pub input_comment: String,
    #[serde(skip)]
    pub view_scroll: u16,
}

impl Default for InputMode {
    fn default() -> Self {
        InputMode::Normal
    }
}

impl Default for App {
    fn default() -> Self {
        App::new()
    }
}

fn format_planka_due(s: &str) -> Option<String> {
    // Try RFC3339 first, else return None to leave empty
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        Some(dt.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string())
    } else if s.len() >= 10 && s.chars().nth(4) == Some('-') {
        // Looks like a date string; keep date only
        Some(s[..10].to_string())
    } else {
        None
    }
}

fn format_planka_created(s: &str) -> String {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        dt.with_timezone(&Local).format("%Y-%m-%d").to_string()
    } else if s.len() >= 10 && s.chars().nth(4) == Some('-') {
        s[..10].to_string()
    } else {
        Local::now().format("%Y-%m-%d").to_string()
    }
}

impl App {
    fn selected_index_in_all(&self) -> Option<usize> {
        // Apply same project and search filtering as the TUI
        let q = if self.search_query.is_empty() {
            None
        } else {
            Some(self.search_query.to_lowercase())
        };

        // Detect Doing/Done list ids for current project
        let doing_id = self
            .planka_lists_by_board
            .get(&self.current_project)
            .map(|l| l.doing_list_id.as_str());
        let done_id = self
            .planka_lists_by_board
            .get(&self.current_project)
            .map(|l| l.done_list_id.as_str());

        // Build grouped indices: Doing first, then Todo, then Done
        let mut doing: Vec<usize> = Vec::new();
        let mut todo: Vec<usize> = Vec::new();
        let mut done: Vec<usize> = Vec::new();

        for (i, t) in self.todos.iter().enumerate() {
            if t.project != self.current_project {
                continue;
            }
            if let Some(ref ql) = q {
                let matches = t.description.to_lowercase().contains(ql)
                    || t
                        .due_date
                        .as_ref()
                        .map(|d| d.to_lowercase().contains(ql))
                        .unwrap_or(false);
                if !matches {
                    continue;
                }
            }
            let in_doing = doing_id
                .map(|id| t.planka_list_id.as_deref() == Some(id))
                .unwrap_or(false);
            let in_done = done_id
                .map(|id| t.planka_list_id.as_deref() == Some(id))
                .unwrap_or(false);

            if !t.done && in_doing {
                doing.push(i);
            } else if t.done || in_done {
                done.push(i);
            } else {
                todo.push(i);
            }
        }

        let mut ordered: Vec<usize> = Vec::with_capacity(doing.len() + todo.len() + done.len());
        ordered.extend(doing);
        ordered.extend(todo);
        ordered.extend(done);

        ordered.get(self.selected).copied()
    }
    pub fn new() -> Self {
        Self {
            todos: Vec::new(),
            input_mode: InputMode::Normal,
            input_description: String::new(),
            input_due_date: String::new(),
            selected: 0,
            error_message: None,
            search_query: String::new(), // Initialize search_query
            projects: default_projects(),
            current_project: default_current_project(),
            input_project: String::new(),
            input_board: String::new(),
            create_board_projects: Vec::new(),
            create_board_project_index: 0,
            planka_config: planka::load_config(),
            planka_lists: None,
            planka_lists_by_board: HashMap::new(),
            planka_boards: Vec::new(),
            input_planka: String::new(),
            planka_setup: None,
            pending_ops: Self::load_pending_ops(),
            inbound_rx: None,
            control_center_index: 0,
            editing_index: None,
            view_card: None,
            view_comments: Vec::new(),
            input_comment: String::new(),
            view_scroll: 0,
        }
    }

    fn pending_ops_path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|p| p.join(".config"))
                    .unwrap_or_else(|| PathBuf::from("."))
            });
        let dir = base.join("Planky");
        std::fs::create_dir_all(&dir).ok();
        dir.join("pending_ops.json")
    }

    fn save_pending_ops(&self) {
        let path = Self::pending_ops_path();
        if let Ok(file) = OpenOptions::new().create(true).write(true).truncate(true).open(path) {
            let _ = serde_json::to_writer(BufWriter::new(file), &self.pending_ops);
        }
    }

    fn load_pending_ops() -> Vec<PendingOp> {
        let path = Self::pending_ops_path();
        if let Ok(file) = File::open(path) {
            let reader = BufReader::new(file);
            serde_json::from_reader(reader).unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    fn enqueue_op(&mut self, op: PendingOp) {
        self.pending_ops.push(op);
        self.save_pending_ops();
    }

    pub fn pending_ops_len(&self) -> usize {
        self.pending_ops.len()
    }

    pub fn start_background_sync(&mut self) {
        let (tx, rx) = mpsc::channel::<Delta>();
        self.inbound_rx = Some(rx);
        thread::spawn(move || {
            loop {
                // Load cfg fresh each tick to allow login during runtime
                let cfg = planka::load_config();
                if let Some(cfg) = cfg {
                    if let Ok((client, _)) = PlankaClient::from_config(cfg) {
                        if let Ok(boards) = client.fetch_boards() {
                            for b in boards {
                                if let Ok(lists) = client.resolve_lists(&b.name) {
                                    // todo + doing as not-done
                                    if let Ok(cards) = client.fetch_cards(&lists.todo_list_id) {
                                        for c in cards {
                                            let _ = tx.send(Delta::Upsert {
                                                project: b.name.clone(),
                                                id: c.id.clone(),
                                                name: c.name.clone(),
                                                due: c.due.clone(),
                                                created: c.created.clone(),
                                                done: false,
                                                list_id: lists.todo_list_id.clone(),
                                            });
                                        }
                                    }
                                    if let Ok(cards) = client.fetch_cards(&lists.doing_list_id) {
                                        for c in cards {
                                            let _ = tx.send(Delta::Upsert {
                                                project: b.name.clone(),
                                                id: c.id.clone(),
                                                name: c.name.clone(),
                                                due: c.due.clone(),
                                                created: c.created.clone(),
                                                done: false,
                                                list_id: lists.doing_list_id.clone(),
                                            });
                                        }
                                    }
                                    if let Ok(cards) = client.fetch_cards(&lists.done_list_id) {
                                        for c in cards {
                                            let _ = tx.send(Delta::Upsert {
                                                project: b.name.clone(),
                                                id: c.id.clone(),
                                                name: c.name.clone(),
                                                due: c.due.clone(),
                                                created: c.created.clone(),
                                                done: true,
                                                list_id: lists.done_list_id.clone(),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                thread::sleep(Duration::from_secs(15));
            }
        });
    }

    pub fn apply_delta(&mut self, d: Delta) {
        match d {
            Delta::Upsert { project, id, name, due, created, done, list_id } => {
                // Skip overwriting local dirty items
                if let Some(t) = self.todos.iter_mut().find(|t| t.project == project && t.planka_card_id.as_deref() == Some(id.as_str())) {
                    if t.sync_dirty { return; }
                    t.description = name;
                    t.done = done;
                    t.due_date = due.as_deref().and_then(|s| format_planka_due(s));
                    if let Some(c) = created.as_deref() {
                        t.created_date = format_planka_created(c);
                    }
                    t.planka_list_id = Some(list_id.clone());
                } else {
                    self.todos.push(Todo {
                        description: name,
                        done,
                        due_date: due.as_deref().and_then(|s| format_planka_due(s)),
                        created_date: created
                            .as_deref()
                            .map(|s| format_planka_created(s))
                            .unwrap_or_else(|| Local::now().format("%Y-%m-%d").to_string()),
                        project,
                        planka_card_id: Some(id),
                        planka_list_id: Some(list_id),
                        planka_board_id: None,
                        sync_dirty: false,
                    });
                }
            }
        }
    }

    pub fn process_pending_ops_tick(&mut self) {
        if self.pending_ops.is_empty() {
            return;
        }
        let client = match self.ensure_planka_client() {
            Ok(c) => c,
            Err(_) => return, // keep ops queued
        };
        // Work on a copy to allow removal while iterating
        let ops = self.pending_ops.clone();
        let mut any_removed = false;
        for op in ops {
            match op.kind {
                PendingOpKind::Create => {
                    // Resolve lists for the project
                    if let Ok(lists) = client.resolve_lists(&op.project) {
                        if let Some(name) = op.name.clone() {
                            let due = op.due.as_deref();
                            if let Ok(cid) = client.create_card(&lists.todo_list_id, &name, due) {
                                // Update the first matching local todo without card id
                                if let Some(t) = self.todos.iter_mut().find(|t| t.project == op.project && t.planka_card_id.is_none() && t.description == name) {
                                    // Capture desired target before overwriting list_id
                                    let wants_doing = t.planka_list_id.as_deref() == Some(lists.doing_list_id.as_str());
                                    let wants_done = t.done;
                                    t.planka_card_id = Some(cid.clone());
                                    t.planka_list_id = Some(lists.todo_list_id.clone());
                                    t.planka_board_id = Some(lists.board_id.clone());
                                    t.sync_dirty = false;
                                    // If user had toggled Doing (or Done) before create succeeded, move now
                                    if wants_doing {
                                        let _ = client.move_card(&cid, &lists.doing_list_id);
                                        t.planka_list_id = Some(lists.doing_list_id.clone());
                                    } else if wants_done {
                                        let _ = client.move_card(&cid, &lists.done_list_id);
                                        t.planka_list_id = Some(lists.done_list_id.clone());
                                    }
                                }
                                // Remove op
                                if let Some(pos) = self.pending_ops.iter().position(|p| p.ts == op.ts) {
                                    self.pending_ops.remove(pos);
                                    any_removed = true;
                                }
                            }
                        }
                    }
                }
                PendingOpKind::Move => {
                    if let (Some(ref cid), Some(ref lid)) = (op.card_id.as_ref(), op.list_id.as_ref()) {
                        if client.move_card(cid, lid).is_ok() {
                            if let Some(t) = self.todos.iter_mut().find(|t| t.planka_card_id.as_deref() == Some(cid.as_str())) {
                                t.planka_list_id = Some(lid.to_string());
                                t.sync_dirty = false;
                            }
                            if let Some(pos) = self.pending_ops.iter().position(|p| p.ts == op.ts) {
                                self.pending_ops.remove(pos);
                                any_removed = true;
                            }
                        }
                    }
                }
                PendingOpKind::Delete => {
                    if let Some(ref cid) = op.card_id {
                        if client.delete_card(cid).is_ok() {
                            if let Some(pos) = self.pending_ops.iter().position(|p| p.ts == op.ts) {
                                self.pending_ops.remove(pos);
                                any_removed = true;
                            }
                        }
                    }
                }
                PendingOpKind::Update => {
                    if let Some(ref cid) = op.card_id {
                        let _ = client.update_card(cid, op.name.as_deref(), op.due.as_deref());
                        if let Some(pos) = self.pending_ops.iter().position(|p| p.ts == op.ts) {
                            self.pending_ops.remove(pos);
                            any_removed = true;
                        }
                    }
                }
            }
        }
        if any_removed { self.save_pending_ops(); }
    }

    pub fn drain_inbound(&mut self) {
        let Some(rx) = self.inbound_rx.take() else { return; };
        let rx = rx;
        while let Ok(d) = rx.try_recv() {
            self.apply_delta(d);
        }
        self.inbound_rx = Some(rx);
    }

    pub fn ensure_planka_client(&mut self) -> Result<PlankaClient, String> {
        let cfg = self
            .planka_config
            .clone()
            .ok_or_else(|| "Planka config not set. Press 'L' to login/setup.".to_string())?;
        let (client, new_cfg) = PlankaClient::from_config(cfg)?;
        self.planka_config = Some(new_cfg);
        Ok(client)
    }

    pub fn sync_current_project_from_planka(&mut self) {
        match self.ensure_planka_client() {
            Ok(client) => {
                // 1) Fetch boards and use them as “projects”
                if let Ok(boards) = client.fetch_boards() {
                    if !boards.is_empty() {
                        let names: Vec<String> = boards.iter().map(|b| b.name.clone()).collect();
                        self.planka_boards = boards;
                        self.projects = names;
                        if !self.projects.iter().any(|p| p == &self.current_project) {
                            if let Some(first) = self.projects.get(0) {
                                self.current_project = first.clone();
                                self.selected = 0;
                            }
                        }
                    }
                }
                // 2) Resolve lists for the current project (board name)
                match client.resolve_lists(&self.current_project) {
                    Ok(lists) => {
                        self.planka_lists = Some(lists.clone());
                        self.planka_lists_by_board
                            .insert(self.current_project.clone(), lists.clone());
                        // 3) Fetch cards from todo/doing/done lists
                        let mut all_cards: Vec<(PlankaCard, String)> = Vec::new(); // (card, list_id)
                        // todo list
                        if let Ok(cards) = client.fetch_cards(&lists.todo_list_id) {
                            all_cards.extend(
                                cards
                                    .into_iter()
                                    .map(|c| (c, lists.todo_list_id.clone())),
                            );
                        }
                        // doing list (treat as not done)
                        if let Ok(cards) = client.fetch_cards(&lists.doing_list_id) {
                            all_cards.extend(
                                cards
                                    .into_iter()
                                    .map(|c| (c, lists.doing_list_id.clone())),
                            );
                        }
                        // done list
                        if let Ok(cards) = client.fetch_cards(&lists.done_list_id) {
                            all_cards.extend(
                                cards
                                    .into_iter()
                                    .map(|c| (c, lists.done_list_id.clone())),
                            );
                        }

                        // 4) Merge into local todos for the current project (by planka_card_id)
                        let mut index_by_card: HashMap<String, usize> = HashMap::new();
                        for (i, t) in self.todos.iter().enumerate() {
                            if t.project == self.current_project {
                                if let Some(ref cid) = t.planka_card_id {
                                    index_by_card.insert(cid.clone(), i);
                                }
                            }
                        }

                        for (card, list_id) in all_cards {
                            if let Some(&idx) = index_by_card.get(&card.id) {
                                // Update existing
                                if let Some(t) = self.todos.get_mut(idx) {
                                    let is_done = list_id == lists.done_list_id;
                                    t.description = card.name.clone();
                                    t.done = is_done;
                                    t.due_date = card
                                        .due
                                        .as_deref()
                                        .and_then(|s| format_planka_due(s));
                                    if let Some(ref s) = card.created {
                                        t.created_date = format_planka_created(s);
                                    }
                                    t.planka_list_id = Some(list_id.clone());
                                    t.planka_board_id = Some(lists.board_id.clone());
                                }
                            } else {
                                // Insert new
                                self.todos.push(Todo {
                                    description: card.name.clone(),
                                    done: list_id == lists.done_list_id,
                                    due_date: card.due.as_deref().and_then(|s| format_planka_due(s)),
                                    created_date: card
                                        .created
                                        .as_deref()
                                        .map(|s| format_planka_created(s))
                                        .unwrap_or_else(|| Local::now().format("%Y-%m-%d").to_string()),
                                    project: self.current_project.clone(),
                                    planka_card_id: Some(card.id.clone()),
                                    planka_list_id: Some(list_id.clone()),
                                    planka_board_id: Some(lists.board_id.clone()),
                                    sync_dirty: false,
                                });
                            }
                        }
                        self.error_message = Some("Synced from Planka".to_string());
                    }
                    Err(e) => self.error_message = Some(e),
                }
            }
            Err(e) => self.error_message = Some(e),
        }
    }

    pub fn sync_all_projects_from_planka(&mut self) {
        let client = match self.ensure_planka_client() {
            Ok(c) => c,
            Err(e) => {
                self.error_message = Some(e);
                return;
            }
        };
        // 1) Fetch boards and reflect as projects
        if let Ok(boards) = client.fetch_boards() {
            if !boards.is_empty() {
                self.planka_boards = boards.clone();
                self.projects = boards.iter().map(|b| b.name.clone()).collect();
                if !self.projects.iter().any(|p| p == &self.current_project) {
                    if let Some(first) = self.projects.get(0) {
                        self.current_project = first.clone();
                        self.selected = 0;
                    }
                }
            }
        }
        // 2) For each board: resolve lists, pull remote, merge; then push local diffs
        for b in &self.planka_boards {
            let proj = b.name.clone();
            // Resolve and cache lists for this board
            let lists = if let Some(l) = self.planka_lists_by_board.get(&proj).cloned() {
                l
            } else {
                match client.resolve_lists(&proj) {
                    Ok(l) => {
                        self.planka_lists_by_board.insert(proj.clone(), l.clone());
                        if proj == self.current_project {
                            self.planka_lists = Some(l.clone());
                        }
                        l
                    }
                    Err(e) => {
                        self.error_message = Some(e);
                        continue;
                    }
                }
            };
            // Pull: fetch remote cards (todo/doing/done)
            let mut remote_by_id: HashMap<String, (PlankaCard, bool, String)> = HashMap::new();
            if let Ok(cards) = client.fetch_cards(&lists.todo_list_id) {
                for c in cards {
                    remote_by_id.insert(c.id.clone(), (c, false, lists.todo_list_id.clone()));
                }
            }
            if let Ok(cards) = client.fetch_cards(&lists.doing_list_id) {
                for c in cards {
                    remote_by_id.insert(c.id.clone(), (c, false, lists.doing_list_id.clone()));
                }
            }
            if let Ok(cards) = client.fetch_cards(&lists.done_list_id) {
                for c in cards {
                    remote_by_id.insert(c.id.clone(), (c, true, lists.done_list_id.clone()));
                }
            }
            // Local index by card id for this project
            let mut local_index: HashMap<String, usize> = HashMap::new();
            for (i, t) in self.todos.iter().enumerate() {
                if t.project == proj {
                    if let Some(ref cid) = t.planka_card_id {
                        local_index.insert(cid.clone(), i);
                    }
                }
            }
            // Merge remote -> local
            for (_cid, (rcard, rdone, rlist)) in &remote_by_id {
                if let Some(&idx) = local_index.get(&rcard.id) {
                    if let Some(t) = self.todos.get_mut(idx) {
                        t.description = rcard.name.clone();
                        t.done = *rdone;
                        t.due_date = rcard
                            .due
                            .as_deref()
                            .and_then(|s| format_planka_due(s));
                        if let Some(ref s) = rcard.created {
                            t.created_date = format_planka_created(s);
                        }
                        t.planka_list_id = Some(rlist.clone());
                        t.planka_board_id = Some(lists.board_id.clone());
                    }
                } else {
                    // Create local for remote-only card
                    self.todos.push(Todo {
                        description: rcard.name.clone(),
                        done: *rdone,
                        due_date: rcard.due.as_deref().and_then(|s| format_planka_due(s)),
                        created_date: rcard
                            .created
                            .as_deref()
                            .map(|s| format_planka_created(s))
                            .unwrap_or_else(|| Local::now().format("%Y-%m-%d").to_string()),
                        project: proj.clone(),
                        planka_card_id: Some(rcard.id.clone()),
                        planka_list_id: Some(rlist.clone()),
                        planka_board_id: Some(lists.board_id.clone()),
                        sync_dirty: false,
                    });
                }
            }
            // Push local -> remote
            for t in self.todos.iter_mut().filter(|t| t.project == proj) {
                // New local: create remote
                if t.planka_card_id.is_none() {
                    let initial_list = if t.done {
                        &lists.done_list_id
                    } else if t.planka_list_id.as_deref() == Some(lists.doing_list_id.as_str()) {
                        &lists.doing_list_id
                    } else {
                        &lists.todo_list_id
                    };
                    match client.create_card(initial_list, &t.description, t.due_date.as_deref()) {
                        Ok(cid) => {
                            t.planka_card_id = Some(cid);
                            t.planka_list_id = Some(initial_list.clone());
                            t.planka_board_id = Some(lists.board_id.clone());
                        }
                        Err(e) => {
                            self.error_message = Some(format!("Planka create card failed: {}", e));
                        }
                    }
                    continue;
                }
                // Existing linked: ensure list matches done-state and fields updated
                if let Some(ref cid) = t.planka_card_id.clone() {
                    let desired_list = if t.done {
                        &lists.done_list_id
                    } else if t.planka_list_id.as_deref() == Some(lists.doing_list_id.as_str()) {
                        &lists.doing_list_id
                    } else {
                        &lists.todo_list_id
                    };
                    let remote = remote_by_id.get(cid);
                    // Move if list differs
                    if remote.map(|(_, _, l)| l.as_str()) != Some(desired_list.as_str()) {
                        if let Err(e) = client.move_card(cid, desired_list) {
                            self.error_message = Some(format!("Planka move failed: {}", e));
                        } else {
                            t.planka_list_id = Some(desired_list.clone());
                        }
                    }
                    // Update name/due if differ
                    if let Some((rcard, _rdone, _)) = remote {
                        let due_disp = t.due_date.clone();
                        let due_raw = due_disp.as_deref();
                        let name_changed = rcard.name != t.description;
                        let due_changed = rcard.due.as_deref() != due_raw;
                        if name_changed || due_changed {
                            let _ = client.update_card(
                                cid,
                                if name_changed { Some(&t.description) } else { None },
                                if due_changed { due_raw } else { None },
                            );
                        }
                    }
                }
            }
        }
        self.error_message = Some("Synced all projects from Planka".to_string());
    }

    pub fn start_planka_setup(&mut self) {
        self.planka_setup = Some(PlankaSetupStep::Url);
        self.input_planka = self
            .planka_config
            .as_ref()
            .map(|c| c.server_url.clone())
            .unwrap_or_default();
        self.input_mode = InputMode::EditingPlanka;
        self.error_message = None;
    }

    pub fn submit_planka_setup(&mut self) {
        let step = match self.planka_setup {
            Some(s) => s,
            None => return,
        };
        let mut cfg = self.planka_config.clone().unwrap_or_default();
        match step {
            PlankaSetupStep::Url => {
                cfg.server_url = self.input_planka.trim().to_string();
                self.input_planka.clear();
                // persist partial config so next step sees server_url
                self.planka_config = Some(cfg.clone());
                self.planka_setup = Some(PlankaSetupStep::Username);
                // optional: prefill username if already present
                if let Some(existing) = self.planka_config.as_ref() {
                    if !existing.email_or_username.is_empty() {
                        self.input_planka = existing.email_or_username.clone();
                    }
                }
            }
            PlankaSetupStep::Username => {
                cfg.email_or_username = self.input_planka.trim().to_string();
                self.input_planka.clear();
                // persist partial config so next step sees server_url + username
                self.planka_config = Some(cfg.clone());
                self.planka_setup = Some(PlankaSetupStep::Password);
                // optional: prefill password if already present (rare)
                if let Some(existing) = self.planka_config.as_ref() {
                    if !existing.password.is_empty() {
                        self.input_planka = existing.password.clone();
                    }
                }
            }
            PlankaSetupStep::Password => {
                cfg.password = self.input_planka.clone();
                self.input_planka.clear();
                match PlankaClient::from_config(cfg.clone()) {
                    Ok((_client, saved)) => {
                        self.planka_config = Some(saved);
                        let _ = planka::save_config(self.planka_config.as_ref().unwrap());
                        self.error_message = Some("Planka login successful".to_string());
                        // Populate projects from Planka boards now.
                        self.sync_current_project_from_planka();
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Planka login failed: {}", e));
                    }
                }
                self.planka_setup = None;
                self.input_mode = InputMode::Normal;
            }
        }
    }

    pub fn begin_create_board(&mut self) {
        self.input_board.clear();
        self.error_message = None;

        // Ensure boards cache is present
        if self.planka_boards.is_empty() {
            if let Ok(client) = self.ensure_planka_client() {
                if let Ok(boards) = client.fetch_boards() {
                    self.planka_boards = boards;
                }
            }
        }

        // Build unique project list from boards
        let mut seen: HashSet<String> = HashSet::new();
        let mut projects: Vec<(String, String)> = Vec::new();
        for b in &self.planka_boards {
            if let Some(ref pid) = b.project_id {
                if seen.insert(pid.clone()) {
                    let pname = b.project_name.clone().unwrap_or_else(|| "Project".to_string());
                    projects.push((pid.clone(), pname));
                }
            }
        }
        // Sort by name for stable UX
        projects.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));

        // Default selection: the project of the current board (if found)
        let mut sel = 0usize;
        if let Some(cur_pid) = self
            .planka_boards
            .iter()
            .find(|b| b.name == self.current_project)
            .and_then(|b| b.project_id.clone())
        {
            if let Some(pos) = projects.iter().position(|(id, _)| *id == cur_pid) {
                sel = pos;
            }
        }

        self.create_board_projects = projects;
        self.create_board_project_index = sel;

        self.input_mode = InputMode::CreatingBoard;
    }

    pub fn submit_create_board(&mut self) -> Result<(), String> {
        let name = self.input_board.trim().to_string();
        if name.is_empty() {
            return Err("Board name cannot be empty.".to_string());
        }
        let client = self.ensure_planka_client()?;
        let proj_id = if let Some((id, _name)) = self
            .create_board_projects
            .get(self.create_board_project_index)
        {
            id.clone()
        } else {
            self.planka_boards
                .iter()
                .find(|b| b.name == self.current_project)
                .and_then(|b| b.project_id.clone())
                .ok_or_else(|| "No project selected and current board not found on Planka; sync first.".to_string())?
        };
        let _bid = client.create_board(&proj_id, &name)?;
        if let Ok(boards) = client.fetch_boards() {
            self.planka_boards = boards.clone();
            self.projects = boards.iter().map(|b| b.name.clone()).collect();
        }
        self.current_project = name;
        self.selected = 0;
        self.input_board.clear();
        self.input_mode = InputMode::Normal;
        self.error_message = Some("Board created".to_string());
        self.create_board_projects.clear();
        self.create_board_project_index = 0;
        Ok(())
    }

    pub fn begin_create_project(&mut self) {
        self.input_project.clear();
        self.input_mode = InputMode::CreatingProject;
        self.error_message = None;
    }

    pub fn submit_create_project(&mut self) -> Result<(), String> {
        let name = self.input_project.trim().to_string();
        if name.is_empty() {
            return Err("Project name cannot be empty.".to_string());
        }
        let client = self.ensure_planka_client()?;
        let pid = client.create_project(&name)?;
        // Create a first board in the new project
        let first_board = "Main".to_string();
        let _bid = client.create_board(&pid, &first_board)?;
        if let Ok(boards) = client.fetch_boards() {
            self.planka_boards = boards.clone();
            self.projects = boards.iter().map(|b| b.name.clone()).collect();
        }
        self.current_project = first_board;
        self.selected = 0;
        self.input_project.clear();
        self.input_mode = InputMode::Normal;
        self.error_message = Some("Project created".to_string());
        Ok(())
    }

    pub fn begin_edit_selected(&mut self) {
        if let Some(idx) = self.selected_index_in_all() {
            let t = &self.todos[idx];
            self.input_description = t.description.clone();
            self.input_due_date = t.due_date.clone().unwrap_or_default();
            self.editing_index = Some(idx);
            self.input_mode = InputMode::EditingDescription;
            self.error_message = None;
        }
    }

    pub fn save_edit(&mut self) -> Result<(), String> {
        let Some(idx) = self.editing_index else { return Ok(()); };
        if self.input_description.trim().is_empty() {
            return Err("Description cannot be empty.".to_string());
        }
        let due_date_str = if self.input_due_date.trim().is_empty() {
            None
        } else {
            Some(parse_due_date(&self.input_due_date)?)
        };
        {
            let t = &mut self.todos[idx];
            t.description = self.input_description.clone();
            t.due_date = due_date_str.clone();
        }
        let card_id = self.todos[idx].planka_card_id.clone();
        if let Some(cid) = card_id {
            if let Ok(client) = self.ensure_planka_client() {
                if let Err(e) = client.update_card(&cid, Some(&self.input_description), due_date_str.as_deref()) {
                    self.error_message = Some(format!("Planka update failed: {}", e));
                    if let Some(t) = self.todos.get_mut(idx) {
                        t.sync_dirty = true;
                    }
                    self.enqueue_op(PendingOp {
                        kind: PendingOpKind::Update,
                        project: self.current_project.clone(),
                        card_id: Some(cid),
                        list_id: None,
                        name: Some(self.input_description.clone()),
                        due: due_date_str.clone(),
                        ts: Local::now().timestamp(),
                    });
                }
            } else {
                if let Some(t) = self.todos.get_mut(idx) {
                    t.sync_dirty = true;
                }
                self.enqueue_op(PendingOp {
                    kind: PendingOpKind::Update,
                    project: self.current_project.clone(),
                    card_id: Some(cid),
                    list_id: None,
                    name: Some(self.input_description.clone()),
                    due: due_date_str.clone(),
                    ts: Local::now().timestamp(),
                });
            }
        }
        // clear inputs and editing state
        self.input_description.clear();
        self.input_due_date.clear();
        self.editing_index = None;
        Ok(())
    }

    pub fn add_todo(&mut self) -> Result<(), String> {
        if self.input_description.trim().is_empty() {
            return Err("Description cannot be empty.".to_string());
        }

        let due_date_str = if self.input_due_date.trim().is_empty() {
            None
        } else {
            Some(parse_due_date(&self.input_due_date)?)
        };

        let mut todo = Todo {
            description: self.input_description.clone(),
            done: false,
            due_date: due_date_str.clone(),
            created_date: Local::now().format("%Y-%m-%d").to_string(),
            project: self.current_project.clone(),
            planka_card_id: None,
            planka_list_id: None,
            planka_board_id: None,
            sync_dirty: false,
        };
        if let Ok(client) = self.ensure_planka_client() {
            let lists_opt = if let Some(l) = self.planka_lists_by_board.get(&self.current_project).cloned() {
                Some(l)
            } else {
                match client.resolve_lists(&self.current_project) {
                    Ok(l) => {
                        self.planka_lists_by_board.insert(self.current_project.clone(), l.clone());
                        self.planka_lists = Some(l.clone());
                        Some(l)
                    }
                    Err(e) => {
                        self.error_message = Some(e);
                        None
                    }
                }
            };
            if let Some(lists) = lists_opt {
                match client.create_card(&lists.todo_list_id, &todo.description, due_date_str.as_deref()) {
                    Ok(card_id) => {
                        todo.planka_card_id = Some(card_id);
                        todo.planka_list_id = Some(lists.todo_list_id.clone());
                        todo.planka_board_id = Some(lists.board_id.clone());
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Planka create card failed: {}", e));
                        // queue create
                        todo.sync_dirty = true;
                        self.enqueue_op(PendingOp {
                            kind: PendingOpKind::Create,
                            project: self.current_project.clone(),
                            card_id: None,
                            list_id: None,
                            name: Some(self.input_description.clone()),
                            due: due_date_str.clone(),
                            ts: Local::now().timestamp(),
                        });
                    }
                }
            } else {
                // no lists: queue create for later
                todo.sync_dirty = true;
                self.enqueue_op(PendingOp {
                    kind: PendingOpKind::Create,
                    project: self.current_project.clone(),
                    card_id: None,
                    list_id: None,
                    name: Some(self.input_description.clone()),
                    due: due_date_str.clone(),
                    ts: Local::now().timestamp(),
                });
            }
        } else {
            // no client: queue create
            todo.sync_dirty = true;
            self.enqueue_op(PendingOp {
                kind: PendingOpKind::Create,
                project: self.current_project.clone(),
                card_id: None,
                list_id: None,
                name: Some(self.input_description.clone()),
                due: due_date_str.clone(),
                ts: Local::now().timestamp(),
            });
        }
        self.todos.push(todo);

        if !self.projects.iter().any(|p| p == &self.current_project) {
            self.projects.push(self.current_project.clone());
        }

        // clear inputs after adding
        self.input_description.clear();
        self.input_due_date.clear();
        self.error_message = None;

        Ok(())
    }

    pub fn delete_todo(&mut self) {
        let Some(idx) = self.selected_index_in_all() else { return; };
        let card_id = self.todos[idx].planka_card_id.clone();
        if let Some(ref cid) = card_id {
            if let Ok(client) = self.ensure_planka_client() {
                if let Err(e) = client.delete_card(cid) {
                    self.error_message = Some(format!("Planka delete failed: {}", e));
                    self.enqueue_op(PendingOp {
                        kind: PendingOpKind::Delete,
                        project: self.current_project.clone(),
                        card_id: Some(cid.clone()),
                        list_id: None,
                        name: None,
                        due: None,
                        ts: Local::now().timestamp(),
                    });
                }
            } else if self.error_message.is_none() {
                // ensure_planka_client sets error_message on failure
            }
        }
        self.todos.remove(idx);
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn mark_done(&mut self) {
        let Some(idx) = self.selected_index_in_all() else { return; };
        // Read needed values without holding a mutable borrow of self
        let (was_done, card_id_opt) = {
            let todo = &self.todos[idx];
            (todo.done, todo.planka_card_id.clone())
        };
        let new_done = !was_done;

        // If changing done-state, attempt to move the card on Planka
        if new_done || (!new_done && was_done) {
            if let Ok(client) = self.ensure_planka_client() {
                let lists = if let Some(l) = self.planka_lists_by_board.get(&self.current_project).cloned() {
                    l
                } else {
                    match client.resolve_lists(&self.current_project) {
                        Ok(l) => {
                            self.planka_lists_by_board.insert(self.current_project.clone(), l.clone());
                            self.planka_lists = Some(l.clone());
                            l
                        }
                        Err(e) => {
                            self.error_message = Some(e);
                            return;
                        }
                    }
                };
                if let Some(ref card_id) = card_id_opt {
                    let target = if new_done { &lists.done_list_id } else { &lists.todo_list_id };
                    if let Err(e) = client.move_card(card_id, target) {
                        self.error_message = Some(format!("Planka move to Done failed: {}", e));
                        if let Some(ref card_id) = card_id_opt {
                            self.enqueue_op(PendingOp {
                                kind: PendingOpKind::Move,
                                project: self.current_project.clone(),
                                card_id: Some(card_id.clone()),
                                list_id: Some(target.clone()),
                                name: None,
                                due: None,
                                ts: Local::now().timestamp(),
                            });
                        }
                        if let Some(todo) = self.todos.get_mut(idx) {
                            todo.sync_dirty = true;
                        }
                    } else {
                        if let Some(todo) = self.todos.get_mut(idx) {
                            todo.planka_list_id = Some(target.clone());
                        }
                    }
                }
            }
        }

        if let Some(todo) = self.todos.get_mut(idx) {
            todo.done = new_done;
        }
    }

    pub fn mark_doing(&mut self) {
        let Some(idx) = self.selected_index_in_all() else { return; };
        // Read needed values without holding a mutable borrow of self
        let (card_id_opt, _was_done, current_list_id) = {
            let t = &self.todos[idx];
            (t.planka_card_id.clone(), t.done, t.planka_list_id.clone())
        };
        // Doing is only for not-done items
        let target_is_doing: bool;
        let lists = match self.ensure_planka_client() {
            Ok(client) => {
                if let Some(l) = self.planka_lists_by_board.get(&self.current_project).cloned() {
                    l
                } else {
                    match client.resolve_lists(&self.current_project) {
                        Ok(l) => {
                            self.planka_lists_by_board.insert(self.current_project.clone(), l.clone());
                            self.planka_lists = Some(l.clone());
                            l
                        }
                        Err(e) => {
                            self.error_message = Some(e);
                            return;
                        }
                    }
                }
            }
            Err(e) => {
                self.error_message = Some(e);
                return;
            }
        };
        // Decide toggle target based on current list
        target_is_doing = current_list_id.as_deref() != Some(lists.doing_list_id.as_str());
        let target_list = if target_is_doing { &lists.doing_list_id } else { &lists.todo_list_id };
        // Move remote if we have a card id
        if let Some(ref cid) = card_id_opt {
            if let Ok(client) = self.ensure_planka_client() {
                if let Err(e) = client.move_card(cid, target_list) {
                    self.error_message = Some(format!("Planka move to Doing failed: {}", e));
                    // Queue a move only if we have a card id
                    self.enqueue_op(PendingOp {
                        kind: PendingOpKind::Move,
                        project: self.current_project.clone(),
                        card_id: Some(cid.clone()),
                        list_id: Some(target_list.clone()),
                        name: None,
                        due: None,
                        ts: Local::now().timestamp(),
                    });
                    if let Some(t) = self.todos.get_mut(idx) {
                        t.sync_dirty = true;
                    }
                } else {
                    if let Some(t) = self.todos.get_mut(idx) {
                        t.planka_list_id = Some(target_list.clone());
                    }
                }
            }
        } else {
            // No remote id: reflect locally; background queue cannot move without id
            if let Some(t) = self.todos.get_mut(idx) {
                t.planka_list_id = Some(target_list.clone());
                t.sync_dirty = true;
            }
        }
        // Ensure not done when marking Doing; clearing done state locally if set
        if let Some(t) = self.todos.get_mut(idx) {
            t.done = false;
        }
    }

    pub fn next_project(&mut self) {
        if self.projects.is_empty() {
            self.projects = default_projects();
        }
        if let Some(pos) = self.projects.iter().position(|p| p == &self.current_project) {
            let next = (pos + 1) % self.projects.len();
            self.current_project = self.projects[next].clone();
        } else {
            self.current_project = self.projects[0].clone();
        }
        if self.planka_lists_by_board.get(&self.current_project).is_none() {
            if let Ok(client) = self.ensure_planka_client() {
                if let Ok(lists) = client.resolve_lists(&self.current_project) {
                    self.planka_lists_by_board
                        .insert(self.current_project.clone(), lists.clone());
                    self.planka_lists = Some(lists);
                }
            }
        }
    }
    pub fn prev_project(&mut self) {
        if self.projects.is_empty() {
            self.projects = default_projects();
        }
        if let Some(pos) = self.projects.iter().position(|p| p == &self.current_project) {
            let prev = (pos + self.projects.len() - 1) % self.projects.len();
            self.current_project = self.projects[prev].clone();
        } else {
            self.current_project = self.projects[0].clone();
        }
        if self.planka_lists_by_board.get(&self.current_project).is_none() {
            if let Ok(client) = self.ensure_planka_client() {
                if let Ok(lists) = client.resolve_lists(&self.current_project) {
                    self.planka_lists_by_board
                        .insert(self.current_project.clone(), lists.clone());
                    self.planka_lists = Some(lists);
                }
            }
        }
    }
    pub fn set_current_project<S: Into<String>>(&mut self, name: S) {
        let name = name.into().trim().to_string();
        if name.is_empty() {
            return;
        }
        if !self.projects.iter().any(|p| p == &name) {
            self.projects.push(name.clone());
        }
        self.current_project = name;
        if self.planka_lists_by_board.get(&self.current_project).is_none() {
            if let Ok(client) = self.ensure_planka_client() {
                if let Ok(lists) = client.resolve_lists(&self.current_project) {
                    self.planka_lists_by_board
                        .insert(self.current_project.clone(), lists.clone());
                    self.planka_lists = Some(lists);
                }
            }
        }
    }
    pub fn refresh_projects_from_todos(&mut self) {
        let mut uniq: Vec<String> = self
            .todos
            .iter()
            .map(|t| t.project.clone())
            .filter(|p| !p.is_empty())
            .collect();
        uniq.sort();
        uniq.dedup();
        if uniq.is_empty() {
            uniq = default_projects();
        }
        self.projects = uniq;
        if !self.projects.iter().any(|p| p == &self.current_project) {
            self.current_project = self.projects[0].clone();
        }
    }

    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), String> {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .map_err(|e| format!("Failed to open file: {}", e))?;

        let writer = BufWriter::new(file);

        serde_json::to_writer_pretty(writer, self)
            .map_err(|e| format!("Failed to write JSON!: {}", e))
    }

    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Self {
        let file = File::open(&path);
        if let Ok(file) = file {
            let reader = BufReader::new(file);
            {
                let mut app: App =
                    serde_json::from_reader(reader).unwrap_or_else(|_| App::new());
                app.refresh_projects_from_todos();
                // Re-load Planka config each run (it’s not persisted in todos.json)
                app.planka_config = planka::load_config();
                app
            }
        } else {
            App::new()
        }
    }

    pub fn open_selected_card(&mut self) {
        let Some(idx) = self.selected_index_in_all() else { return; };
        let cid = match self.todos[idx].planka_card_id.as_deref() {
            Some(c) => c.to_string(),
            None => {
                self.error_message = Some("This task is not linked to a Planka card yet".to_string());
                return;
            }
        };
        match self.ensure_planka_client() {
            Ok(client) => match client.fetch_card_details(&cid) {
                Ok(details) => {
                    self.view_card = Some(details);
                    match client.fetch_comments(&cid) {
                        Ok(comments) => self.view_comments = comments,
                        Err(e) => { self.view_comments = Vec::new(); self.error_message = Some(e); }
                    }
                    self.view_scroll = 0;
                    self.input_mode = InputMode::ViewingCard;
                    self.error_message = None;
                }
                Err(e) => self.error_message = Some(e),
            },
            Err(e) => self.error_message = Some(e),
        }
    }

    pub fn close_view(&mut self) {
        self.view_card = None;
        self.view_scroll = 0;
        self.input_mode = InputMode::Normal;
    }

    pub fn begin_new_comment(&mut self) {
        self.input_comment.clear();
        self.input_mode = InputMode::CreatingComment;
        self.error_message = None;
    }

    pub fn begin_reply_to_last_comment(&mut self) {
        self.input_comment.clear();
        if let Some(last) = self.view_comments.last() {
            if let Some(name) = &last.user_name {
                self.input_comment = format!("@{} ", name);
            } else {
                let preview = last.text.lines().take(1).next().unwrap_or("");
                if !preview.is_empty() {
                    self.input_comment = format!("> {}\n", preview);
                }
            }
        }
        self.input_mode = InputMode::CreatingComment;
        self.error_message = None;
    }

    pub fn submit_comment(&mut self) -> Result<(), String> {
        let text = self.input_comment.trim().to_string();
        if text.is_empty() {
            return Err("Comment cannot be empty.".to_string());
        }
        let card_id = match self.view_card.as_ref() {
            Some(c) => c.id.clone(),
            None => return Err("No card open".to_string()),
        };
        let client = self.ensure_planka_client()?;
        let _cid = client.create_comment(&card_id, &text)?;
        match client.fetch_comments(&card_id) {
            Ok(comments) => self.view_comments = comments,
            Err(e) => self.error_message = Some(e),
        }
        self.input_comment.clear();
        self.input_mode = InputMode::ViewingCard;
        self.error_message = Some("Comment added".to_string());
        Ok(())
    }
}
