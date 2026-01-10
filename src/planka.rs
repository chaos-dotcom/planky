use directories::ProjectDirs;
use reqwest::blocking::Client;
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};
#[cfg(debug_assertions)]
use serde_json::json;
use std::fs::{create_dir_all, File};
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;

#[cfg(debug_assertions)]
fn log_http_request(method: &str, url: &str, headers: &[(&str, &str)], body: Option<&str>) {
    eprintln!("[HTTP OUT] {} {}", method, url);
    for (k, v) in headers {
        let shown = if k.eq_ignore_ascii_case("authorization") {
            mask_bearer(v)
        } else {
            (*v).to_string()
        };
        eprintln!("  {}: {}", k, shown);
    }
    if let Some(b) = body {
        eprintln!("  Body: {}", truncate(b, 4000));
    }
}

#[cfg(debug_assertions)]
fn log_http_response(status: u16, body: &str) {
    eprintln!("[HTTP IN] Status: {}", status);
    eprintln!("  Body: {}", truncate(body, 4000));
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

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct PlankaConfig {
    pub server_url: String,
    pub email_or_username: String,
    pub password: String,
    pub token: Option<String>,
}

pub fn config_path() -> PathBuf {
    let proj = ProjectDirs::from("com", "KushalMeghani", "RustyTodos").expect("proj dirs");
    let dir = proj.config_dir();
    create_dir_all(dir).ok();
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
        if cfg.server_url.trim().is_empty() {
            return Err("Planka server URL is empty".into());
        }
        if cfg.token.is_none() {
            let token = login(&cfg.server_url, &cfg.email_or_username, &cfg.password)?;
            cfg.token = Some(token);
            let _ = save_config(&cfg);
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

    // TODO: Fill with actual Planka endpoints.
    pub fn resolve_lists(&self, _board_name: &str) -> Result<PlankaLists, String> {
        Err("Planka list resolution not yet implemented (fill endpoints)".into())
    }

    pub fn create_card(&self, _list_id: &str, _name: &str, _due: Option<&str>) -> Result<String, String> {
        Err("Planka create_card not yet implemented (fill endpoint)".into())
    }

    pub fn move_card(&self, _card_id: &str, _to_list_id: &str) -> Result<(), String> {
        Err("Planka move_card not yet implemented (fill endpoint)".into())
    }

    pub fn fetch_cards(&self, _list_id: &str) -> Result<Vec<PlankaCard>, String> {
        Ok(vec![])
    }
}

// POST /api/access-tokens
fn login(server_url: &str, email_or_username: &str, password: &str) -> Result<String, String> {
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
            &[("Content-Type", "application/json")],
            Some(&preview),
        );
    }
    let res = client
        .post(&url)
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
    Ok(body.item)
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
