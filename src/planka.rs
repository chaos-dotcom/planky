use reqwest::blocking::Client;
use reqwest::blocking::multipart::Form;
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value, Map};
use std::fs::{create_dir_all, File};
#[cfg(debug_assertions)]
use std::fs::OpenOptions;
use std::io::{BufReader, BufWriter};
#[cfg(debug_assertions)]
use std::io::Write as IoWrite;
use std::path::PathBuf;
use std::collections::HashMap;

#[cfg(debug_assertions)]
fn log_http_request(method: &str, url: &str, headers: &[(&str, &str)], body: Option<&str>) {
    let head = format!("[HTTP OUT] {} {}", method, url);
    eprintln!("{}", head);
    log_to_file_line(&head);
    for (k, v) in headers {
        let shown = if k.eq_ignore_ascii_case("authorization") {
            mask_bearer(v)
        } else {
            (*v).to_string()
        };
        let hline = format!("  {}: {}", k, shown);
        eprintln!("{}", hline);
        log_to_file_line(&hline);
    }
    if let Some(b) = body {
        let bline = format!("  Body: {}", truncate(b, 4000));
        eprintln!("{}", bline);
        log_to_file_line(&bline);
    }
}

#[cfg(debug_assertions)]
fn log_http_response(status: u16, body: &str) {
    let head = format!("[HTTP IN] Status: {}", status);
    eprintln!("{}", head);
    log_to_file_line(&head);
    let bline = format!("  Body: {}", truncate(body, 4000));
    eprintln!("{}", bline);
    log_to_file_line(&bline);
}

#[cfg(debug_assertions)]
fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } else { format!("{}…(+{} bytes)", &s[..max], s.len() - max) }
}

#[cfg(debug_assertions)]
fn mask_bearer(v: &str) -> String {
    if let Some(token) = v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")) {
        let head = &token[..token.len().min(6)];
        let tail = &token[token.len().saturating_sub(4)..];
        format!("Bearer {}…{}", head, tail)
    } else {
        "*****".to_string()
    }
}

#[cfg(not(debug_assertions))]
fn log_http_request(_method: &str, _url: &str, _headers: &[(&str, &str)], _body: Option<&str>) {}
#[cfg(not(debug_assertions))]
fn log_http_response(_status: u16, _body: &str) {}

#[cfg(debug_assertions)]
static INIT_LOG_ONCE: std::sync::Once = std::sync::Once::new();

#[cfg(debug_assertions)]
fn log_file_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|p| p.join(".config"))
                .unwrap_or_else(|| PathBuf::from("."))
        });
    let dir = base.join("Planky");
    create_dir_all(&dir).ok();
    dir.join("planka_debug.log")
}

#[cfg(debug_assertions)]
fn log_to_file_line(s: &str) {
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(log_file_path()) {
        let _ = writeln!(f, "{}", s);
    }
}

#[cfg(debug_assertions)]
fn log_debug(msg: &str) {
    let line = format!("[DEBUG] {}", msg);
    eprintln!("{}", line);
    log_to_file_line(&line);
}

#[cfg(debug_assertions)]
fn init_log_notice() {
    INIT_LOG_ONCE.call_once(|| {
        let path = log_file_path();
        let note = format!("Planka debug logs -> {}", path.display());
        eprintln!("{}", note);
        log_to_file_line(&note);
    });
}

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct PlankaConfig {
    pub server_url: String,
    pub email_or_username: String,
    pub password: String,
    pub token: Option<String>,
}

pub fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|p| p.join(".config"))
                .unwrap_or_else(|| PathBuf::from("."))
        });
    let dir = base.join("Planky");
    create_dir_all(&dir).ok();
    dir.join("planka.json")
}

pub fn load_config() -> Option<PlankaConfig> {
    let path = config_path();
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    serde_json::from_reader(reader).ok()
}

pub fn save_config(cfg: &PlankaConfig) -> Result<(), String> {
    let path = config_path();
    let file = File::create(path).map_err(|e| format!("Open planka config failed: {}", e))?;
    let writer = BufWriter::new(file);
    serde_json::to_writer_pretty(writer, cfg)
        .map_err(|e| format!("Write planka config failed: {}", e))
}

#[derive(serde::Deserialize)]
struct CardDetailsRes {
    item: CardDetailsItem,
}

#[derive(serde::Deserialize)]
struct CardDetailsItem {
    #[serde(rename = "id")]
    id: String,
    #[serde(rename = "createdAt")]
    created_at: Option<String>,
    #[serde(rename = "dueDate")]
    due_date: Option<String>,
}

pub struct PlankaClient {
    pub base_url: String,
    pub client: Client,
    pub token: String,
}

