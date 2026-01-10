// app.rs
use crate::todo::Todo;
use crate::tui::parse_due_date;
use chrono::Local;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
fn default_projects() -> Vec<String> { vec!["Inbox".to_string()] }
fn default_current_project() -> String { "Inbox".to_string() }

pub fn get_data_file_path() -> PathBuf {
    let proj_dirs = ProjectDirs::from("com", "KushalMeghani", "RustyTodos")
        .expect("Failed to get project directories");
    let dir = proj_dirs.config_dir();
    std::fs::create_dir_all(dir).unwrap();
    dir.join("todos.json")
}

#[derive(PartialEq, Deserialize, Serialize)]
pub enum InputMode {
    Normal,
    EditingDescription,
    EditingDueDate,
    Searching,      // search mode
    EditingProject, // project name editing
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

        self.todos.push(Todo {
            description: self.input_description.clone(),
            done: false,
            due_date: due_date_str,
            created_date: Local::now().format("%Y-%m-%d").to_string(),
            project: self.current_project.clone(),
        });

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
        if let Some(todo) = self.todos.get_mut(self.selected) {
            todo.done = !todo.done;
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
                app
            }
        } else {
            App::new()
        }
    }
}
