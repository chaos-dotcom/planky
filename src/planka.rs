use directories::ProjectDirs;
use reqwest::blocking::Client;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use std::fs::{create_dir_all, File};
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;

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

    if !res.status().is_success() {
        return Err(format!("Login failed: HTTP {}", res.status()));
    }
    let body: LoginRes = res.json().map_err(|e| format!("Login parse failed: {}", e))?;
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