impl PlankaClient {
    pub fn from_config(mut cfg: PlankaConfig) -> Result<(Self, PlankaConfig), String> {
        #[cfg(debug_assertions)]
        init_log_notice();
        #[cfg(debug_assertions)]
        log_debug("PlankaClient::from_config called");
        if cfg.server_url.trim().is_empty() {
            return Err("Planka server URL is empty".into());
        }
        if cfg.token.is_none() {
            #[cfg(debug_assertions)]
            log_debug("No existing token in config; attempting login");
            let token = login(&cfg.server_url, &cfg.email_or_username, &cfg.password)?;
            cfg.token = Some(token);
            #[cfg(debug_assertions)]
            log_debug("Login successful; token stored in config");
            let _ = save_config(&cfg);
        }
        #[cfg(debug_assertions)]
        if cfg.token.is_some() {
            log_debug("Using existing token from config");
        }
        let token = cfg.token.clone().unwrap();
        let client = Client::builder()
            .cookie_store(true)
            .build()
            .map_err(|e| format!("HTTP client build failed: {}", e))?;
        Ok((
            Self {
                base_url: cfg.server_url.clone(),
                client,
                token,
            },
            cfg,
        ))
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.token)
    }

    pub fn fetch_boards(&self) -> Result<Vec<PlankaBoard>, String> {
        let base = self.base_url.trim_end_matches('/');
        let auth = self.auth_header();
        // 1) Get all projects
        let projects_url = format!("{}/api/projects?include=boards", base);
        #[cfg(debug_assertions)]
        log_http_request(
            "GET",
            &projects_url,
            &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("X-Requested-With", "XMLHttpRequest")],
            None,
        );
        let resp = self
            .client
            .get(&projects_url)
            .header("Authorization", auth.clone())
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .send()
            .map_err(|e| format!("GET {} failed: {}", projects_url, e))?;
        let status = resp.status();
        let text = resp
            .text()
            .map_err(|e| format!("read {} failed: {}", projects_url, e))?;
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() {
            return Err(format!("List projects failed: HTTP {} - {}", status, text));
        }
        let v: Value =
            serde_json::from_str(&text).map_err(|e| format!("parse projects failed: {}", e))?;
        let mut proj_names: HashMap<String, String> = HashMap::new();
        let mut project_ids: Vec<String> = Vec::new();
        if let Some(arr) = v.as_array() {
            for p in arr {
                if let Some(id) = p.get("id").and_then(|x| x.as_str()) {
                    project_ids.push(id.to_string());
                    if let Some(pname) = p.get("name").and_then(|x| x.as_str())
                        .or_else(|| p.get("title").and_then(|x| x.as_str()))
                    {
                        proj_names.insert(id.to_string(), pname.to_string());
                    }
                }
            }
        } else if let Some(items) = v.get("items").and_then(|x| x.as_array()) {
            for p in items {
                if let Some(id) = p.get("id").and_then(|x| x.as_str()) {
                    project_ids.push(id.to_string());
                    if let Some(pname) = p.get("name").and_then(|x| x.as_str())
                        .or_else(|| p.get("title").and_then(|x| x.as_str()))
                    {
                        proj_names.insert(id.to_string(), pname.to_string());
                    }
                }
            }
        } else if let Some(projects) = v.get("projects").and_then(|x| x.as_array()) {
            for p in projects {
                if let Some(id) = p.get("id").and_then(|x| x.as_str()) {
                    project_ids.push(id.to_string());
                    if let Some(pname) = p.get("name").and_then(|x| x.as_str())
                        .or_else(|| p.get("title").and_then(|x| x.as_str()))
                    {
                        proj_names.insert(id.to_string(), pname.to_string());
                    }
                }
            }
        }
        // 2) Prefer boards embedded in the projects response (included.boards)
        let mut boards: Vec<PlankaBoard> = Vec::new();
        if let Some(included_boards) = v
            .get("included")
            .and_then(|i| i.get("boards"))
            .and_then(|b| b.as_array())
        {
            for b in included_boards {
                if let (Some(id), Some(name)) = (
                    b.get("id").and_then(|x| x.as_str()),
                    b.get("name").and_then(|x| x.as_str())
                        .or_else(|| b.get("title").and_then(|x| x.as_str())),
                ) {
                    let project_id = b.get("projectId").and_then(|x| x.as_str()).map(|s| s.to_string());
                    let project_name = project_id.as_ref().and_then(|pid| proj_names.get(pid)).cloned();
                    boards.push(PlankaBoard { id: id.to_string(), name: name.to_string(), project_id, project_name });
                }
            }
        }
        if !boards.is_empty() {
            return Ok(boards);
        }

        // 3) Fallback: query boards per project (skip HTML responses)
        for pid in project_ids {
            let url = format!("{}/api/projects/{}/boards", base, pid);
            #[cfg(debug_assertions)]
            log_http_request(
                "GET",
                &url,
                &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("X-Requested-With", "XMLHttpRequest")],
                None,
            );
            let resp = self
                .client
                .get(&url)
                .header("Authorization", auth.clone())
                .header("Accept", "application/json")
                .header("X-Requested-With", "XMLHttpRequest")
                .send()
                .map_err(|e| format!("GET {} failed: {}", url, e))?;
            let status = resp.status();
            let text = resp.text().map_err(|e| format!("read {} failed: {}", url, e))?;
            #[cfg(debug_assertions)]
            log_http_response(status.as_u16(), &text);
            if !status.is_success() {
                continue;
            }
            // Skip HTML SPA responses
            if text.trim_start().starts_with('<') {
                continue;
            }
            let v: Value = serde_json::from_str(&text)
                .map_err(|e| format!("parse boards failed: {}", e))?;
            if let Some(arr) = v.as_array() {
                for b in arr {
                    if let (Some(id), Some(name)) = (
                        b.get("id").and_then(|x| x.as_str()),
                        b.get("name").and_then(|x| x.as_str())
                            .or_else(|| b.get("title").and_then(|x| x.as_str())),
                    ) {
                        let project_id = b
                            .get("projectId")
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string())
                            .or_else(|| Some(pid.clone()));
                        let project_name = project_id.as_ref().and_then(|p| proj_names.get(p)).cloned();
                        boards.push(PlankaBoard {
                            id: id.to_string(),
                            name: name.to_string(),
                            project_id,
                            project_name,
                        });
                    }
                }
            } else if let Some(items) = v.get("items").and_then(|x| x.as_array()) {
                for b in items {
                    if let (Some(id), Some(name)) = (
                        b.get("id").and_then(|x| x.as_str()),
                        b.get("name").and_then(|x| x.as_str())
                            .or_else(|| b.get("title").and_then(|x| x.as_str())),
                    ) {
                        let project_id = b
                            .get("projectId")
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string())
                            .or_else(|| Some(pid.clone()));
                        let project_name = project_id.as_ref().and_then(|p| proj_names.get(p)).cloned();
                        boards.push(PlankaBoard {
                            id: id.to_string(),
                            name: name.to_string(),
                            project_id,
                            project_name,
                        });
                    }
                }
            }
        }
        Ok(boards)
    }

    pub fn resolve_lists(&self, board_name: &str) -> Result<PlankaLists, String> {
        let boards = self.fetch_boards()?;
        let board = boards
            .into_iter()
            .find(|b| b.name.eq_ignore_ascii_case(board_name))
            .ok_or_else(|| format!("Board '{}' not found on Planka", board_name))?;
        let base = self.base_url.trim_end_matches('/');
        let mut lists: Vec<(String, String)> = Vec::new(); // (id, name)

        // Attempt 1: GET /api/boards/{id}?include=lists
        {
            let url = format!("{}/api/boards/{}?include=lists", base, board.id);
            let auth = self.auth_header();
            #[cfg(debug_assertions)]
            log_http_request(
                "GET",
                &url,
                &[
                    ("Authorization", auth.as_str()),
                    ("Accept", "application/json"),
                    ("X-Requested-With", "XMLHttpRequest"),
                ],
                None,
            );
            let resp = self
                .client
                .get(&url)
                .header("Authorization", auth.clone())
                .header("Accept", "application/json")
                .header("X-Requested-With", "XMLHttpRequest")
                .send()
                .map_err(|e| format!("GET {} failed: {}", url, e))?;
            let status = resp.status();
            let text = resp.text().map_err(|e| format!("read {} failed: {}", url, e))?;
            #[cfg(debug_assertions)]
            log_http_response(status.as_u16(), &text);
            if status.is_success() && !text.trim_start().starts_with('<') {
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    if let Some(arr) = v
                        .get("included")
                        .and_then(|i| i.get("lists"))
                        .and_then(|x| x.as_array())
                    {
                        for l in arr {
                            if let (Some(id), Some(name)) = (
                                l.get("id").and_then(|x| x.as_str()),
                                l.get("name")
                                    .and_then(|x| x.as_str())
                                    .or_else(|| l.get("title").and_then(|x| x.as_str())),
                            ) {
                                lists.push((id.to_string(), name.to_string()));
                            }
                        }
                    }
                }
            }
        }

        // Attempt 2: GET /api/projects/{projectId}?include=boards,lists and filter by boardId
        if lists.is_empty() {
            if let Some(ref project_id) = board.project_id {
                let url = format!("{}/api/projects/{}?include=boards,lists", base, project_id);
                let auth = self.auth_header();
                #[cfg(debug_assertions)]
                log_http_request(
                    "GET",
                    &url,
                    &[
                        ("Authorization", auth.as_str()),
                        ("Accept", "application/json"),
                        ("X-Requested-With", "XMLHttpRequest"),
                    ],
                    None,
                );
                let resp = self
                    .client
                    .get(&url)
                    .header("Authorization", auth.clone())
                    .header("Accept", "application/json")
                    .header("X-Requested-With", "XMLHttpRequest")
                    .send()
                    .map_err(|e| format!("GET {} failed: {}", url, e))?;
                let status = resp.status();
                let text = resp.text().map_err(|e| format!("read {} failed: {}", url, e))?;
                #[cfg(debug_assertions)]
                log_http_response(status.as_u16(), &text);
                if status.is_success() && !text.trim_start().starts_with('<') {
                    if let Ok(v) = serde_json::from_str::<Value>(&text) {
                        if let Some(arr) = v
                            .get("included")
                            .and_then(|i| i.get("lists"))
                            .and_then(|x| x.as_array())
                        {
                            for l in arr {
                                let belongs = l
                                    .get("boardId")
                                    .and_then(|x| x.as_str())
                                    .map(|s| s == board.id)
                                    .unwrap_or(false);
                                if !belongs {
                                    continue;
                                }
                                if let (Some(id), Some(name)) = (
                                    l.get("id").and_then(|x| x.as_str()),
                                    l.get("name")
                                        .and_then(|x| x.as_str())
                                        .or_else(|| l.get("title").and_then(|x| x.as_str())),
                                ) {
                                    lists.push((id.to_string(), name.to_string()));
                                }
                            }
                        }
                    }
                }
            }
        }

        if lists.is_empty() {
            return Err("No lists found for board".into());
        }

        // Match names to todo/doing/done (case- and space-insensitive)
        let mut todo_id: Option<String> = None;
        let mut doing_id: Option<String> = None;
        let mut done_id: Option<String> = None;
        for (id, name) in &lists {
            let n = name.to_lowercase().replace(' ', "");
            if n.contains("todo") || n.contains("to-do") || n.contains("to_do") || n.contains("to.do") {
                if todo_id.is_none() { todo_id = Some(id.clone()); }
            } else if n.contains("doing") || n.contains("inprogress") || n.contains("in-progress") || n.contains("in_progress") {
                if doing_id.is_none() { doing_id = Some(id.clone()); }
            } else if n.contains("done") || n.contains("completed") || n.contains("complete") {
                if done_id.is_none() { done_id = Some(id.clone()); }
            }
        }
        let todo = todo_id.ok_or_else(|| "Couldn't find a 'Todo' list on board".to_string())?;
        let doing = doing_id.unwrap_or_else(|| todo.clone());
        let done = done_id.ok_or_else(|| "Couldn't find a 'Done' list on board".to_string())?;
        Ok(PlankaLists {
            board_id: board.id,
            todo_list_id: todo,
            doing_list_id: doing,
            done_list_id: done,
        })
    }

    pub fn create_project(&self, name: &str) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/projects", base);
        let auth = self.auth_header();
        let body = json!({ "type": "private", "name": name });
        #[cfg(debug_assertions)]
        log_http_request(
            "POST",
            &url,
            &[
                ("Authorization", auth.as_str()),
                ("Accept", "application/json"),
                ("Content-Type", "application/json"),
            ],
            Some(&body.to_string()),
        );
        let resp = self.client
            .post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() {
            return Err(format!("Create project failed: HTTP {} - {}", status, text));
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse create_project failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str())
            .or_else(|| v.get("id").and_then(|x| x.as_str()))
            .map(|s| s.to_string())
            .ok_or_else(|| "Create project response missing id".to_string())
    }

    pub fn create_board(&self, project_id: &str, name: &str) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        // Per API: POST /projects/{projectId}/boards
        let url = format!("{}/api/projects/{}/boards", base, project_id);
        let auth = self.auth_header();
        let body = json!({ "name": name, "position": 65536 });
        #[cfg(debug_assertions)]
        log_http_request(
            "POST",
            &url,
            &[
                ("Authorization", auth.as_str()),
                ("Accept", "application/json"),
                ("Content-Type", "application/json"),
            ],
            Some(&body.to_string()),
        );
        let resp = self.client
            .post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() {
            return Err(format!("Create board failed: HTTP {} - {}", status, text));
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse create_board failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str())
            .or_else(|| v.get("id").and_then(|x| x.as_str()))
            .map(|s| s.to_string())
            .ok_or_else(|| "Create board response missing id".to_string())
    }

    pub fn create_card(&self, list_id: &str, name: &str, due: Option<&str>) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/lists/{}/cards", base, list_id);
        let auth = self.auth_header();
        // Build body
        let mut body = Map::new();
        body.insert("name".to_string(), Value::String(name.to_string()));
        if let Some(d) = due {
            body.insert("dueDate".to_string(), Value::String(d.to_string()));
        }
        body.insert("position".to_string(), Value::from(65536));
        body.insert("type".to_string(), Value::String("project".to_string()));
        #[cfg(debug_assertions)]
        {
            let preview = Value::Object(body.clone());
            log_http_request(
                "POST",
                &url,
                &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("X-Requested-With", "XMLHttpRequest"), ("Content-Type", "application/json")],
                Some(&preview.to_string()),
            );
        }
        let resp = self.client
            .post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().map_err(|e| format!("read {} failed: {}", url, e))?;
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() {
            return Err(format!("Create card failed: HTTP {} - {}", status, text));
        }
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| format!("parse create_card failed: {}", e))?;
        if let Some(id) = v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str())
            .or_else(|| v.get("id").and_then(|x| x.as_str())) {
            Ok(id.to_string())
        } else {
            Err("Create card response missing id".into())
        }
    }

    pub fn move_card(&self, card_id: &str, to_list_id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/cards/{}", base, card_id);
        let auth = self.auth_header();
        let body = json!({
            "listId": to_list_id,
            "position": 65536
        });
        #[cfg(debug_assertions)]
        log_http_request(
            "PATCH",
            &url,
            &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")],
            Some(&body.to_string()),
        );
        let resp = self.client
            .patch(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() {
            return Err(format!("Move card failed: HTTP {} - {}", status, text));
        }
        Ok(())
    }

    pub fn update_card(&self, card_id: &str, name: Option<&str>, due: Option<&str>) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/cards/{}", base, card_id);
        let auth = self.auth_header();
        let mut body = Map::new();
        if let Some(n) = name {
            body.insert("name".to_string(), Value::String(n.to_string()));
        }
        if let Some(d) = due {
            body.insert("dueDate".to_string(), Value::String(d.to_string()));
        }
        if body.is_empty() {
            return Ok(());
        }
        #[cfg(debug_assertions)]
        {
            let preview = Value::Object(body.clone());
            log_http_request(
                "PATCH",
                &url,
                &[
                    ("Authorization", auth.as_str()),
                    ("Accept", "application/json"),
                    ("Content-Type", "application/json"),
                ],
                Some(&preview.to_string()),
            );
        }
        let resp = self.client
            .patch(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() {
            return Err(format!("Update card failed: HTTP {} - {}", status, text));
        }
        Ok(())
    }

    pub fn delete_card(&self, card_id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/cards/{}", base, card_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request(
            "DELETE",
            &url,
            &[
                ("Authorization", auth.as_str()),
                ("Accept", "application/json"),
                ("X-Requested-With", "XMLHttpRequest"),
            ],
            None,
        );
        let resp = self
            .client
            .delete(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .send()
            .map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() {
            return Err(format!("Delete card failed: HTTP {} - {}", status, text));
        }
        Ok(())
    }

    pub fn fetch_card_created(&self, card_id: &str) -> Result<Option<String>, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/cards/{}", base, card_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request(
            "GET",
            &url,
            &[
                ("Authorization", auth.as_str()),
                ("Accept", "application/json"),
                ("X-Requested-With", "XMLHttpRequest"),
            ],
            None,
        );
        let resp = self
            .client
            .get(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .send()
            .map_err(|e| format!("GET {} failed: {}", url, e))?;
        let status = resp.status().as_u16();
        let body = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status, &body);
        if status != 200 {
            return Ok(None);
        }
        let parsed: CardDetailsRes =
            serde_json::from_str(&body).map_err(|e| format!("Parse card details failed: {}", e))?;
        Ok(parsed.item.created_at)
    }

    pub fn fetch_cards(&self, list_id: &str) -> Result<Vec<PlankaCard>, String> {
        let base = self.base_url.trim_end_matches('/');
        let auth = self.auth_header();
        // Try 1: /api/lists/{id}?include=cards
        let url1 = format!("{}/api/lists/{}?include=cards", base, list_id);
        #[cfg(debug_assertions)]
        log_http_request(
            "GET",
            &url1,
            &[
                ("Authorization", auth.as_str()),
                ("Accept", "application/json"),
                ("X-Requested-With", "XMLHttpRequest"),
            ],
            None,
        );
        let resp1 = self.client
            .get(&url1)
            .header("Authorization", auth.clone())
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .send();
        if let Ok(r) = resp1 {
            let status = r.status();
            let text = r.text().unwrap_or_default();
            #[cfg(debug_assertions)]
            log_http_response(status.as_u16(), &text);
            if status.is_success() && !text.trim_start().starts_with('<') {
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    if let Some(arr) = v
                        .get("included")
                        .and_then(|i| i.get("cards"))
                        .and_then(|x| x.as_array())
                    {
                        let mut out = Vec::new();
                        for c in arr {
                            if let (Some(id), Some(name)) = (
                                c.get("id").and_then(|x| x.as_str()),
                                c.get("name").and_then(|x| x.as_str()),
                            ) {
                                let due = c.get("dueDate").and_then(|x| x.as_str()).map(|s| s.to_string());
                                let created = c.get("createdAt").and_then(|x| x.as_str()).map(|s| s.to_string());
                                out.push(PlankaCard { id: id.to_string(), name: name.to_string(), due, created });
                            }
                        }
                        // Enrich created from card details if missing
                        for c in &mut out {
                            if c.created.is_none() {
                                if let Ok(created) = self.fetch_card_created(&c.id) {
                                    c.created = created;
                                }
                            }
                        }
                        return Ok(out);
                    }
                }
            }
        }

        // Try 2: /api/cards?listId=...
        let url2 = format!("{}/api/cards?listId={}", base, list_id);
        #[cfg(debug_assertions)]
        log_http_request(
            "GET",
            &url2,
            &[
                ("Authorization", auth.as_str()),
                ("Accept", "application/json"),
                ("X-Requested-With", "XMLHttpRequest"),
            ],
            None,
        );
        let resp2 = self.client
            .get(&url2)
            .header("Authorization", auth.clone())
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .send()
            .map_err(|e| format!("GET {} failed: {}", url2, e))?;
        let status = resp2.status();
        let text = resp2.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() || text.trim_start().starts_with('<') {
            return Ok(vec![]);
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse cards failed: {}", e))?;
        let mut out = Vec::new();
        if let Some(arr) = v.as_array() {
            for c in arr {
                if let (Some(id), Some(name)) = (
                    c.get("id").and_then(|x| x.as_str()),
                    c.get("name").and_then(|x| x.as_str()),
                ) {
                    let due = c.get("dueDate").and_then(|x| x.as_str()).map(|s| s.to_string());
                    let created = c.get("createdAt").and_then(|x| x.as_str()).map(|s| s.to_string());
                    out.push(PlankaCard { id: id.to_string(), name: name.to_string(), due, created });
                }
            }
        } else if let Some(items) = v.get("items").and_then(|x| x.as_array()) {
            for c in items {
                if let (Some(id), Some(name)) = (
                    c.get("id").and_then(|x| x.as_str()),
                    c.get("name").and_then(|x| x.as_str()),
                ) {
                    let due = c.get("dueDate").and_then(|x| x.as_str()).map(|s| s.to_string());
                    let created = c.get("createdAt").and_then(|x| x.as_str()).map(|s| s.to_string());
                    out.push(PlankaCard { id: id.to_string(), name: name.to_string(), due, created });
                }
            }
        }
        // Enrich created from card details if missing
        for c in &mut out {
            if c.created.is_none() {
                if let Ok(created) = self.fetch_card_created(&c.id) {
                    c.created = created;
                }
            }
        }
        Ok(out)
    }

    pub fn fetch_card_details(&self, card_id: &str) -> Result<PlankaCardDetails, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/cards/{}", base, card_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request(
            "GET",
            &url,
            &[
                ("Authorization", auth.as_str()),
                ("Accept", "application/json"),
                ("X-Requested-With", "XMLHttpRequest"),
            ],
            None,
        );
        let resp = self.client
            .get(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .send()
            .map_err(|e| format!("GET {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() || text.trim_start().starts_with('<') {
            return Err(format!("Fetch card details failed: HTTP {} - {}", status, text));
        }
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| format!("Parse card details failed: {}", e))?;
        let item = v.get("item").and_then(|x| x.as_object())
            .ok_or_else(|| "Missing item".to_string())?;

        let id = item.get("id").and_then(|x| x.as_str()).unwrap_or(card_id).to_string();
        let name = item.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let description = item.get("description").and_then(|x| x.as_str()).map(|s| s.to_string());
        let due = item.get("dueDate").and_then(|x| x.as_str()).map(|s| s.to_string());
        let is_due_completed = item.get("isDueCompleted").and_then(|x| x.as_bool());
        let created = item.get("createdAt").and_then(|x| x.as_str()).map(|s| s.to_string());
        let updated = item.get("updatedAt").and_then(|x| x.as_str()).map(|s| s.to_string());
        let list_id = item.get("listId").and_then(|x| x.as_str()).map(|s| s.to_string());
        let board_id = item.get("boardId").and_then(|x| x.as_str()).map(|s| s.to_string());

        let included = v.get("included").and_then(|x| x.as_object());

        let mut list_name: Option<String> = None;
        let mut labels: Vec<String> = Vec::new();
        let mut attachments: Vec<String> = Vec::new();
        let mut tasks: Vec<(String, bool)> = Vec::new();
        let mut attachments_full: Vec<PlankaAttachment> = Vec::new();
        let mut tasks_full: Vec<PlankaTask> = Vec::new();
        let mut task_lists: Vec<(String, String)> = Vec::new();

        if let Some(inc) = included {
            // lists -> find current list name
            if let (Some(lists), Some(lid)) = (inc.get("lists").and_then(|x| x.as_array()), list_id.as_ref()) {
                for l in lists {
                    let lid_json = l.get("id").and_then(|x| x.as_str());
                    if lid_json == Some(lid.as_str()) {
                        if let Some(n) = l.get("name").and_then(|x| x.as_str()).or_else(|| l.get("title").and_then(|x| x.as_str())) {
                            list_name = Some(n.to_string());
                            break;
                        }
                    }
                }
            }
            // labels: from labels + cardLabels
            let mut label_by_id: std::collections::HashMap<String, String> = std::collections::HashMap::new();
            if let Some(arr) = inc.get("labels").and_then(|x| x.as_array()) {
                for lab in arr {
                    if let (Some(id), Some(name)) = (
                        lab.get("id").and_then(|x| x.as_str()),
                        lab.get("name").and_then(|x| x.as_str()),
                    ) {
                        label_by_id.insert(id.to_string(), name.to_string());
                    }
                }
            }
            if let Some(arr) = inc.get("cardLabels").and_then(|x| x.as_array()) {
                for cl in arr {
                    let cl_card = cl.get("cardId").and_then(|x| x.as_str());
                    if cl_card == Some(id.as_str()) {
                        if let Some(lid) = cl.get("labelId").and_then(|x| x.as_str()) {
                            if let Some(name) = label_by_id.get(lid) {
                                labels.push(name.clone());
                            }
                        }
                    }
                }
            }
            // attachments: take names and collect full models
            if let Some(arr) = inc.get("attachments").and_then(|x| x.as_array()) {
                for a in arr {
                    if let Some(id) = a.get("id").and_then(|x| x.as_str()) {
                        let name = a.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                        let url = a.get("data")
                            .and_then(|x| x.as_object())
                            .and_then(|o| o.get("url"))
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string());
                        attachments.push(if let Some(ref u) = url { format!("{} ({})", name, u) } else { name.clone() });
                        attachments_full.push(PlankaAttachment { id: id.to_string(), name, url });
                    }
                }
            }
            // tasks: included.tasks
            if let Some(arr) = inc.get("tasks").and_then(|x| x.as_array()) {
                for t in arr {
                    let id = t.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    let n = t.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    let c = t.get("isCompleted").and_then(|x| x.as_bool()).unwrap_or(false);
                    let tlid = t.get("taskListId").and_then(|x| x.as_str()).map(|s| s.to_string());
                    tasks.push((n.clone(), c));
                    tasks_full.push(PlankaTask { id, name: n, is_completed: c, task_list_id: tlid });
                }
            }
            // task lists
            if let Some(arr) = inc.get("taskLists").and_then(|x| x.as_array()) {
                for tl in arr {
                    if let (Some(id), Some(name)) = (
                        tl.get("id").and_then(|x| x.as_str()),
                        tl.get("name").and_then(|x| x.as_str()).or_else(|| tl.get("title").and_then(|x| x.as_str())),
                    ) {
                        task_lists.push((id.to_string(), name.to_string()));
                    }
                }
            }
        }

        // custom field groups, fields, values
        let mut cfg_name_by_id: HashMap<String, Option<String>> = HashMap::new();
        let mut fields_by_group: HashMap<String, Vec<PlankaCustomField>> = HashMap::new();
        let mut values_by_group: HashMap<String, HashMap<String, String>> = HashMap::new();
        if let Some(inc) = included {
            if let Some(arr) = inc.get("customFieldGroups").and_then(|x| x.as_array()) {
                for g in arr {
                    if let Some(gid) = g.get("id").and_then(|x| x.as_str()) {
                        let name = g.get("name").and_then(|x| x.as_str()).map(|s| s.to_string());
                        cfg_name_by_id.insert(gid.to_string(), name);
                    }
                }
            }
            if let Some(arr) = inc.get("customFields").and_then(|x| x.as_array()) {
                for f in arr {
                    if let (Some(fid), Some(name)) = (
                        f.get("id").and_then(|x| x.as_str()),
                        f.get("name").and_then(|x| x.as_str()),
                    ) {
                        if let Some(gid) = f.get("customFieldGroupId").and_then(|x| x.as_str()) {
                            fields_by_group
                                .entry(gid.to_string())
                                .or_default()
                                .push(PlankaCustomField {
                                    id: fid.to_string(),
                                    name: name.to_string(),
                                    show_on_front_of_card: f.get("showOnFrontOfCard").and_then(|x| x.as_bool()),
                                });
                        }
                    }
                }
            }
            if let Some(arr) = inc.get("customFieldValues").and_then(|x| x.as_array()) {
                for v in arr {
                    if let (Some(gid), Some(fid), Some(content)) = (
                        v.get("customFieldGroupId").and_then(|x| x.as_str()),
                        v.get("customFieldId").and_then(|x| x.as_str()),
                        v.get("content").and_then(|x| x.as_str()),
                    ) {
                        values_by_group
                            .entry(gid.to_string())
                            .or_default()
                            .insert(fid.to_string(), content.to_string());
                    }
                }
            }
        }
        let mut custom_field_groups: Vec<PlankaCustomFieldGroupDetails> = Vec::new();
        for (gid, name) in cfg_name_by_id {
            let mut fields = fields_by_group.remove(&gid).unwrap_or_default();
            fields.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            let values = values_by_group.remove(&gid).unwrap_or_default();
            custom_field_groups.push(PlankaCustomFieldGroupDetails { id: gid, name, fields, values_by_field: values });
        }
        custom_field_groups.sort_by(|a, b| a.name.as_deref().unwrap_or("").to_lowercase().cmp(&b.name.as_deref().unwrap_or("").to_lowercase()));
        Ok(PlankaCardDetails {
            id, name, description, due, is_due_completed, created, updated, list_name, labels, attachments, tasks,
            board_id, attachments_full, tasks_full, task_lists, custom_field_groups,
        })
    }
    pub fn fetch_comments(&self, card_id: &str) -> Result<Vec<PlankaComment>, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/cards/{}/comments", base, card_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request(
            "GET",
            &url,
            &[
                ("Authorization", auth.as_str()),
                ("Accept", "application/json"),
                ("X-Requested-With", "XMLHttpRequest"),
            ],
            None,
        );
        let resp = self.client
            .get(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .send()
            .map_err(|e| format!("GET {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() || text.trim_start().starts_with('<') {
            return Err(format!("Fetch comments failed: HTTP {} - {}", status, text));
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("Parse comments failed: {}", e))?;
        // Map userId -> user name
        let mut user_name_by_id: HashMap<String, String> = HashMap::new();
        if let Some(included) = v.get("included").and_then(|x| x.as_object()) {
            if let Some(users) = included.get("users").and_then(|x| x.as_array()) {
                for u in users {
                    if let Some(uid) = u.get("id").and_then(|x| x.as_str()) {
                        if let Some(name) = u.get("name").and_then(|x| x.as_str()).or_else(|| u.get("username").and_then(|x| x.as_str())) {
                            user_name_by_id.insert(uid.to_string(), name.to_string());
                        }
                    }
                }
            }
        }
        // Collect items
        let mut out: Vec<PlankaComment> = Vec::new();
        if let Some(items) = v.get("items").and_then(|x| x.as_array()) {
            for c in items {
                let id = c.get("id").and_then(|x| x.as_str()).unwrap_or_default().to_string();
                let user_id = c.get("userId").and_then(|x| x.as_str()).map(|s| s.to_string());
                let text = c.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let created = c.get("createdAt").and_then(|x| x.as_str()).map(|s| s.to_string());
                let user_name = user_id.as_ref().and_then(|uid| user_name_by_id.get(uid)).cloned();
                out.push(PlankaComment { id, user_id, user_name, text, created });
            }
        }
        Ok(out)
    }

    pub fn create_comment(&self, card_id: &str, text: &str) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/cards/{}/comments", base, card_id);
        let auth = self.auth_header();
        let body = json!({ "text": text });
        #[cfg(debug_assertions)]
        log_http_request(
            "POST",
            &url,
            &[
                ("Authorization", auth.as_str()),
                ("Accept", "application/json"),
                ("X-Requested-With", "XMLHttpRequest"),
                ("Content-Type", "application/json"),
            ],
            Some(&body.to_string()),
        );
        let resp = self.client
            .post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() {
            return Err(format!("Create comment failed: HTTP {} - {}", status, text));
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("Parse create comment failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "Create comment response missing id".to_string())
    }

    pub fn update_comment(&self, comment_id: &str, text: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/comments/{}", base, comment_id);
        let auth = self.auth_header();
        let body = json!({ "text": text });
        #[cfg(debug_assertions)]
        log_http_request("PATCH", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&body.to_string()));
        let resp = self.client.patch(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Update comment failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn delete_comment(&self, comment_id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/comments/{}", base, comment_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("DELETE", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], None);
        let resp = self.client.delete(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .send()
            .map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Delete comment failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn create_link_attachment(&self, card_id: &str, url_str: &str, name: &str) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/cards/{}/attachments", base, card_id);
        let auth = self.auth_header();
        let form = Form::new()
            .text("type", "link".to_string())
            .text("url", url_str.to_string())
            .text("name", name.to_string());
        #[cfg(debug_assertions)]
        log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], Some("[multipart form]"));
        let resp = self.client
            .post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .multipart(form)
            .send()
            .map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Create attachment failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("Parse create attachment failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str()).map(|s| s.to_string()).ok_or_else(|| "Create attachment response missing id".to_string())
    }

    pub fn delete_attachment(&self, attachment_id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/attachments/{}", base, attachment_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("DELETE", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], None);
        let resp = self.client
            .delete(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .send()
            .map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Delete attachment failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn create_task_list(&self, card_id: &str, name: &str) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/cards/{}/task-lists", base, card_id);
        let auth = self.auth_header();
        let body = json!({ "position": 65536, "name": name });
        #[cfg(debug_assertions)]
        log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&body.to_string()));
        let resp = self.client.post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Create task list failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("Parse create task list failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str()).map(|s| s.to_string()).ok_or_else(|| "Create task list response missing id".to_string())
    }

    pub fn create_task(&self, task_list_id: &str, name: &str) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/task-lists/{}/tasks", base, task_list_id);
        let auth = self.auth_header();
        let body = json!({ "position": 65536, "name": name });
        #[cfg(debug_assertions)]
        log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&body.to_string()));
        let resp = self.client.post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Create task failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("Parse create task failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str()).map(|s| s.to_string()).ok_or_else(|| "Create task response missing id".to_string())
    }

    pub fn update_task(&self, task_id: &str, name: Option<&str>, is_completed: Option<bool>) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/tasks/{}", base, task_id);
        let auth = self.auth_header();
        let mut body = Map::new();
        if let Some(n) = name { body.insert("name".to_string(), Value::String(n.to_string())); }
        if let Some(c) = is_completed { body.insert("isCompleted".to_string(), Value::Bool(c)); }
        if body.is_empty() { return Ok(()); }
        #[cfg(debug_assertions)]
        { let preview = Value::Object(body.clone()); log_http_request("PATCH", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&preview.to_string())); }
        let resp = self.client.patch(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Update task failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn delete_task(&self, task_id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/tasks/{}", base, task_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("DELETE", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], None);
        let resp = self.client.delete(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .send()
            .map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Delete task failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn fetch_board_actions(&self, board_id: &str, before_id: Option<&str>) -> Result<Vec<PlankaAction>, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = match before_id {
            Some(b) => format!("{}/api/boards/{}/actions?beforeId={}", base, board_id, b),
            None => format!("{}/api/boards/{}/actions", base, board_id),
        };
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("GET", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("X-Requested-With", "XMLHttpRequest")], None);
        let resp = self.client.get(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .send()
            .map_err(|e| format!("GET {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() || text.trim_start().starts_with('<') {
            return Err(format!("Fetch board actions failed: HTTP {} - {}", status, text));
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse actions failed: {}", e))?;
        let mut out = Vec::new();
        if let Some(items) = v.get("items").and_then(|x| x.as_array()) {
            for a in items {
                let id = a.get("id").and_then(|x| x.as_str()).unwrap_or_default().to_string();
                let type_ = a.get("type").and_then(|x| x.as_str()).unwrap_or_default().to_string();
                let user_id = a.get("userId").and_then(|x| x.as_str()).map(|s| s.to_string());
                let card_id = a.get("cardId").and_then(|x| x.as_str()).map(|s| s.to_string());
                let created = a.get("createdAt").and_then(|x| x.as_str()).map(|s| s.to_string());
                let data = a.get("data").cloned();
                out.push(PlankaAction { id, type_, user_id, card_id, data, created });
            }
        }
        Ok(out)
    }

    pub fn fetch_card_actions(&self, card_id: &str, before_id: Option<&str>) -> Result<Vec<PlankaAction>, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = match before_id {
            Some(b) => format!("{}/api/cards/{}/actions?beforeId={}", base, card_id, b),
            None => format!("{}/api/cards/{}/actions", base, card_id),
        };
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("GET", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("X-Requested-With", "XMLHttpRequest")], None);
        let resp = self.client.get(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .send()
            .map_err(|e| format!("GET {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() || text.trim_start().starts_with('<') {
            return Err(format!("Fetch card actions failed: HTTP {} - {}", status, text));
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse actions failed: {}", e))?;
        let mut out = Vec::new();
        if let Some(items) = v.get("items").and_then(|x| x.as_array()) {
            for a in items {
                let id = a.get("id").and_then(|x| x.as_str()).unwrap_or_default().to_string();
                let type_ = a.get("type").and_then(|x| x.as_str()).unwrap_or_default().to_string();
                let user_id = a.get("userId").and_then(|x| x.as_str()).map(|s| s.to_string());
                let card_id = a.get("cardId").and_then(|x| x.as_str()).map(|s| s.to_string());
                let created = a.get("createdAt").and_then(|x| x.as_str()).map(|s| s.to_string());
                let data = a.get("data").cloned();
                out.push(PlankaAction { id, type_, user_id, card_id, data, created });
            }
        }
        Ok(out)
    }

    pub fn add_label_to_card(&self, card_id: &str, label_id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/cards/{}/card-labels", base, card_id);
        let auth = self.auth_header();
        let body = json!({ "labelId": label_id });
        #[cfg(debug_assertions)]
        log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&body.to_string()));
        let resp = self.client.post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Add label failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn remove_label_from_card(&self, card_id: &str, label_id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/cards/{}/card-labels/labelId:{}", base, card_id, label_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("DELETE", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], None);
        let resp = self.client.delete(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .send()
            .map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Remove label failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn add_member_to_card(&self, card_id: &str, user_id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/cards/{}/card-memberships", base, card_id);
        let auth = self.auth_header();
        let body = json!({ "userId": user_id });
        #[cfg(debug_assertions)]
        log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&body.to_string()));
        let resp = self.client.post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Add member failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn remove_member_from_card(&self, card_id: &str, user_id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/cards/{}/card-memberships/userId:{}", base, card_id, user_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("DELETE", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], None);
        let resp = self.client.delete(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .send()
            .map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Remove member failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn create_project_manager(&self, project_id: &str, user_id: &str) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/projects/{}/project-managers", base, project_id);
        let auth = self.auth_header();
        let body = json!({ "userId": user_id });
        #[cfg(debug_assertions)]
        log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&body.to_string()));
        let resp = self.client.post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Create project manager failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse project manager failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str()).map(|s| s.to_string()).ok_or_else(|| "Response missing id".to_string())
    }

    pub fn delete_project_manager(&self, id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/project-managers/{}", base, id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("DELETE", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], None);
        let resp = self.client.delete(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .send()
            .map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Delete project manager failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn fetch_projects(&self) -> Result<Vec<PlankaProject>, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/projects", base);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("GET", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("X-Requested-With", "XMLHttpRequest")], None);
        let resp = self.client.get(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .send()
            .map_err(|e| format!("GET {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() || text.trim_start().starts_with('<') {
            return Err(format!("Fetch projects failed: HTTP {} - {}", status, text));
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse projects failed: {}", e))?;
        let iter = v.as_array()
            .cloned()
            .or_else(|| v.get("items").and_then(|x| x.as_array()).cloned())
            .unwrap_or_default();
        let mut out = Vec::new();
        for p in iter {
            if let Some(id) = p.get("id").and_then(|x| x.as_str()) {
                let name = p.get("name").and_then(|x| x.as_str()).or_else(|| p.get("title").and_then(|x| x.as_str())).unwrap_or("").to_string();
                let description = p.get("description").and_then(|x| x.as_str()).map(|s| s.to_string());
                let is_hidden = p.get("isHidden").and_then(|x| x.as_bool());
                out.push(PlankaProject { id: id.to_string(), name, description, is_hidden });
            }
        }
        Ok(out)
    }

    pub fn fetch_project_details(&self, project_id: &str) -> Result<PlankaProjectDetails, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/projects/{}?include=boards", base, project_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("GET", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("X-Requested-With", "XMLHttpRequest")], None);
        let resp = self.client.get(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .send()
            .map_err(|e| format!("GET {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() || text.trim_start().starts_with('<') {
            return Err(format!("Fetch project failed: HTTP {} - {}", status, text));
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse project failed: {}", e))?;
        let item = v.get("item").and_then(|x| x.as_object()).ok_or_else(|| "Missing item".to_string())?;
        let id = item.get("id").and_then(|x| x.as_str()).unwrap_or(project_id).to_string();
        let name = item.get("name").and_then(|x| x.as_str()).or_else(|| item.get("title").and_then(|x| x.as_str())).unwrap_or("").to_string();
        let description = item.get("description").and_then(|x| x.as_str()).map(|s| s.to_string());
        let is_hidden = item.get("isHidden").and_then(|x| x.as_bool());
        let mut boards: Vec<PlankaBoard> = Vec::new();
        if let Some(arr) = v.get("included").and_then(|i| i.get("boards")).and_then(|x| x.as_array()) {
            for b in arr {
                if let (Some(bid), Some(bname)) = (
                    b.get("id").and_then(|x| x.as_str()),
                    b.get("name").and_then(|x| x.as_str()).or_else(|| b.get("title").and_then(|x| x.as_str())),
                ) {
                    boards.push(PlankaBoard {
                        id: bid.to_string(),
                        name: bname.to_string(),
                        project_id: Some(id.clone()),
                        project_name: Some(name.clone()),
                    });
                }
            }
        }
        Ok(PlankaProjectDetails { id, name, description, is_hidden, boards })
    }

    pub fn update_project(
        &self,
        project_id: &str,
        name: Option<&str>,
        description: Option<&str>,
        is_hidden: Option<bool>,
        is_favorite: Option<bool>,
        background_type: Option<&str>,
        background_gradient: Option<&str>,
    ) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/projects/{}", base, project_id);
        let auth = self.auth_header();
        let mut body = Map::new();
        if let Some(n) = name { body.insert("name".to_string(), Value::String(n.to_string())); }
        if let Some(d) = description { body.insert("description".to_string(), Value::String(d.to_string())); }
        if let Some(h) = is_hidden { body.insert("isHidden".to_string(), Value::Bool(h)); }
        if let Some(f) = is_favorite { body.insert("isFavorite".to_string(), Value::Bool(f)); }
        if let Some(bt) = background_type { body.insert("backgroundType".to_string(), Value::String(bt.to_string())); }
        if let Some(bg) = background_gradient { body.insert("backgroundGradient".to_string(), Value::String(bg.to_string())); }
        if body.is_empty() { return Ok(()); }
        #[cfg(debug_assertions)]
        { let preview = Value::Object(body.clone()); log_http_request("PATCH", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&preview.to_string())); }
        let resp = self.client.patch(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Update project failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn delete_project(&self, project_id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/projects/{}", base, project_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("DELETE", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], None);
        let resp = self.client.delete(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .send()
            .map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Delete project failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn fetch_task_list(&self, task_list_id: &str) -> Result<PlankaTaskListDetails, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/task-lists/{}", base, task_list_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("GET", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("X-Requested-With", "XMLHttpRequest")], None);
        let resp = self.client.get(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .send()
            .map_err(|e| format!("GET {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() || text.trim_start().starts_with('<') {
            return Err(format!("Fetch task list failed: HTTP {} - {}", status, text));
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse task list failed: {}", e))?;
        let item = v.get("item").and_then(|x| x.as_object()).ok_or_else(|| "Missing item".to_string())?;
        let id = item.get("id").and_then(|x| x.as_str()).unwrap_or(task_list_id).to_string();
        let name = item.get("name").and_then(|x| x.as_str()).or_else(|| item.get("title").and_then(|x| x.as_str())).unwrap_or("").to_string();
        let mut tasks: Vec<PlankaTask> = Vec::new();
        if let Some(arr) = v.get("included").and_then(|i| i.get("tasks")).and_then(|x| x.as_array()) {
            for t in arr {
                let tid = t.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let n = t.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let c = t.get("isCompleted").and_then(|x| x.as_bool()).unwrap_or(false);
                let tlid = t.get("taskListId").and_then(|x| x.as_str()).map(|s| s.to_string());
                tasks.push(PlankaTask { id: tid, name: n, is_completed: c, task_list_id: tlid });
            }
        }
        Ok(PlankaTaskListDetails { id, name, tasks })
    }

    pub fn update_task_list(
        &self,
        task_list_id: &str,
        name: Option<&str>,
        position: Option<i64>,
        show_on_front_of_card: Option<bool>,
        hide_completed_tasks: Option<bool>,
    ) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/task-lists/{}", base, task_list_id);
        let auth = self.auth_header();
        let mut body = Map::new();
        if let Some(n) = name { body.insert("name".to_string(), Value::String(n.to_string())); }
        if let Some(p) = position { body.insert("position".to_string(), Value::from(p)); }
        if let Some(s) = show_on_front_of_card { body.insert("showOnFrontOfCard".to_string(), Value::Bool(s)); }
        if let Some(h) = hide_completed_tasks { body.insert("hideCompletedTasks".to_string(), Value::Bool(h)); }
        if body.is_empty() { return Ok(()); }
        #[cfg(debug_assertions)]
        { let preview = Value::Object(body.clone()); log_http_request("PATCH", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&preview.to_string())); }
        let resp = self.client.patch(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Update task list failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn delete_task_list(&self, task_list_id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/task-lists/{}", base, task_list_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("DELETE", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], None);
        let resp = self.client.delete(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .send()
            .map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Delete task list failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn update_board(
        &self,
        board_id: &str,
        position: Option<i64>,
        name: Option<&str>,
        default_view: Option<&str>,               // "kanban" | "grid" | "list"
        default_card_type: Option<&str>,          // "project" | "story"
        limit_card_types_to_default_one: Option<bool>,
        always_display_card_creator: Option<bool>,
        expand_task_lists_by_default: Option<bool>,
        is_subscribed: Option<bool>,
    ) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/boards/{}", base, board_id);
        let auth = self.auth_header();
        let mut body = Map::new();
        if let Some(v) = position { body.insert("position".to_string(), Value::from(v)); }
        if let Some(v) = name { body.insert("name".to_string(), Value::String(v.to_string())); }
        if let Some(v) = default_view { body.insert("defaultView".to_string(), Value::String(v.to_string())); }
        if let Some(v) = default_card_type { body.insert("defaultCardType".to_string(), Value::String(v.to_string())); }
        if let Some(v) = limit_card_types_to_default_one { body.insert("limitCardTypesToDefaultOne".to_string(), Value::Bool(v)); }
        if let Some(v) = always_display_card_creator { body.insert("alwaysDisplayCardCreator".to_string(), Value::Bool(v)); }
        if let Some(v) = expand_task_lists_by_default { body.insert("expandTaskListsByDefault".to_string(), Value::Bool(v)); }
        if let Some(v) = is_subscribed { body.insert("isSubscribed".to_string(), Value::Bool(v)); }
        if body.is_empty() { return Ok(()); }
        #[cfg(debug_assertions)]
        { let preview = Value::Object(body.clone()); log_http_request("PATCH", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&preview.to_string())); }
        let resp = self.client.patch(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send().map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Update board failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn delete_board(&self, board_id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/boards/{}", base, board_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("DELETE", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], None);
        let resp = self.client.delete(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .send().map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Delete board failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn fetch_board_details(&self, board_id: &str) -> Result<PlankaBoardDetails, String> {
        let base = self.base_url.trim_end_matches('/');
        // include lists and labels explicitly
        let url = format!("{}/api/boards/{}?include=lists,labels", base, board_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("GET", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("X-Requested-With", "XMLHttpRequest")], None);
        let resp = self.client.get(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .send().map_err(|e| format!("GET {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() || text.trim_start().starts_with('<') {
            return Err(format!("Fetch board failed: HTTP {} - {}", status, text));
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse board failed: {}", e))?;
        let item = v.get("item").and_then(|x| x.as_object()).ok_or_else(|| "Missing item".to_string())?;
        let id = item.get("id").and_then(|x| x.as_str()).unwrap_or(board_id).to_string();
        let name = item.get("name").and_then(|x| x.as_str()).or_else(|| item.get("title").and_then(|x| x.as_str())).unwrap_or("").to_string();
        let project_id = item.get("projectId").and_then(|x| x.as_str()).map(|s| s.to_string());
        let mut lists = Vec::new();
        let mut labels = Vec::new();
        if let Some(inc) = v.get("included").and_then(|x| x.as_object()) {
            if let Some(arr) = inc.get("lists").and_then(|x| x.as_array()) {
                for l in arr {
                    if let (Some(lid), Some(nm)) = (l.get("id").and_then(|x| x.as_str()), l.get("name").and_then(|x| x.as_str()).or_else(|| l.get("title").and_then(|x| x.as_str()))) {
                        lists.push((lid.to_string(), nm.to_string()));
                    }
                }
            }
            if let Some(arr) = inc.get("labels").and_then(|x| x.as_array()) {
                for lab in arr {
                    let lid = lab.get("id").and_then(|x| x.as_str());
                    let nm = lab.get("name").and_then(|x| x.as_str()).or_else(|| lab.get("title").and_then(|x| x.as_str()));
                    let color = lab.get("color").and_then(|x| x.as_str());
                    if let (Some(lid), Some(nm), Some(color)) = (lid, nm, color) {
                        labels.push((lid.to_string(), nm.to_string(), color.to_string()));
                    }
                }
            }
        }
        Ok(PlankaBoardDetails { id, name, project_id, lists, labels })
    }

    pub fn create_board_membership(&self, board_id: &str, user_id: &str, role: &str, can_comment: Option<bool>) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/boards/{}/board-memberships", base, board_id);
        let auth = self.auth_header();
        let mut body = Map::new();
        body.insert("userId".to_string(), Value::String(user_id.to_string()));
        body.insert("role".to_string(), Value::String(role.to_string())); // "editor" | "viewer"
        if let Some(v) = can_comment { body.insert("canComment".to_string(), Value::Bool(v)); }
        #[cfg(debug_assertions)]
        { let preview = Value::Object(body.clone()); log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&preview.to_string())); }
        let resp = self.client.post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Create board membership failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse create board membership failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str()).map(|s| s.to_string()).ok_or_else(|| "Response missing id".to_string())
    }

    pub fn update_board_membership(&self, membership_id: &str, role: Option<&str>, can_comment: Option<bool>) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/board-memberships/{}", base, membership_id);
        let auth = self.auth_header();
        let mut body = Map::new();
        if let Some(r) = role { body.insert("role".to_string(), Value::String(r.to_string())); }
        if let Some(c) = can_comment { body.insert("canComment".to_string(), Value::Bool(c)); }
        if body.is_empty() { return Ok(()); }
        #[cfg(debug_assertions)]
        { let preview = Value::Object(body.clone()); log_http_request("PATCH", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&preview.to_string())); }
        let resp = self.client.patch(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send().map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Update board membership failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn delete_board_membership(&self, membership_id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/board-memberships/{}", base, membership_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("DELETE", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], None);
        let resp = self.client.delete(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .send().map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Delete board membership failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn create_label(&self, board_id: &str, color: &str, name: Option<&str>, position: Option<i64>) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/boards/{}/labels", base, board_id);
        let auth = self.auth_header();
        let mut body = Map::new();
        body.insert("color".to_string(), Value::String(color.to_string()));
        body.insert("position".to_string(), Value::from(position.unwrap_or(65536)));
        if let Some(n) = name { body.insert("name".to_string(), Value::String(n.to_string())); }
        #[cfg(debug_assertions)]
        { let preview = Value::Object(body.clone()); log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&preview.to_string())); }
        let resp = self.client.post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Create label failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse create label failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str()).map(|s| s.to_string()).ok_or_else(|| "Response missing id".to_string())
    }

    pub fn update_label(&self, label_id: &str, position: Option<i64>, name: Option<&str>, color: Option<&str>) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/labels/{}", base, label_id);
        let auth = self.auth_header();
        let mut body = Map::new();
        if let Some(p) = position { body.insert("position".to_string(), Value::from(p)); }
        if let Some(n) = name { body.insert("name".to_string(), Value::String(n.to_string())); }
        if let Some(c) = color { body.insert("color".to_string(), Value::String(c.to_string())); }
        if body.is_empty() { return Ok(()); }
        #[cfg(debug_assertions)]
        { let preview = Value::Object(body.clone()); log_http_request("PATCH", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&preview.to_string())); }
        let resp = self.client.patch(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send().map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Update label failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn delete_label(&self, label_id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/labels/{}", base, label_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("DELETE", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], None);
        let resp = self.client.delete(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .send().map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Delete label failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn clear_list(&self, list_id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/lists/{}/clear", base, list_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some("{}"));
        let resp = self.client.post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&json!({}))
            .send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Clear list failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn create_list(&self, board_id: &str, name: &str, list_type: Option<&str>, position: Option<i64>, color: Option<&str>) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/boards/{}/lists", base, board_id);
        let auth = self.auth_header();
        let mut body = Map::new();
        body.insert("type".to_string(), Value::String(list_type.unwrap_or("active").to_string()));
        body.insert("position".to_string(), Value::from(position.unwrap_or(65536)));
        body.insert("name".to_string(), Value::String(name.to_string()));
        if let Some(c) = color { body.insert("color".to_string(), Value::String(c.to_string())); }
        #[cfg(debug_assertions)]
        { let preview = Value::Object(body.clone()); log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&preview.to_string())); }
        let resp = self.client.post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Create list failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse create list failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str()).map(|s| s.to_string()).ok_or_else(|| "Response missing id".to_string())
    }

    pub fn fetch_list_details(&self, list_id: &str) -> Result<PlankaListDetails, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/lists/{}", base, list_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("GET", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("X-Requested-With", "XMLHttpRequest")], None);
        let resp = self.client.get(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .send().map_err(|e| format!("GET {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() || text.trim_start().starts_with('<') {
            return Err(format!("Fetch list failed: HTTP {} - {}", status, text));
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse list failed: {}", e))?;
        let item = v.get("item").and_then(|x| x.as_object()).ok_or_else(|| "Missing item".to_string())?;
        let id = item.get("id").and_then(|x| x.as_str()).unwrap_or(list_id).to_string();
        let name = item.get("name").and_then(|x| x.as_str()).or_else(|| item.get("title").and_then(|x| x.as_str())).unwrap_or("").to_string();
        let mut cards = Vec::new();
        if let Some(arr) = v.get("included").and_then(|i| i.get("cards")).and_then(|x| x.as_array()) {
            for c in arr {
                if let (Some(cid), Some(nm)) = (c.get("id").and_then(|x| x.as_str()), c.get("name").and_then(|x| x.as_str())) {
                    let due = c.get("dueDate").and_then(|x| x.as_str()).map(|s| s.to_string());
                    let created = c.get("createdAt").and_then(|x| x.as_str()).map(|s| s.to_string());
                    cards.push(PlankaCard { id: cid.to_string(), name: nm.to_string(), due, created });
                }
            }
        }
        Ok(PlankaListDetails { id, name, cards })
    }

    pub fn update_list(&self, list_id: &str, board_id: Option<&str>, list_type: Option<&str>, position: Option<i64>, name: Option<&str>, color: Option<&str>) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/lists/{}", base, list_id);
        let auth = self.auth_header();
        let mut body = Map::new();
        if let Some(b) = board_id { body.insert("boardId".to_string(), Value::String(b.to_string())); }
        if let Some(t) = list_type { body.insert("type".to_string(), Value::String(t.to_string())); }
        if let Some(p) = position { body.insert("position".to_string(), Value::from(p)); }
        if let Some(n) = name { body.insert("name".to_string(), Value::String(n.to_string())); }
        if let Some(c) = color { body.insert("color".to_string(), Value::String(c.to_string())); }
        if body.is_empty() { return Ok(()); }
        #[cfg(debug_assertions)]
        { let preview = Value::Object(body.clone()); log_http_request("PATCH", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&preview.to_string())); }
        let resp = self.client.patch(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send().map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Update list failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn delete_list(&self, list_id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/lists/{}", base, list_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("DELETE", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], None);
        let resp = self.client.delete(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .send().map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Delete list failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn move_list_cards(&self, source_list_id: &str, target_list_id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/lists/{}/move-cards", base, source_list_id);
        let auth = self.auth_header();
        let body = json!({ "listId": target_list_id });
        #[cfg(debug_assertions)]
        log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&body.to_string()));
        let resp = self.client.post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Move list cards failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn sort_list(&self, list_id: &str, field_name: &str, order: Option<&str>) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/lists/{}/sort", base, list_id);
        let auth = self.auth_header();
        let mut body = Map::new();
        body.insert("fieldName".to_string(), Value::String(field_name.to_string())); // "name" | "dueDate" | "createdAt"
        if let Some(o) = order { body.insert("order".to_string(), Value::String(o.to_string())); } // "asc" | "desc"
        #[cfg(debug_assertions)]
        { let preview = Value::Object(body.clone()); log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&preview.to_string())); }
        let resp = self.client.post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Sort list failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn create_file_attachment(&self, card_id: &str, file_path: &str, name: Option<&str>) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/cards/{}/attachments", base, card_id);
        let auth = self.auth_header();
        let mut form = Form::new()
            .text("type", "file".to_string());
        form = form.file("file", file_path)
            .map_err(|e| format!("Read file failed: {}", e))?;
        if let Some(n) = name {
            form = form.text("name", n.to_string());
        }
        #[cfg(debug_assertions)]
        log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], Some("[multipart form]"));
        let resp = self.client
            .post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .multipart(form)
            .send()
            .map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Create file attachment failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("Parse create attachment failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "Create attachment response missing id".to_string())
    }

    pub fn update_attachment_name(&self, attachment_id: &str, name: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/attachments/{}", base, attachment_id);
        let auth = self.auth_header();
        let body = json!({ "name": name });
        #[cfg(debug_assertions)]
        log_http_request("PATCH", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&body.to_string()));
        let resp = self.client
            .patch(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Update attachment failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn upload_background_image(&self, project_id: &str, file_path: &str) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/projects/{}/background-images", base, project_id);
        let auth = self.auth_header();
        let form = Form::new()
            .file("file", file_path)
            .map_err(|e| format!("Read file failed: {}", e))?;
        #[cfg(debug_assertions)]
        log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], Some("[multipart form]"));
        let resp = self.client
            .post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .multipart(form)
            .send()
            .map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Upload background image failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("Parse background image failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "Upload background image response missing id".to_string())
    }

    pub fn delete_background_image(&self, background_image_id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/background-images/{}", base, background_image_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("DELETE", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], None);
        let resp = self.client
            .delete(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .send()
            .map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Delete background image failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn create_base_custom_field_group(&self, project_id: &str, name: &str) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/projects/{}/base-custom-field-groups", base, project_id);
        let auth = self.auth_header();
        let body = json!({ "name": name });
        #[cfg(debug_assertions)]
        log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&body.to_string()));
        let resp = self.client.post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Create base custom field group failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse base custom field group failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str()).map(|s| s.to_string()).ok_or_else(|| "Response missing id".to_string())
    }

    pub fn update_base_custom_field_group(&self, id: &str, name: Option<&str>) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/base-custom-field-groups/{}", base, id);
        let auth = self.auth_header();
        let mut body = Map::new();
        if let Some(n) = name { body.insert("name".to_string(), Value::String(n.to_string())); }
        if body.is_empty() { return Ok(()); }
        #[cfg(debug_assertions)]
        { let preview = Value::Object(body.clone()); log_http_request("PATCH", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&preview.to_string())); }
        let resp = self.client.patch(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send().map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Update base custom field group failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn delete_base_custom_field_group(&self, id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/base-custom-field-groups/{}", base, id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("DELETE", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], None);
        let resp = self.client.delete(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .send().map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Delete base custom field group failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn duplicate_card(&self, card_id: &str, position: i64, name: &str) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/cards/{}/duplicate", base, card_id);
        let auth = self.auth_header();
        let body = json!({ "position": position, "name": name });
        #[cfg(debug_assertions)]
        log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&body.to_string()));
        let resp = self.client.post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Duplicate card failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse duplicate card failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str()).map(|s| s.to_string())
            .ok_or_else(|| "Duplicate card response missing id".to_string())
    }

    pub fn read_card_notifications(&self, card_id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/cards/{}/read-notifications", base, card_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)]
        log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some("{}"));
        let resp = self.client.post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json")
            .json(&json!({}))
            .send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status();
        let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)]
        log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Read card notifications failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn create_board_custom_field_group(&self, board_id: &str, position: i64, name: Option<&str>, base_custom_field_group_id: Option<&str>) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/boards/{}/custom-field-groups", base, board_id);
        let auth = self.auth_header();
        let mut body = Map::new();
        body.insert("position".to_string(), Value::from(position));
        if let Some(n) = name { body.insert("name".to_string(), Value::String(n.to_string())); }
        if let Some(bid) = base_custom_field_group_id { body.insert("baseCustomFieldGroupId".to_string(), Value::String(bid.to_string())); }
        #[cfg(debug_assertions)]
        { let preview = Value::Object(body.clone()); log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&preview.to_string())); }
        let resp = self.client.post(&url)
            .header("Authorization", auth).header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json").json(&body)
            .send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status(); let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Create board custom field group failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse create board cfg failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str()).map(|s| s.to_string()).ok_or_else(|| "Response missing id".to_string())
    }

    pub fn create_card_custom_field_group(&self, card_id: &str, position: i64, name: Option<&str>, base_custom_field_group_id: Option<&str>) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/cards/{}/custom-field-groups", base, card_id);
        let auth = self.auth_header();
        let mut body = Map::new();
        body.insert("position".to_string(), Value::from(position));
        if let Some(n) = name { body.insert("name".to_string(), Value::String(n.to_string())); }
        if let Some(bid) = base_custom_field_group_id { body.insert("baseCustomFieldGroupId".to_string(), Value::String(bid.to_string())); }
        #[cfg(debug_assertions)]
        { let preview = Value::Object(body.clone()); log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&preview.to_string())); }
        let resp = self.client.post(&url)
            .header("Authorization", auth).header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json").json(&body)
            .send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status(); let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Create card custom field group failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse create card cfg failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str()).map(|s| s.to_string()).ok_or_else(|| "Response missing id".to_string())
    }

    pub fn delete_custom_field_group(&self, id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/'); let url = format!("{}/api/custom-field-groups/{}", base, id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)] log_http_request("DELETE", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], None);
        let resp = self.client.delete(&url).header("Authorization", auth).header("Accept", "application/json")
            .send().map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status = resp.status(); let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Delete custom field group failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn get_custom_field_group(&self, id: &str) -> Result<PlankaCustomFieldGroupDetails, String> {
        let base = self.base_url.trim_end_matches('/'); let url = format!("{}/api/custom-field-groups/{}", base, id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)] log_http_request("GET", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], None);
        let resp = self.client.get(&url).header("Authorization", auth).header("Accept", "application/json")
            .send().map_err(|e| format!("GET {} failed: {}", url, e))?;
        let status = resp.status(); let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Get custom field group failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse cfg failed: {}", e))?;
        let item = v.get("item").and_then(|x| x.as_object()).ok_or("Missing item")?;
        let gid = item.get("id").and_then(|x| x.as_str()).unwrap_or(id).to_string();
        let name = item.get("name").and_then(|x| x.as_str()).map(|s| s.to_string());
        let mut fields: Vec<PlankaCustomField> = Vec::new();
        let mut values_by_field: HashMap<String, String> = HashMap::new();
        if let Some(inc) = v.get("included").and_then(|x| x.as_object()) {
            if let Some(arr) = inc.get("customFields").and_then(|x| x.as_array()) {
                for f in arr {
                    if let (Some(fid), Some(nm)) = (f.get("id").and_then(|x| x.as_str()), f.get("name").and_then(|x| x.as_str())) {
                        fields.push(PlankaCustomField { id: fid.to_string(), name: nm.to_string(), show_on_front_of_card: f.get("showOnFrontOfCard").and_then(|x| x.as_bool()) });
                    }
                }
            }
            if let Some(arr) = inc.get("customFieldValues").and_then(|x| x.as_array()) {
                for val in arr {
                    if let (Some(fid), Some(content)) = (val.get("customFieldId").and_then(|x| x.as_str()), val.get("content").and_then(|x| x.as_str())) {
                        values_by_field.insert(fid.to_string(), content.to_string());
                    }
                }
            }
        }
        Ok(PlankaCustomFieldGroupDetails { id: gid, name, fields, values_by_field })
    }

    pub fn update_custom_field_group(&self, id: &str, position: Option<i64>, name: Option<&str>) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/'); let url = format!("{}/api/custom-field-groups/{}", base, id);
        let auth = self.auth_header();
        let mut body = Map::new();
        if let Some(p) = position { body.insert("position".to_string(), Value::from(p)); }
        if let Some(n) = name { body.insert("name".to_string(), Value::String(n.to_string())); }
        if body.is_empty() { return Ok(()); }
        #[cfg(debug_assertions)] { let preview = Value::Object(body.clone()); log_http_request("PATCH", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&preview.to_string())); }
        let resp = self.client.patch(&url)
            .header("Authorization", auth).header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json").json(&body)
            .send().map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status = resp.status(); let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Update custom field group failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn update_custom_field_value(&self, card_id: &str, custom_field_group_id: &str, custom_field_id: &str, content: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/cards/{}/custom-field-values/customFieldGroupId:{}:customFieldId:{}", base, card_id, custom_field_group_id, custom_field_id);
        let auth = self.auth_header();
        let body = json!({ "content": content });
        #[cfg(debug_assertions)] log_http_request("PATCH", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&body.to_string()));
        let resp = self.client.patch(&url)
            .header("Authorization", auth).header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json").json(&body)
            .send().map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status = resp.status(); let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Update custom field value failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn delete_custom_field_value(&self, card_id: &str, custom_field_group_id: &str, custom_field_id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/cards/{}/custom-field-value/customFieldGroupId:{}:customFieldId:{}", base, card_id, custom_field_group_id, custom_field_id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)] log_http_request("DELETE", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], None);
        let resp = self.client.delete(&url)
            .header("Authorization", auth).header("Accept", "application/json")
            .send().map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status = resp.status(); let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Delete custom field value failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn create_custom_field_in_base_group(&self, base_custom_field_group_id: &str, position: i64, name: &str, show_on_front_of_card: Option<bool>) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/base-custom-field-groups/{}/custom-fields", base, base_custom_field_group_id);
        let auth = self.auth_header();
        let mut body = Map::new();
        body.insert("position".to_string(), Value::from(position));
        body.insert("name".to_string(), Value::String(name.to_string()));
        if let Some(s) = show_on_front_of_card { body.insert("showOnFrontOfCard".to_string(), Value::Bool(s)); }
        #[cfg(debug_assertions)] { let preview = Value::Object(body.clone()); log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&preview.to_string())); }
        let resp = self.client.post(&url)
            .header("Authorization", auth).header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json").json(&body)
            .send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status(); let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Create custom field (base group) failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse custom field failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str()).map(|s| s.to_string()).ok_or_else(|| "Response missing id".to_string())
    }

    pub fn create_custom_field_in_group(&self, custom_field_group_id: &str, position: i64, name: &str, show_on_front_of_card: Option<bool>) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/custom-field-groups/{}/custom-fields", base, custom_field_group_id);
        let auth = self.auth_header();
        let mut body = Map::new();
        body.insert("position".to_string(), Value::from(position));
        body.insert("name".to_string(), Value::String(name.to_string()));
        if let Some(s) = show_on_front_of_card { body.insert("showOnFrontOfCard".to_string(), Value::Bool(s)); }
        #[cfg(debug_assertions)] { let preview = Value::Object(body.clone()); log_http_request("POST", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&preview.to_string())); }
        let resp = self.client.post(&url)
            .header("Authorization", auth).header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json").json(&body)
            .send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status(); let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Create custom field failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse custom field failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str()).map(|s| s.to_string()).ok_or_else(|| "Response missing id".to_string())
    }

    pub fn delete_custom_field(&self, id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/'); let url = format!("{}/api/custom-fields/{}", base, id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)] log_http_request("DELETE", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json")], None);
        let resp = self.client.delete(&url).header("Authorization", auth).header("Accept", "application/json")
            .send().map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status = resp.status(); let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Delete custom field failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn update_custom_field(&self, id: &str, position: Option<i64>, name: Option<&str>, show_on_front_of_card: Option<bool>) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/'); let url = format!("{}/api/custom-fields/{}", base, id);
        let auth = self.auth_header();
        let mut body = Map::new();
        if let Some(p) = position { body.insert("position".to_string(), Value::from(p)); }
        if let Some(n) = name { body.insert("name".to_string(), Value::String(n.to_string())); }
        if let Some(s) = show_on_front_of_card { body.insert("showOnFrontOfCard".to_string(), Value::Bool(s)); }
        if body.is_empty() { return Ok(()); }
        #[cfg(debug_assertions)] { let preview = Value::Object(body.clone()); log_http_request("PATCH", &url, &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")], Some(&preview.to_string())); }
        let resp = self.client.patch(&url)
            .header("Authorization", auth).header("Accept", "application/json")
            .header(CONTENT_TYPE, "application/json").json(&body)
            .send().map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status = resp.status(); let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success() { return Err(format!("Update custom field failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn accept_terms(&self, pending_token: &str, signature: &str) -> Result<String, String> {
        let url = format!("{}/api/access-tokens/accept-terms", self.base_url.trim_end_matches('/'));
        let body = json!({ "pendingToken": pending_token, "signature": signature });
        #[cfg(debug_assertions)]
        log_http_request("POST", &url, &[("Accept","application/json"),("Content-Type","application/json")], Some(&body.to_string()));
        let resp = self.client.post(&url)
            .header("Accept","application/json").header(CONTENT_TYPE,"application/json")
            .json(&body).send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status = resp.status(); let text = resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(), &text);
        if !status.is_success(){ return Err(format!("Accept terms failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse accept-terms failed: {}", e))?;
        v.get("item").and_then(|x| x.as_str()).map(|s| s.to_string()).ok_or_else(|| "Response missing token".to_string())
    }

    pub fn logout_me(&self) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/'); let url = format!("{}/api/access-tokens/me", base);
        let auth = self.auth_header();
        #[cfg(debug_assertions)] log_http_request("DELETE",&url,&[("Authorization",auth.as_str()),("Accept","application/json")],None);
        let resp = self.client.delete(&url)
            .header("Authorization",auth).header("Accept","application/json")
            .send().map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Logout failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse logout failed: {}", e))?;
        v.get("item").and_then(|x| x.as_str()).map(|s| s.to_string()).ok_or_else(|| "Response missing item".to_string())
    }

    pub fn exchange_with_oidc(&self, code: &str, nonce: &str, with_http_only_token: Option<bool>) -> Result<String, String> {
        let url = format!("{}/api/access-tokens/exchange-with-oidc", self.base_url.trim_end_matches('/'));
        let mut body = Map::new();
        body.insert("code".into(), Value::String(code.into()));
        body.insert("nonce".into(), Value::String(nonce.into()));
        if let Some(v) = with_http_only_token { body.insert("withHttpOnlyToken".into(), Value::Bool(v)); }
        #[cfg(debug_assertions)] { let preview = Value::Object(body.clone());
            log_http_request("POST",&url,&[("Accept","application/json"),("Content-Type","application/json")],Some(&preview.to_string())); }
        let resp = self.client.post(&url)
            .header("Accept","application/json").header(CONTENT_TYPE,"application/json")
            .json(&body).send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("OIDC exchange failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse exchange failed: {}", e))?;
        v.get("item").and_then(|x| x.as_str()).map(|s| s.to_string()).ok_or_else(|| "Response missing token".to_string())
    }

    pub fn revoke_pending_token(&self, pending_token: &str) -> Result<(), String> {
        let url = format!("{}/api/access-tokens/revoke-pending-token", self.base_url.trim_end_matches('/'));
        let body = json!({ "pendingToken": pending_token });
        #[cfg(debug_assertions)]
        log_http_request("POST",&url,&[("Accept","application/json"),("Content-Type","application/json")],Some(&body.to_string()));
        let resp = self.client.post(&url)
            .header("Accept","application/json").header(CONTENT_TYPE,"application/json")
            .json(&body).send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Revoke pending token failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn fetch_server_config(&self) -> Result<PlankaServerConfig, String> {
        let url = format!("{}/api/config", self.base_url.trim_end_matches('/'));
        #[cfg(debug_assertions)] log_http_request("GET",&url,&[("Accept","application/json")],None);
        let resp = self.client.get(&url)
            .header("Accept","application/json").send()
            .map_err(|e| format!("GET {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Fetch config failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse config failed: {}", e))?;
        let item = v.get("item").ok_or("Missing item")?;
        let version = item.get("version").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let active = item.get("activeUsersLimit").and_then(|x| x.as_i64());
        let oidc = item.get("oidc").and_then(|x| x.as_object()).map(|o| PlankaOidcConfig{
            authorization_url: o.get("authorizationUrl").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
            end_session_url: o.get("endSessionUrl").and_then(|x| x.as_str()).map(|s| s.to_string()),
            is_enforced: o.get("isEnforced").and_then(|x| x.as_bool()).unwrap_or(false),
        });
        Ok(PlankaServerConfig{ version, active_users_limit: active, oidc })
    }

    pub fn create_board_notification_service(&self, board_id: &str, url_value: &str, format_value: &str) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/'); let url = format!("{}/api/boards/{}/notification-services", base, board_id);
        let auth = self.auth_header(); let body = json!({ "url": url_value, "format": format_value });
        #[cfg(debug_assertions)] log_http_request("POST",&url,&[("Authorization",auth.as_str()),("Accept","application/json"),("Content-Type","application/json")],Some(&body.to_string()));
        let resp = self.client.post(&url).header("Authorization",auth).header("Accept","application/json").header(CONTENT_TYPE,"application/json").json(&body).send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Create board notification service failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse notification service failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str()).map(|s| s.to_string()).ok_or_else(|| "Response missing id".to_string())
    }

    pub fn create_user_notification_service(&self, user_id: &str, url_value: &str, format_value: &str) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/'); let url = format!("{}/api/users/{}/notification-services", base, user_id);
        let auth = self.auth_header(); let body = json!({ "url": url_value, "format": format_value });
        #[cfg(debug_assertions)] log_http_request("POST",&url,&[("Authorization",auth.as_str()),("Accept","application/json"),("Content-Type","application/json")],Some(&body.to_string()));
        let resp = self.client.post(&url).header("Authorization",auth).header("Accept","application/json").header(CONTENT_TYPE,"application/json").json(&body).send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Create user notification service failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse notification service failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str()).map(|s| s.to_string()).ok_or_else(|| "Response missing id".to_string())
    }

    pub fn delete_notification_service(&self, id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/'); let url = format!("{}/api/notification-services/{}", base, id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)] log_http_request("DELETE",&url,&[("Authorization",auth.as_str()),("Accept","application/json")],None);
        let resp = self.client.delete(&url).header("Authorization",auth).header("Accept","application/json").send().map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Delete notification service failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn update_notification_service(&self, id: &str, url_value: Option<&str>, format_value: Option<&str>) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/'); let url = format!("{}/api/notification-services/{}", base, id);
        let auth = self.auth_header(); let mut body = Map::new();
        if let Some(u) = url_value { body.insert("url".into(), Value::String(u.into())); }
        if let Some(f) = format_value { body.insert("format".into(), Value::String(f.into())); }
        if body.is_empty(){ return Ok(()); }
        #[cfg(debug_assertions)] { let preview = Value::Object(body.clone());
            log_http_request("PATCH",&url,&[("Authorization",auth.as_str()),("Accept","application/json"),("Content-Type","application/json")],Some(&preview.to_string())); }
        let resp = self.client.patch(&url).header("Authorization",auth).header("Accept","application/json").header(CONTENT_TYPE,"application/json").json(&body).send().map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Update notification service failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn test_notification_service(&self, id: &str) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/'); let url = format!("{}/api/notification-services/{}/test", base, id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)] log_http_request("POST",&url,&[("Authorization",auth.as_str()),("Accept","application/json"),("Content-Type","application/json")],Some("{}"));
        let resp = self.client.post(&url).header("Authorization",auth).header("Accept","application/json").header(CONTENT_TYPE,"application/json").json(&json!({})).send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Test notification service failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn fetch_notifications(&self) -> Result<Vec<PlankaNotification>, String> {
        let base = self.base_url.trim_end_matches('/'); let url = format!("{}/api/notifications", base);
        let auth = self.auth_header();
        #[cfg(debug_assertions)] log_http_request("GET",&url,&[("Authorization",auth.as_str()),("Accept","application/json")],None);
        let resp = self.client.get(&url).header("Authorization",auth).header("Accept","application/json").send().map_err(|e| format!("GET {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Fetch notifications failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse notifications failed: {}", e))?;
        let mut out=Vec::new();
        if let Some(items)=v.get("items").and_then(|x| x.as_array()){
            for n in items {
                let id = n.get("id").and_then(|x| x.as_str()).unwrap_or_default().to_string();
                let user_id = n.get("userId").and_then(|x| x.as_str()).unwrap_or_default().to_string();
                let card_id = n.get("cardId").and_then(|x| x.as_str()).map(|s| s.to_string());
                let r#type = n.get("type").and_then(|x| x.as_str()).unwrap_or_default().to_string();
                let is_read = n.get("isRead").and_then(|x| x.as_bool()).unwrap_or(false);
                let created = n.get("createdAt").and_then(|x| x.as_str()).map(|s| s.to_string());
                let text = n.get("data").and_then(|d| d.get("text")).and_then(|x| x.as_str()).map(|s| s.to_string());
                out.push(PlankaNotification{ id, user_id, card_id, r#type, text, is_read, created });
            }
        }
        Ok(out)
    }

    pub fn read_all_notifications(&self) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/'); let url = format!("{}/api/notifications/read-all", base);
        let auth = self.auth_header();
        #[cfg(debug_assertions)] log_http_request("POST",&url,&[("Authorization",auth.as_str()),("Accept","application/json"),("Content-Type","application/json")],Some("{}"));
        let resp = self.client.post(&url).header("Authorization",auth).header("Accept","application/json").header(CONTENT_TYPE,"application/json").json(&json!({})).send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Read-all notifications failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn fetch_notification(&self, id: &str) -> Result<PlankaNotification, String> {
        let base = self.base_url.trim_end_matches('/'); let url = format!("{}/api/notifications/{}", base, id);
        let auth = self.auth_header();
        #[cfg(debug_assertions)] log_http_request("GET",&url,&[("Authorization",auth.as_str()),("Accept","application/json")],None);
        let resp = self.client.get(&url).header("Authorization",auth).header("Accept","application/json").send().map_err(|e| format!("GET {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Fetch notification failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse notification failed: {}", e))?;
        let n = v.get("item").ok_or("Missing item")?;
        let nid = n.get("id").and_then(|x| x.as_str()).unwrap_or(id).to_string();
        let user_id = n.get("userId").and_then(|x| x.as_str()).unwrap_or_default().to_string();
        let card_id = n.get("cardId").and_then(|x| x.as_str()).map(|s| s.to_string());
        let r#type = n.get("type").and_then(|x| x.as_str()).unwrap_or_default().to_string();
        let is_read = n.get("isRead").and_then(|x| x.as_bool()).unwrap_or(false);
        let created = n.get("createdAt").and_then(|x| x.as_str()).map(|s| s.to_string());
        let textf = n.get("data").and_then(|d| d.get("text")).and_then(|x| x.as_str()).map(|s| s.to_string());
        Ok(PlankaNotification{ id: nid, user_id, card_id, r#type, text: textf, is_read, created })
    }

    pub fn update_notification(&self, id: &str, is_read: bool) -> Result<(), String> {
        let base = self.base_url.trim_end_matches('/'); let url = format!("{}/api/notifications/{}", base, id);
        let auth = self.auth_header(); let body = json!({ "isRead": is_read });
        #[cfg(debug_assertions)] log_http_request("PATCH",&url,&[("Authorization",auth.as_str()),("Accept","application/json"),("Content-Type","application/json")],Some(&body.to_string()));
        let resp = self.client.patch(&url).header("Authorization",auth).header("Accept","application/json").header(CONTENT_TYPE,"application/json").json(&body).send().map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Update notification failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn fetch_terms(&self, r#type: &str, language: Option<&str>) -> Result<PlankaTerms, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = if let Some(lang) = language {
            format!("{}/api/terms/{}?language={}", base, r#type, lang)
        } else {
            format!("{}/api/terms/{}", base, r#type)
        };
        #[cfg(debug_assertions)] log_http_request("GET",&url,&[("Accept","application/json")],None);
        let resp = self.client.get(&url).header("Accept","application/json").send().map_err(|e| format!("GET {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Fetch terms failed: HTTP {} - {}", status, text)); }
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("parse terms failed: {}", e))?;
        let item = v.get("item").ok_or("Missing item")?;
        Ok(PlankaTerms{
            r#type: item.get("type").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            language: item.get("language").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            content: item.get("content").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            signature: item.get("signature").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        })
    }

    pub fn create_user(&self, email: &str, password: &str, role: &str, name: &str, username: Option<&str>) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/'); let url = format!("{}/api/users", base);
        let auth = self.auth_header(); let mut body = Map::new();
        body.insert("email".into(), Value::String(email.into()));
        body.insert("password".into(), Value::String(password.into()));
        body.insert("role".into(), Value::String(role.into()));
        body.insert("name".into(), Value::String(name.into()));
        if let Some(u)=username { body.insert("username".into(), Value::String(u.into())); }
        #[cfg(debug_assertions)] { let preview=Value::Object(body.clone()); log_http_request("POST",&url,&[("Authorization",auth.as_str()),("Accept","application/json"),("Content-Type","application/json")],Some(&preview.to_string())); }
        let resp = self.client.post(&url).header("Authorization",auth).header("Accept","application/json").header(CONTENT_TYPE,"application/json").json(&body).send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Create user failed: HTTP {} - {}", status, text)); }
        let v: Value=serde_json::from_str(&text).map_err(|e| format!("parse create user failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str()).map(|s| s.to_string()).ok_or_else(|| "Response missing id".to_string())
    }

    pub fn fetch_users(&self) -> Result<Vec<PlankaUser>, String> {
        let base = self.base_url.trim_end_matches('/'); let url = format!("{}/api/users", base);
        let auth = self.auth_header();
        #[cfg(debug_assertions)] log_http_request("GET",&url,&[("Authorization",auth.as_str()),("Accept","application/json")],None);
        let resp = self.client.get(&url).header("Authorization",auth).header("Accept","application/json").send().map_err(|e| format!("GET {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Fetch users failed: HTTP {} - {}", status, text)); }
        let v: Value=serde_json::from_str(&text).map_err(|e| format!("parse users failed: {}", e))?;
        let mut out=Vec::new();
        if let Some(items)=v.get("items").and_then(|x| x.as_array()){
            for u in items {
                out.push(PlankaUser{
                    id: u.get("id").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                    role: u.get("role").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                    name: u.get("name").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                    username: u.get("username").and_then(|x| x.as_str()).map(|s| s.to_string()),
                    email: u.get("email").and_then(|x| x.as_str()).map(|s| s.to_string()),
                    is_deactivated: u.get("isDeactivated").and_then(|x| x.as_bool()).unwrap_or(false),
                });
            }
        }
        Ok(out)
    }

    pub fn delete_user(&self, id: &str) -> Result<(), String> {
        let base=self.base_url.trim_end_matches('/'); let url=format!("{}/api/users/{}", base, id);
        let auth=self.auth_header();
        #[cfg(debug_assertions)] log_http_request("DELETE",&url,&[("Authorization",auth.as_str()),("Accept","application/json")],None);
        let resp=self.client.delete(&url).header("Authorization",auth).header("Accept","application/json").send().map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Delete user failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn fetch_user(&self, id: &str) -> Result<PlankaUser, String> {
        let base=self.base_url.trim_end_matches('/'); let url=format!("{}/api/users/{}", base, id);
        let auth=self.auth_header();
        #[cfg(debug_assertions)] log_http_request("GET",&url,&[("Authorization",auth.as_str()),("Accept","application/json")],None);
        let resp=self.client.get(&url).header("Authorization",auth).header("Accept","application/json").send().map_err(|e| format!("GET {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Fetch user failed: HTTP {} - {}", status, text)); }
        let v: Value=serde_json::from_str(&text).map_err(|e| format!("parse user failed: {}", e))?;
        let u=v.get("item").ok_or("Missing item")?;
        Ok(PlankaUser{
            id: u.get("id").and_then(|x| x.as_str()).unwrap_or(id).to_string(),
            role: u.get("role").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
            name: u.get("name").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
            username: u.get("username").and_then(|x| x.as_str()).map(|s| s.to_string()),
            email: u.get("email").and_then(|x| x.as_str()).map(|s| s.to_string()),
            is_deactivated: u.get("isDeactivated").and_then(|x| x.as_bool()).unwrap_or(false),
        })
    }

    pub fn update_user(&self, id: &str, role: Option<&str>, name: Option<&str>, is_deactivated: Option<bool>) -> Result<(), String> {
        let base=self.base_url.trim_end_matches('/'); let url=format!("{}/api/users/{}", base, id);
        let auth=self.auth_header(); let mut body=Map::new();
        if let Some(r)=role { body.insert("role".into(), Value::String(r.into())); }
        if let Some(n)=name { body.insert("name".into(), Value::String(n.into())); }
        if let Some(d)=is_deactivated { body.insert("isDeactivated".into(), Value::Bool(d)); }
        if body.is_empty(){ return Ok(()); }
        #[cfg(debug_assertions)] { let preview=Value::Object(body.clone());
            log_http_request("PATCH",&url,&[("Authorization",auth.as_str()),("Accept","application/json"),("Content-Type","application/json")],Some(&preview.to_string())); }
        let resp=self.client.patch(&url).header("Authorization",auth).header("Accept","application/json").header(CONTENT_TYPE,"application/json").json(&body).send().map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Update user failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn update_user_avatar(&self, id: &str, file_path: &str) -> Result<(), String> {
        let base=self.base_url.trim_end_matches('/'); let url=format!("{}/api/users/{}/avatar", base, id);
        let auth=self.auth_header();
        let form = Form::new().file("file", file_path).map_err(|e| format!("Read file failed: {}", e))?;
        #[cfg(debug_assertions)] log_http_request("POST",&url,&[("Authorization",auth.as_str()),("Accept","application/json")],Some("[multipart form]"));
        let resp=self.client.post(&url).header("Authorization",auth).header("Accept","application/json").multipart(form).send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Update avatar failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn update_user_email(&self, id: &str, email: &str, current_password: Option<&str>) -> Result<(), String> {
        let base=self.base_url.trim_end_matches('/'); let url=format!("{}/api/users/{}/email", base, id);
        let auth=self.auth_header(); let mut body=Map::new();
        body.insert("email".into(), Value::String(email.into()));
        if let Some(p)=current_password { body.insert("currentPassword".into(), Value::String(p.into())); }
        #[cfg(debug_assertions)] { let preview=Value::Object(body.clone());
            log_http_request("PATCH",&url,&[("Authorization",auth.as_str()),("Accept","application/json"),("Content-Type","application/json")],Some(&preview.to_string())); }
        let resp=self.client.patch(&url).header("Authorization",auth).header("Accept","application/json").header(CONTENT_TYPE,"application/json").json(&body).send().map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Update user email failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn update_user_password(&self, id: &str, password: &str, current_password: Option<&str>) -> Result<(), String> {
        let base=self.base_url.trim_end_matches('/'); let url=format!("{}/api/users/{}/password", base, id);
        let auth=self.auth_header(); let mut body=Map::new();
        body.insert("password".into(), Value::String(password.into()));
        if let Some(p)=current_password { body.insert("currentPassword".into(), Value::String(p.into())); }
        #[cfg(debug_assertions)] { let preview=Value::Object(body.clone());
            log_http_request("PATCH",&url,&[("Authorization",auth.as_str()),("Accept","application/json"),("Content-Type","application/json")],Some(&preview.to_string())); }
        let resp=self.client.patch(&url).header("Authorization",auth).header("Accept","application/json").header(CONTENT_TYPE,"application/json").json(&body).send().map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Update user password failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn update_user_username(&self, id: &str, username: Option<&str>, current_password: Option<&str>) -> Result<(), String> {
        let base=self.base_url.trim_end_matches('/'); let url=format!("{}/api/users/{}/username", base, id);
        let auth=self.auth_header(); let mut body=Map::new();
        if let Some(u)=username { body.insert("username".into(), Value::String(u.into())); }
        if let Some(p)=current_password { body.insert("currentPassword".into(), Value::String(p.into())); }
        #[cfg(debug_assertions)] { let preview=Value::Object(body.clone());
            log_http_request("PATCH",&url,&[("Authorization",auth.as_str()),("Accept","application/json"),("Content-Type","application/json")],Some(&preview.to_string())); }
        let resp=self.client.patch(&url).header("Authorization",auth).header("Accept","application/json").header(CONTENT_TYPE,"application/json").json(&body).send().map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Update user username failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn create_webhook(&self, name: &str, url_value: &str, access_token: Option<&str>, events: Option<&str>, excluded_events: Option<&str>) -> Result<String, String> {
        let base=self.base_url.trim_end_matches('/'); let url=format!("{}/api/webhooks", base);
        let auth=self.auth_header(); let mut body=Map::new();
        body.insert("name".into(), Value::String(name.into()));
        body.insert("url".into(), Value::String(url_value.into()));
        if let Some(t)=access_token { body.insert("accessToken".into(), Value::String(t.into())); }
        if let Some(e)=events { body.insert("events".into(), Value::String(e.into())); }
        if let Some(x)=excluded_events { body.insert("excludedEvents".into(), Value::String(x.into())); }
        #[cfg(debug_assertions)] { let preview=Value::Object(body.clone());
            log_http_request("POST",&url,&[("Authorization",auth.as_str()),("Accept","application/json"),("Content-Type","application/json")],Some(&preview.to_string())); }
        let resp=self.client.post(&url).header("Authorization",auth).header("Accept","application/json").header(CONTENT_TYPE,"application/json").json(&body).send().map_err(|e| format!("POST {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Create webhook failed: HTTP {} - {}", status, text)); }
        let v: Value=serde_json::from_str(&text).map_err(|e| format!("parse create webhook failed: {}", e))?;
        v.get("item").and_then(|i| i.get("id")).and_then(|x| x.as_str()).map(|s| s.to_string()).ok_or_else(|| "Response missing id".to_string())
    }

    pub fn fetch_webhooks(&self) -> Result<Vec<PlankaWebhook>, String> {
        let base=self.base_url.trim_end_matches('/'); let url=format!("{}/api/webhooks", base);
        let auth=self.auth_header();
        #[cfg(debug_assertions)] log_http_request("GET",&url,&[("Authorization",auth.as_str()),("Accept","application/json")],None);
        let resp=self.client.get(&url).header("Authorization",auth).header("Accept","application/json").send().map_err(|e| format!("GET {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Fetch webhooks failed: HTTP {} - {}", status, text)); }
        let v: Value=serde_json::from_str(&text).map_err(|e| format!("parse webhooks failed: {}", e))?;
        let mut out=Vec::new();
        if let Some(items)=v.get("items").and_then(|x| x.as_array()){
            for w in items {
                out.push(PlankaWebhook{
                    id: w.get("id").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                    name: w.get("name").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                    url: w.get("url").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                    access_token: w.get("accessToken").and_then(|x| x.as_str()).map(|s| s.to_string()),
                    events: w.get("events").and_then(|x| x.as_str()).map(|s| s.to_string()),
                    excluded_events: w.get("excludedEvents").and_then(|x| x.as_str()).map(|s| s.to_string()),
                });
            }
        }
        Ok(out)
    }

    pub fn delete_webhook(&self, id: &str) -> Result<(), String> {
        let base=self.base_url.trim_end_matches('/'); let url=format!("{}/api/webhooks/{}", base, id);
        let auth=self.auth_header();
        #[cfg(debug_assertions)] log_http_request("DELETE",&url,&[("Authorization",auth.as_str()),("Accept","application/json")],None);
        let resp=self.client.delete(&url).header("Authorization",auth).header("Accept","application/json").send().map_err(|e| format!("DELETE {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Delete webhook failed: HTTP {} - {}", status, text)); }
        Ok(())
    }

    pub fn update_webhook(&self, id: &str, name: Option<&str>, url_value: Option<&str>, access_token: Option<&str>, events: Option<&str>, excluded_events: Option<&str>) -> Result<(), String> {
        let base=self.base_url.trim_end_matches('/'); let url=format!("{}/api/webhooks/{}", base, id);
        let auth=self.auth_header(); let mut body=Map::new();
        if let Some(v)=name { body.insert("name".into(), Value::String(v.into())); }
        if let Some(v)=url_value { body.insert("url".into(), Value::String(v.into())); }
        if let Some(v)=access_token { body.insert("accessToken".into(), Value::String(v.into())); }
        if let Some(v)=events { body.insert("events".into(), Value::String(v.into())); }
        if let Some(v)=excluded_events { body.insert("excludedEvents".into(), Value::String(v.into())); }
        if body.is_empty(){ return Ok(()); }
        #[cfg(debug_assertions)] { let preview=Value::Object(body.clone());
            log_http_request("PATCH",&url,&[("Authorization",auth.as_str()),("Accept","application/json"),("Content-Type","application/json")],Some(&preview.to_string())); }
        let resp=self.client.patch(&url).header("Authorization",auth).header("Accept","application/json").header(CONTENT_TYPE,"application/json").json(&body).send().map_err(|e| format!("PATCH {} failed: {}", url, e))?;
        let status=resp.status(); let text=resp.text().unwrap_or_default();
        #[cfg(debug_assertions)] log_http_response(status.as_u16(),&text);
        if !status.is_success(){ return Err(format!("Update webhook failed: HTTP {} - {}", status, text)); }
        Ok(())
    }
}

// POST /api/access-tokens
fn login(server_url: &str, email_or_username: &str, password: &str) -> Result<String, String> {
    #[cfg(debug_assertions)]
    init_log_notice();
    #[derive(Serialize)]
    struct LoginReq<'a> {
        #[serde(rename = "emailOrUsername")]
        email_or_username: &'a str,
        password: &'a str,
        #[serde(rename = "withHttpOnlyToken")]
        with_http_only_token: bool,
    }
    #[derive(Deserialize)]
    struct LoginRes {
        item: String,
    }

    let url = format!("{}/api/access-tokens", server_url.trim_end_matches('/'));
    let client = Client::new();
    // Debug: log outgoing request (mask password)
    #[cfg(debug_assertions)]
    {
        let preview = json!({
            "emailOrUsername": email_or_username,
            "password": "***",
            "withHttpOnlyToken": false
        }).to_string();
        log_http_request(
            "POST",
            &url,
            &[("Accept", "application/json"), ("Content-Type", "application/json")],
            Some(&preview),
        );
    }
    let res = client
        .post(&url)
        .header("Accept", "application/json")
        .header(CONTENT_TYPE, "application/json")
        .json(&LoginReq {
            email_or_username,
            password,
            with_http_only_token: false,
        })
        .send()
        .map_err(|e| format!("Login request failed: {}", e))?;
    let status = res.status();
    let text = res.text().map_err(|e| format!("Login read failed: {}", e))?;
    #[cfg(debug_assertions)]
    log_http_response(status.as_u16(), &text);
    if !status.is_success() {
        return Err(format!("Login failed: HTTP {} - {}", status, text));
    }
    let body: LoginRes =
        serde_json::from_str(&text).map_err(|e| format!("Login parse failed: {}", e))?;
    #[cfg(debug_assertions)]
    log_debug("Login succeeded and token parsed");
    Ok(body.item)
}

