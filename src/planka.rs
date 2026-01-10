use reqwest::blocking::Client;
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value, Map};
use std::fs::{create_dir_all, File, OpenOptions};
use std::io::{BufReader, BufWriter, Write as IoWrite};
use std::path::PathBuf;

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
    let dir = base.join("RustyTodos");
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
    let dir = base.join("RustyTodos");
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
        let projects_url = format!("{}/api/projects", base);
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
        let mut project_ids: Vec<String> = Vec::new();
        if let Some(arr) = v.as_array() {
            for p in arr {
                if let Some(id) = p.get("id").and_then(|x| x.as_str()) {
                    project_ids.push(id.to_string());
                }
            }
        } else if let Some(items) = v.get("items").and_then(|x| x.as_array()) {
            for p in items {
                if let Some(id) = p.get("id").and_then(|x| x.as_str()) {
                    project_ids.push(id.to_string());
                }
            }
        } else if let Some(projects) = v.get("projects").and_then(|x| x.as_array()) {
            for p in projects {
                if let Some(id) = p.get("id").and_then(|x| x.as_str()) {
                    project_ids.push(id.to_string());
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
                    boards.push(PlankaBoard { id: id.to_string(), name: name.to_string(), project_id });
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
                        boards.push(PlankaBoard {
                            id: id.to_string(),
                            name: name.to_string(),
                            project_id,
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
                        boards.push(PlankaBoard {
                            id: id.to_string(),
                            name: name.to_string(),
                            project_id,
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

    pub fn create_card(&self, list_id: &str, name: &str, due: Option<&str>) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/api/cards", base);
        let auth = self.auth_header();
        // Build body
        let mut body = Map::new();
        body.insert("listId".to_string(), Value::String(list_id.to_string()));
        body.insert("name".to_string(), Value::String(name.to_string()));
        if let Some(d) = due {
            body.insert("dueDate".to_string(), Value::String(d.to_string()));
        }
        #[cfg(debug_assertions)]
        {
            let preview = Value::Object(body.clone());
            log_http_request(
                "POST",
                &url,
                &[("Authorization", auth.as_str()), ("Accept", "application/json"), ("Content-Type", "application/json")],
                Some(&preview.to_string()),
            );
        }
        let resp = self.client
            .post(&url)
            .header("Authorization", auth)
            .header("Accept", "application/json")
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
        let body = json!({ "listId": to_list_id });
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

    pub fn fetch_cards(&self, _list_id: &str) -> Result<Vec<PlankaCard>, String> {
        #[cfg(debug_assertions)]
        log_debug(&format!("fetch_cards(list_id={}) called (stub returns empty)", _list_id));
        Ok(vec![])
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
}
