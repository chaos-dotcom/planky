// app.rs
use crate::todo::Todo;
use crate::tui::parse_due_date;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use crate::planka::{self, PlankaBoard, PlankaClient, PlankaConfig, PlankaLists};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PlankaSetupStep {
    Url,
    Username,
    Password,
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
    let dir = base.join("RustyTodos");
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
    pub planka_boards: Vec<PlankaBoard>,
    #[serde(skip)]
    pub input_planka: String,
    #[serde(skip)]
    pub planka_setup: Option<PlankaSetupStep>,
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

impl App {
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
            planka_config: planka::load_config(),
            planka_lists: None,
            planka_boards: Vec::new(),
            input_planka: String::new(),
            planka_setup: None,
        }
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
                        self.error_message = Some("Synced from Planka".to_string());
                    }
                    Err(e) => self.error_message = Some(e),
                }
            }
            Err(e) => self.error_message = Some(e),
        }
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
        };
        if let Ok(client) = self.ensure_planka_client() {
            if self.planka_lists.is_none() {
                if let Ok(lists) = client.resolve_lists(&self.current_project) {
                    self.planka_lists = Some(lists);
                }
            }
            if let Some(ref lists) = self.planka_lists {
                match client.create_card(&lists.todo_list_id, &todo.description, due_date_str.as_deref()) {
                    Ok(card_id) => {
                        todo.planka_card_id = Some(card_id);
                        todo.planka_list_id = Some(lists.todo_list_id.clone());
                        todo.planka_board_id = Some(lists.board_id.clone());
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Planka create card failed: {}", e));
                    }
                }
            }
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
        if !self.todos.is_empty() {
            self.todos.remove(self.selected);
            if self.selected > 0 {
                self.selected -= 1;
            }
        }
    }

    pub fn mark_done(&mut self) {
        if self.selected >= self.todos.len() {
            return;
        }
        // Read needed values without holding a mutable borrow of self
        let (was_done, card_id_opt) = {
            let todo = &self.todos[self.selected];
            (todo.done, todo.planka_card_id.clone())
        };
        let new_done = !was_done;

        // If marking done, attempt to move the card on Planka first
        if new_done {
            if let Ok(client) = self.ensure_planka_client() {
                if self.planka_lists.is_none() {
                    if let Ok(lists) = client.resolve_lists(&self.current_project) {
                        self.planka_lists = Some(lists);
                    }
                }
                if let (Some(ref lists), Some(ref card_id)) =
                    (self.planka_lists.as_ref(), card_id_opt.as_ref())
                {
                    if let Err(e) = client.move_card(card_id, &lists.done_list_id) {
                        self.error_message = Some(format!("Planka move to Done failed: {}", e));
                    } else {
                        if let Some(todo) = self.todos.get_mut(self.selected) {
                            todo.planka_list_id = Some(lists.done_list_id.clone());
                        }
                    }
                }
            }
        }

        if let Some(todo) = self.todos.get_mut(self.selected) {
            todo.done = new_done;
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
}