#[derive(Clone, Debug)]
pub struct PlankaBoard {
    pub id: String,
    pub name: String,
    pub project_id: Option<String>,
    pub project_name: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PlankaLists {
    pub board_id: String,
    pub todo_list_id: String,
    pub doing_list_id: String,
    pub done_list_id: String,
}

#[derive(Clone, Debug)]
pub struct PlankaCard {
    pub id: String,
    pub name: String,
    pub due: Option<String>,
    pub created: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PlankaComment {
    pub id: String,
    pub user_id: Option<String>,
    pub user_name: Option<String>,
    pub text: String,
    pub created: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PlankaAttachment {
    pub id: String,
    pub name: String,
    pub url: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PlankaTask {
    pub id: String,
    pub name: String,
    pub is_completed: bool,
    pub task_list_id: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PlankaCustomField {
    pub id: String,
    pub name: String,
    pub show_on_front_of_card: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct PlankaCustomFieldGroupDetails {
    pub id: String,
    pub name: Option<String>,
    pub fields: Vec<PlankaCustomField>,
    pub values_by_field: std::collections::HashMap<String, String>, // fieldId -> content
}

#[derive(Clone, Debug)]
pub struct PlankaCardDetails {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub due: Option<String>,
    pub is_due_completed: Option<bool>,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub list_name: Option<String>,
    pub labels: Vec<String>,
    pub attachments: Vec<String>,
    pub tasks: Vec<(String, bool)>, // (name, isCompleted)
    pub board_id: Option<String>,
    pub attachments_full: Vec<PlankaAttachment>,
    pub tasks_full: Vec<PlankaTask>,
    pub task_lists: Vec<(String, String)>, // (id, name)
    pub custom_field_groups: Vec<PlankaCustomFieldGroupDetails>,
}

#[derive(Clone, Debug)]
pub struct PlankaAction {
    pub id: String,
    pub type_: String,
    pub user_id: Option<String>,
    pub card_id: Option<String>,
    pub data: Option<Value>,
    pub created: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PlankaProject {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub is_hidden: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct PlankaProjectDetails {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub is_hidden: Option<bool>,
    pub boards: Vec<PlankaBoard>,
}

#[derive(Clone, Debug)]
pub struct PlankaTaskListDetails {
    pub id: String,
    pub name: String,
    pub tasks: Vec<PlankaTask>,
}

#[derive(Clone, Debug)]
pub struct PlankaBoardDetails {
    pub id: String,
    pub name: String,
    pub project_id: Option<String>,
    pub lists: Vec<(String, String)>,   // (id, name)
    pub labels: Vec<(String, String, String)>, // (id, name, color)
}

#[derive(Clone, Debug)]
pub struct PlankaListDetails {
    pub id: String,
    pub name: String,
    pub cards: Vec<PlankaCard>,
}

#[derive(Clone, Debug)]
pub struct PlankaServerConfig {
    pub version: String,
    pub active_users_limit: Option<i64>,
    pub oidc: Option<PlankaOidcConfig>,
}

#[derive(Clone, Debug)]
pub struct PlankaOidcConfig {
    pub authorization_url: String,
    pub end_session_url: Option<String>,
    pub is_enforced: bool,
}

#[derive(Clone, Debug)]
pub struct PlankaNotification {
    pub id: String,
    pub user_id: String,
    pub card_id: Option<String>,
    pub r#type: String,
    pub text: Option<String>,
    pub is_read: bool,
    pub created: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PlankaNotificationService {
    pub id: String,
    pub user_id: Option<String>,
    pub board_id: Option<String>,
    pub url: String,
    pub format: String,
}

#[derive(Clone, Debug)]
pub struct PlankaUser {
    pub id: String,
    pub role: String,
    pub name: String,
    pub username: Option<String>,
    pub email: Option<String>,
    pub is_deactivated: bool,
}

#[derive(Clone, Debug)]
pub struct PlankaWebhook {
    pub id: String,
    pub name: String,
    pub url: String,
    pub access_token: Option<String>,
    pub events: Option<String>,
    pub excluded_events: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PlankaTerms {
    pub r#type: String,
    pub language: String,
    pub content: String,
    pub signature: String,
}
