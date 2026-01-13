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

        Ok(PlankaCardDetails {
            id, name, description, due, is_due_completed, created, updated, list_name, labels, attachments, tasks,
            board_id, attachments_full, tasks_full, task_lists,
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
