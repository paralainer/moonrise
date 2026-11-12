//! A structured verdict from an LLM: one call, a JSON schema, a JSON answer. Used only by
//! identification, for the files the deterministic steps can't settle (Appendix A of the plan).
//!
//! **Providers** ([`PRESETS`], `MOONRISE_LLM_PROVIDER` / the setting `llm.provider`): each preset
//! gives an API style, a base URL, a default model, the answer format that works with it and
//! where its key comes from. The base URL, model and format can be changed for any of them.
//! - `anthropic` (default): the Messages API (`POST {base}/v1/messages`, `x-api-key`,
//!   `anthropic-version`). Default model `claude-haiku-5-5`.
//! - `openai`, `gemini` (Google's OpenAI-compatible endpoint), `openrouter`, `ollama`, `lmstudio`,
//!   `llamacpp`, `custom` (any OpenAI-compatible server: vLLM, LocalAI, a proxy…): Chat
//!   Completions (`POST {base}/chat/completions`, `Authorization: Bearer` when there is a key).
//! - `chatgpt`: the user's ChatGPT plan through "Sign in with ChatGPT" (`chatgpt.rs`): the
//!   Responses API (`POST {base}/responses`, streamed, `store: false`) with the signed-in
//!   account's access token.
//!
//! **Answer format** (`MOONRISE_LLM_OUTPUT`):
//! - `schema`: Anthropic `output_config.format` json_schema; OpenAI `response_format:
//!   json_schema`; Responses `text.format` json_schema.
//! - `tool`: one strict tool and an instruction to call it; `tool_choice` `auto` (Anthropic:
//!   forced choice is refused by some newer models) or `required` (OpenAI, Responses).
//! - `prompt`: JSON asked for in the prompt, parsed leniently (servers without either).
//! - `auto`: the first call tries `schema`, then `tool`, then `prompt`, moving on when the server
//!   rejects the request (HTTP 400/422) or the answer isn't the JSON asked for; the format that
//!   worked is kept for this configuration (until the settings change or the server restarts).
//!
//! No temperature/top_p/top_k is ever sent (API version 2026-10-21 rejects them). A refusal,
//! `max_tokens`, or a reply that isn't the JSON asked for is "no verdict", never a guess.
//! 429/5xx are retried with backoff, at most 3 tries. Keys and tokens are never logged, and are
//! cut out of error messages (a server that echoes a request back can't leak them).

pub mod admin;
pub mod chatgpt;
pub mod keys;

use std::{sync::Mutex, time::Duration};

use anyhow::Result;
use serde_json::{json, Value};

use crate::admin::config::LlmConfig;

/// Where a preset's key comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum KeyNeed {
    /// An API key is needed (at the preset's own address).
    Required,
    /// A key is sent when one is set.
    Optional,
    /// Signed in with an account (ChatGPT).
    SignIn,
}

/// A provider: what to fill in when it is chosen.
#[derive(Debug)]
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    /// `anthropic`, `openai` (Chat Completions) or `responses`.
    pub style: &'static str,
    pub base_url: &'static str,
    /// The default model ("": the admin picks one; llama.cpp serves the one it loaded).
    pub model: &'static str,
    /// The answer format that works with it.
    pub output: &'static str,
    pub key: KeyNeed,
    /// The environment variable read for the key by default (`MOONRISE_LLM_API_KEY_ENV`).
    pub key_env: &'static str,
    /// Where to get a key, or what to run.
    pub help: &'static str,
    /// Runs on the admin's own machine (nothing leaves the network).
    pub local: bool,
}

pub const PRESETS: &[Preset] = &[
    Preset {
        id: "anthropic",
        name: "Anthropic",
        style: "anthropic",
        base_url: "https://api.anthropic.com",
        model: "claude-haiku-5-5",
        output: "schema",
        key: KeyNeed::Required,
        key_env: "ANTHROPIC_API_KEY",
        help: "An API key from console.anthropic.com.",
        local: false,
    },
    Preset {
        id: "openai",
        name: "OpenAI",
        style: "openai",
        base_url: "https://api.openai.com/v1",
        model: "gpt-5.4-mini",
        output: "schema",
        key: KeyNeed::Required,
        key_env: "OPENAI_API_KEY",
        help: "An API key from platform.openai.com (billed by the API, not a ChatGPT plan).",
        local: false,
    },
    Preset {
        id: "chatgpt",
        name: "ChatGPT plan",
        style: "responses",
        base_url: "https://api.openai.com/v1",
        model: "",
        output: "auto",
        key: KeyNeed::SignIn,
        key_env: "",
        help: "Your ChatGPT plan (Plus, Pro and others that allow it) instead of an API key: sign in below.",
        local: false,
    },
    Preset {
        id: "gemini",
        name: "Google Gemini",
        style: "openai",
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        model: "gemini-2.5-flash",
        output: "auto",
        key: KeyNeed::Required,
        key_env: "GEMINI_API_KEY",
        help: "An API key from aistudio.google.com.",
        local: false,
    },
    Preset {
        id: "openrouter",
        name: "OpenRouter",
        style: "openai",
        base_url: "https://openrouter.ai/api/v1",
        model: "google/gemini-2.5-flash",
        output: "auto",
        key: KeyNeed::Required,
        key_env: "OPENROUTER_API_KEY",
        help: "An API key from openrouter.ai; the model is any OpenRouter model id.",
        local: false,
    },
    Preset {
        id: "ollama",
        name: "Ollama",
        style: "openai",
        base_url: "http://localhost:11434/v1",
        model: "",
        output: "auto",
        key: KeyNeed::Optional,
        key_env: "",
        help: "Ollama on this network: its address with /v1, and a model it has pulled.",
        local: true,
    },
    Preset {
        id: "lmstudio",
        name: "LM Studio",
        style: "openai",
        base_url: "http://localhost:1234/v1",
        model: "",
        output: "auto",
        key: KeyNeed::Optional,
        key_env: "",
        help: "LM Studio's server on this network: its address with /v1, and a loaded model.",
        local: true,
    },
    Preset {
        id: "llamacpp",
        name: "llama.cpp",
        style: "openai",
        base_url: "http://localhost:8080/v1",
        model: "",
        output: "auto",
        key: KeyNeed::Optional,
        key_env: "",
        help: "llama-server on this network: its address with /v1 (start it with --jinja for tool calls).",
        local: true,
    },
    Preset {
        id: "custom",
        name: "Other (OpenAI-compatible)",
        style: "openai",
        base_url: "",
        model: "",
        output: "auto",
        key: KeyNeed::Optional,
        key_env: "",
        help: "Any server with the OpenAI Chat Completions API (vLLM, LocalAI, a gateway…): its base URL (usually ending in /v1), a model, and a key if it needs one.",
        local: false,
    },
];

pub const OUTPUTS: &[&str] = &["auto", "schema", "tool", "prompt"];

pub fn preset(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}

/// The preset a provider name means (unknown names: any OpenAI-compatible server).
pub fn preset_or_custom(id: &str) -> &'static Preset {
    preset(id).unwrap_or_else(|| preset("custom").unwrap())
}

/// The presets as the admin API answers them.
pub fn presets_json() -> Value {
    json!(PRESETS
        .iter()
        .map(|p| json!({
            "id": p.id, "name": p.name, "style": p.style, "base_url": p.base_url, "model": p.model, "output": p.output,
            "key": match p.key { KeyNeed::Required => "required", KeyNeed::Optional => "optional", KeyNeed::SignIn => "sign_in" },
            "help": p.help, "local": p.local,
        }))
        .collect::<Vec<_>>())
}

/// `scheme://host[:port]` of a URL, lower-cased ("" when it has none).
pub fn origin(url: &str) -> String {
    let u = url.trim().to_ascii_lowercase();
    let Some((scheme, rest)) = u.split_once("://") else {
        return String::new();
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    format!("{scheme}://{host}")
}

/// Whether a model reasons before answering (OpenAI and Gemini take `reasoning_effort`).
fn reasoning_model(provider: &str, model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    match provider {
        "openai" => m.starts_with("gpt-5") || m.starts_with("gpt-6") || m.starts_with("o1") || m.starts_with("o3") || m.starts_with("o4"),
        "gemini" => m.starts_with("gemini-2.5") || m.starts_with("gemini-3"),
        _ => false,
    }
}

pub struct Llm {
    pub config: LlmConfig,
    http: reqwest::Client,
    /// `auto`: the format that worked.
    resolved: Mutex<Option<String>>,
}

#[derive(Debug, Clone)]
pub struct Verdict {
    pub value: Value,
    pub input_tokens: i64,
    pub output_tokens: i64,
    /// The answer format used.
    pub output: String,
}

/// Why there was no verdict (the call itself worked).
#[derive(Debug)]
pub struct NoVerdict(pub String);

impl std::fmt::Display for NoVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "no verdict: {}", self.0)
    }
}

impl std::error::Error for NoVerdict {}

/// The server refused the request (not retried).
#[derive(Debug)]
pub struct HttpError {
    pub status: u16,
    pub body: String,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "llm HTTP {}: {}", self.status, self.body)
    }
}

impl std::error::Error for HttpError {}

/// A request the next answer format might get through: the server rejected its shape, or the
/// answer wasn't the JSON asked for.
fn format_problem(e: &anyhow::Error) -> bool {
    e.downcast_ref::<NoVerdict>().is_some() || e.downcast_ref::<HttpError>().is_some_and(|h| h.status == 400 || h.status == 422)
}

impl Llm {
    pub fn new(config: LlmConfig) -> Self {
        // Redirects are never followed: a key or header must not be carried to another address
        // (`redirected`).
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("http client");
        Llm {
            config,
            http,
            resolved: Mutex::new(None),
        }
    }

    pub fn describe(&self) -> String {
        let output = match (self.config.output.as_str(), self.resolved.lock().unwrap().as_deref()) {
            ("auto", Some(r)) => format!("auto: {r}"),
            (o, _) => o.to_string(),
        };
        format!("{} {} ({output})", self.config.provider, self.config.model)
    }

    /// The answer format in use (`auto` before a call has settled it).
    pub fn output(&self) -> String {
        match self.config.output.as_str() {
            "auto" => self.resolved.lock().unwrap().clone().unwrap_or_else(|| "auto".into()),
            o => o.to_string(),
        }
    }

    /// One verdict for `input` that matches `schema`. Err(NoVerdict) when the model answered
    /// but not with a usable verdict; Err(other) when the call failed.
    pub async fn classify(&self, system: &str, input: &str, schema: &Value) -> Result<Verdict> {
        let auto = self.config.output == "auto";
        let resolved = self.resolved.lock().unwrap().clone();
        let modes: Vec<String> = match (auto, resolved) {
            (true, Some(m)) => vec![m],
            (true, None) => ["schema", "tool", "prompt"].map(String::from).to_vec(),
            (false, _) => vec![self.config.output.clone()],
        };
        let mut tried: Vec<(String, anyhow::Error)> = Vec::new();
        for mode in &modes {
            match self.call(mode, system, input, schema).await {
                Ok(v) => {
                    if auto {
                        *self.resolved.lock().unwrap() = Some(mode.clone());
                        if !tried.is_empty() {
                            tracing::info!(output = mode, "llm answer format settled");
                        }
                    }
                    return Ok(v);
                }
                Err(e) if modes.len() > 1 && format_problem(&e) => tried.push((mode.clone(), e)),
                Err(e) => return Err(e),
            }
        }
        if tried.len() == 1 {
            return Err(tried.pop().unwrap().1);
        }
        let all_no_verdict = tried.iter().all(|(_, e)| e.downcast_ref::<NoVerdict>().is_some());
        let why = tried.iter().map(|(m, e)| format!("{m}: {e:#}")).collect::<Vec<_>>().join("; ");
        if all_no_verdict {
            Err(NoVerdict(format!("no answer format worked ({why})")).into())
        } else {
            Err(anyhow::anyhow!("no answer format worked ({why})"))
        }
    }

    /// Keys and tokens cut out of a text that is about to be shown or logged.
    pub fn scrub(&self, s: &str) -> String {
        // A provider's own masked echo of a key ("Incorrect API key provided: sk-proj-****abcd").
        static MASKED: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let mut out = s.to_string();
        let mut secrets: Vec<String> = vec![self.config.api_key.expose().to_string()];
        secrets.extend(self.config.extra_headers.iter().map(|(_, v)| v.expose().to_string()));
        for sec in secrets.iter().filter(|s| s.len() >= 8) {
            out = out.replace(sec.as_str(), "***");
        }
        let masked = MASKED.get_or_init(|| regex::Regex::new(r"[A-Za-z0-9_\-]*\*{4,}[A-Za-z0-9_\-]*").unwrap());
        out = masked.replace_all(&out, "***").into_owned();
        crate::llm::chatgpt::scrub(&out)
    }

    /// The address requests go to: the configured one, except for the ChatGPT plan, whose
    /// access token only ever goes to OpenAI's API ([`crate::llm::chatgpt::API_BASE`]).
    fn base(&self) -> &str {
        if self.config.style == "responses" {
            crate::llm::chatgpt::API_BASE
        } else {
            self.config.base_url.trim_end_matches('/')
        }
    }

    /// Puts on `req` the credentials it may carry. Decided here, on the request itself, for
    /// every request: the key only when the request's provider and origin are the ones the key
    /// is bound to (`llm/keys.rs`), the extra headers likewise, and the ChatGPT plan's token only
    /// to OpenAI's API. Anything else goes without them.
    async fn attach(&self, req: &mut reqwest::Request) -> Result<()> {
        use reqwest::header::{HeaderName, HeaderValue, AUTHORIZATION};
        let url = req.url().clone();
        let secret = |v: &str| -> Result<HeaderValue> {
            let mut h =
                HeaderValue::from_str(v).map_err(|_| anyhow::anyhow!("a key or header value with characters a header can't hold"))?;
            h.set_sensitive(true);
            Ok(h)
        };
        let c = &self.config;
        if c.style == "responses" {
            // The ChatGPT plan's token: only to OpenAI's API (`base()` already pins the address).
            let api = reqwest::Url::parse(crate::llm::chatgpt::API_BASE).expect("api base");
            if crate::llm::keys::origin_of_url(&url) != crate::llm::keys::origin_of_url(&api)
                || !url.path().starts_with(&format!("{}/", api.path()))
            {
                anyhow::bail!("the ChatGPT plan is only used at {}", crate::llm::chatgpt::API_BASE);
            }
            let token = crate::llm::chatgpt::get().access_token().await?;
            req.headers_mut().insert(AUTHORIZATION, secret(&format!("Bearer {token}"))?);
        } else {
            if c.style == "anthropic" {
                req.headers_mut()
                    .insert("anthropic-version", HeaderValue::from_str(&c.anthropic_version)?);
            }
            let key = c.api_key.expose();
            if !key.is_empty() {
                if c.key_for.as_ref().is_some_and(|b| b.allows(&c.provider, &url)) {
                    match c.style.as_str() {
                        "anthropic" => req.headers_mut().insert("x-api-key", secret(key)?),
                        _ => req.headers_mut().insert(AUTHORIZATION, secret(&format!("Bearer {key}"))?),
                    };
                } else {
                    tracing::warn!(
                        provider = c.provider,
                        "llm key not sent: it is kept for another provider or address"
                    );
                }
            }
        }
        if !c.extra_headers.is_empty() {
            if c.headers_for.as_ref().is_some_and(|b| b.allows(&c.provider, &url)) {
                for (k, v) in &c.extra_headers {
                    let name = HeaderName::from_bytes(k.as_bytes())
                        .map_err(|_| anyhow::anyhow!("MOONRISE_LLM_EXTRA_HEADERS: a header name that isn't valid"))?;
                    req.headers_mut().insert(name, secret(v.expose())?);
                }
            } else {
                tracing::warn!(
                    provider = c.provider,
                    "llm extra headers not sent: they are for another provider or address"
                );
            }
        }
        Ok(())
    }

    async fn call(&self, mode: &str, system: &str, input: &str, schema: &Value) -> Result<Verdict> {
        let (url, body) = self.request(mode, system, input, schema);
        let mut last_err = None;
        let mut refreshed = false;
        for attempt in 0..3u32 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_secs(2u64 << attempt)).await;
            }
            let mut req = self
                .http
                .post(&url)
                .json(&body)
                .build()
                .map_err(|_| anyhow::anyhow!("llm address isn't a valid URL"))?;
            self.attach(&mut req).await?;
            let resp = match self.http.execute(req).await {
                Ok(r) => r,
                Err(e) => {
                    last_err = Some(anyhow::anyhow!(
                        "llm request failed ({})",
                        if e.is_timeout() { "timeout" } else { "connection" }
                    ));
                    continue;
                }
            };
            let status = resp.status().as_u16();
            if let Some(e) = redirected(&resp) {
                return Err(e);
            }
            let text = resp.text().await.unwrap_or_default();
            if self.config.style == "responses" {
                if let Some(e) = crate::llm::chatgpt::plan_error(status, &text) {
                    return Err(e);
                }
                if status == 401 && !refreshed {
                    // The access token may have been revoked or expired early: once more with a
                    // fresh one.
                    refreshed = true;
                    crate::llm::chatgpt::get().expire_access().await;
                    continue;
                }
            }
            if status == 429 || status >= 500 {
                last_err = Some(anyhow::anyhow!("llm HTTP {status}"));
                continue;
            }
            if status != 200 {
                return Err(HttpError {
                    status,
                    body: self.scrub(&text.chars().take(300).collect::<String>()),
                }
                .into());
            }
            let v: Value = if self.config.style == "responses" {
                responses_stream(&text)?
            } else {
                // A page (a wrong address, a login portal) rather than the API's JSON: a format
                // problem, so `auto` tries the next format and the file goes to review.
                serde_json::from_str(&text).map_err(|_| NoVerdict(not_json(&text)))?
            };
            return self.parse(mode, &v);
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("llm failed")))
    }

    fn request(&self, mode: &str, system: &str, input: &str, schema: &Value) -> (String, Value) {
        let c = &self.config;
        let base = self.base();
        let prompt_suffix = match mode {
            "tool" => "\n\nRecord your answer by calling the `verdict` tool exactly once.".to_string(),
            "prompt" => format!("\n\nAnswer with only one JSON object (no other text) that matches this JSON schema:\n{schema}"),
            _ => String::new(),
        };
        let system = format!("{system}{prompt_suffix}");
        match c.style.as_str() {
            "anthropic" => {
                let mut body = json!({
                    "model": c.model,
                    "max_tokens": c.max_tokens,
                    "system": system,
                    "messages": [{"role": "user", "content": input}],
                });
                let mut output_config = json!({});
                if !c.effort.is_empty() {
                    output_config["effort"] = json!(c.effort);
                }
                match mode {
                    "tool" => {
                        body["tools"] =
                            json!([{"name": "verdict", "description": "Record the verdict.", "input_schema": schema, "strict": true}]);
                        body["tool_choice"] = json!({"type": "auto"});
                    }
                    "prompt" => {}
                    _ => output_config["format"] = json!({"type": "json_schema", "schema": schema}),
                }
                if output_config.as_object().is_some_and(|o| !o.is_empty()) {
                    body["output_config"] = output_config;
                }
                (format!("{base}/v1/messages"), body)
            }
            "responses" => {
                // Sign in with ChatGPT: streamed, nothing stored, no output-token limit (the
                // field is refused), instructions rather than a system message.
                let mut body = json!({
                    "model": c.model,
                    "instructions": system,
                    "input": [{"role": "user", "content": input}],
                    "store": false,
                    "stream": true,
                });
                match mode {
                    "tool" => {
                        body["tools"] = json!([{"type": "function", "name": "verdict", "description": "Record the verdict.", "parameters": schema, "strict": true}]);
                        body["tool_choice"] = json!("required");
                    }
                    "prompt" => {}
                    _ => body["text"] = json!({"format": {"type": "json_schema", "name": "verdict", "schema": schema, "strict": true}}),
                }
                (format!("{base}/responses"), body)
            }
            _ => {
                let mut body = json!({
                    "model": c.model,
                    "messages": [{"role": "system", "content": system}, {"role": "user", "content": input}],
                });
                // OpenAI's reasoning models refuse `max_tokens`.
                let tokens_field = if c.provider == "openai" {
                    "max_completion_tokens"
                } else {
                    "max_tokens"
                };
                body[tokens_field] = json!(c.max_tokens);
                if !c.effort.is_empty() && reasoning_model(&c.provider, &c.model) {
                    body["reasoning_effort"] = json!(c.effort);
                }
                match mode {
                    "tool" => {
                        body["tools"] = json!([{"type": "function", "function": {"name": "verdict", "description": "Record the verdict.", "parameters": schema, "strict": true}}]);
                        body["tool_choice"] = json!("required");
                    }
                    "prompt" => {}
                    _ => {
                        body["response_format"] =
                            json!({"type": "json_schema", "json_schema": {"name": "verdict", "strict": true, "schema": schema}});
                    }
                }
                (format!("{base}/chat/completions"), body)
            }
        }
    }

    fn parse(&self, mode: &str, v: &Value) -> Result<Verdict> {
        let no = |why: String| -> anyhow::Error { NoVerdict(why).into() };
        let verdict = |value, input_tokens, output_tokens| Verdict {
            value,
            input_tokens,
            output_tokens,
            output: mode.to_string(),
        };
        match self.config.style.as_str() {
            "anthropic" => {
                let input_tokens = v["usage"]["input_tokens"].as_i64().unwrap_or(0);
                let output_tokens = v["usage"]["output_tokens"].as_i64().unwrap_or(0);
                let stop = v["stop_reason"].as_str().unwrap_or("");
                if stop != "end_turn" && stop != "tool_use" {
                    return Err(no(format!("stop_reason {stop}")));
                }
                let blocks = v["content"].as_array().cloned().unwrap_or_default();
                let value = if mode == "tool" {
                    blocks
                        .iter()
                        .find(|b| b["type"] == "tool_use" && b["name"] == "verdict")
                        .map(|b| b["input"].clone())
                        .ok_or_else(|| no("no tool call".into()))?
                } else {
                    let text: String = blocks
                        .iter()
                        .filter(|b| b["type"] == "text")
                        .filter_map(|b| b["text"].as_str())
                        .collect();
                    lenient_json(&text).ok_or_else(|| no("answer is not JSON".into()))?
                };
                Ok(verdict(value, input_tokens, output_tokens))
            }
            "responses" => {
                let input_tokens = v["usage"]["input_tokens"].as_i64().unwrap_or(0);
                let output_tokens = v["usage"]["output_tokens"].as_i64().unwrap_or(0);
                let items = v["output"].as_array().cloned().unwrap_or_default();
                let value = if mode == "tool" {
                    let args = items
                        .iter()
                        .find(|i| i["type"] == "function_call" && i["name"] == "verdict")
                        .and_then(|i| i["arguments"].as_str())
                        .ok_or_else(|| no("no tool call".into()))?;
                    serde_json::from_str(args).map_err(|_| no("tool arguments are not JSON".into()))?
                } else {
                    let parts: Vec<&Value> = items
                        .iter()
                        .filter(|i| i["type"] == "message")
                        .flat_map(|i| i["content"].as_array().into_iter().flatten())
                        .collect();
                    if parts.iter().any(|p| p["type"] == "refusal") {
                        return Err(no("refused".into()));
                    }
                    let text: String = parts
                        .iter()
                        .filter(|p| p["type"] == "output_text")
                        .filter_map(|p| p["text"].as_str())
                        .collect();
                    lenient_json(&text).ok_or_else(|| no("answer is not JSON".into()))?
                };
                Ok(verdict(value, input_tokens, output_tokens))
            }
            _ => {
                let input_tokens = v["usage"]["prompt_tokens"].as_i64().unwrap_or(0);
                let output_tokens = v["usage"]["completion_tokens"].as_i64().unwrap_or(0);
                let choice = &v["choices"][0];
                let finish = choice["finish_reason"].as_str().unwrap_or("");
                if finish != "stop" && finish != "tool_calls" {
                    return Err(no(format!("finish_reason {finish}")));
                }
                let msg = &choice["message"];
                let value = if mode == "tool" {
                    let args = msg["tool_calls"][0]["function"]["arguments"]
                        .as_str()
                        .ok_or_else(|| no("no tool call".into()))?;
                    serde_json::from_str(args).map_err(|_| no("tool arguments are not JSON".into()))?
                } else {
                    if msg["refusal"].is_string() {
                        return Err(no("refused".into()));
                    }
                    lenient_json(msg["content"].as_str().unwrap_or("")).ok_or_else(|| no("answer is not JSON".into()))?
                };
                Ok(verdict(value, input_tokens, output_tokens))
            }
        }
    }

    /// The models the endpoint offers: (id, name). Anthropic `GET {base}/v1/models`; the others
    /// `GET {base}/models` (OpenAI's `data[].id`; ChatGPT's `models[]` with `visibility: list`).
    pub async fn models(&self) -> Result<Vec<(String, String)>> {
        let base = self.base();
        let url = if self.config.style == "anthropic" {
            format!("{base}/v1/models?limit=100")
        } else {
            format!("{base}/models")
        };
        let mut tries = 0;
        let (status, text) = loop {
            tries += 1;
            let mut req = self
                .http
                .get(&url)
                .build()
                .map_err(|_| anyhow::anyhow!("llm address isn't a valid URL"))?;
            self.attach(&mut req).await?;
            let resp = self
                .http
                .execute(req)
                .await
                .map_err(|e| anyhow::anyhow!("models request failed ({})", if e.is_timeout() { "timeout" } else { "connection" }))?;
            let status = resp.status().as_u16();
            if let Some(e) = redirected(&resp) {
                return Err(e);
            }
            if status == 401 && self.config.style == "responses" && tries == 1 {
                // The access token may have been revoked or expired early: once more with a
                // fresh one.
                crate::llm::chatgpt::get().expire_access().await;
                continue;
            }
            break (status, resp.text().await.unwrap_or_default());
        };
        if status != 200 {
            return Err(HttpError {
                status,
                body: self.scrub(&text.chars().take(300).collect::<String>()),
            }
            .into());
        }
        let v: Value = serde_json::from_str(&text).map_err(|_| anyhow::anyhow!("the model list {}", not_json(&text)))?;
        Ok(model_list(&v))
    }
}

/// A redirect, which is never followed (a key or header must not be carried elsewhere): an
/// error saying where it pointed (the origin only).
fn redirected(resp: &reqwest::Response) -> Option<anyhow::Error> {
    let status = resp.status();
    if !status.is_redirection() {
        return None;
    }
    let to = resp
        .headers()
        .get(reqwest::header::LOCATION)
        .and_then(|l| l.to_str().ok())
        .and_then(|l| resp.url().join(l).ok())
        .and_then(|u| crate::llm::keys::origin_of_url(&u).map(|o| format!(" to {o}")))
        .unwrap_or_default();
    Some(
        HttpError {
            status: status.as_u16(),
            body: format!("the server redirects{to}; Moonrise doesn't follow redirects, so a key can't be passed on to another address: enter the address the requests should go to"),
        }
        .into(),
    )
}

/// Why an answer that should be JSON isn't.
fn not_json(text: &str) -> String {
    let t = text.trim_start().to_ascii_lowercase();
    if t.starts_with("<!doctype html") || t.starts_with("<html") || t.starts_with('<') {
        "answer is a web page, not JSON (is the address the API's, usually ending in /v1?)".into()
    } else {
        "answer is not JSON".into()
    }
}

/// (id, name) of each model in a model-list answer.
fn model_list(v: &Value) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    if let Some(models) = v["models"].as_array().filter(|m| m.iter().any(|x| x["slug"].is_string())) {
        // ChatGPT: the account's models, in the server's order.
        for m in models.iter().filter(|m| m["visibility"].as_str().is_none_or(|s| s == "list")) {
            if let Some(slug) = m["slug"].as_str() {
                out.push((slug.to_string(), m["display_name"].as_str().unwrap_or(slug).to_string()));
            }
        }
        return out;
    }
    for m in v["data"].as_array().into_iter().flatten() {
        if let Some(id) = m["id"].as_str() {
            out.push((id.to_string(), m["display_name"].as_str().unwrap_or(id).to_string()));
        }
    }
    out
}

/// The final response of a Responses stream (`response.completed`), or why there is none.
fn responses_stream(text: &str) -> Result<Value> {
    let mut last_type = String::new();
    for line in text.lines() {
        let Some(data) = line.strip_prefix("data:") else { continue };
        let Ok(ev) = serde_json::from_str::<Value>(data.trim()) else {
            continue;
        };
        let ty = ev["type"].as_str().unwrap_or("").to_string();
        match ty.as_str() {
            "response.completed" => return Ok(ev["response"].clone()),
            "response.incomplete" => {
                let why = ev["response"]["incomplete_details"]["reason"].as_str().unwrap_or("incomplete");
                return Err(NoVerdict(format!("response incomplete ({why})")).into());
            }
            "response.failed" | "error" => {
                let err = if ty == "error" { &ev } else { &ev["response"]["error"] };
                let code = err["code"].as_str().unwrap_or("failed");
                if let Some(e) = crate::llm::chatgpt::plan_error_code(code) {
                    return Err(e);
                }
                return Err(anyhow::anyhow!(
                    "llm response failed: {code}: {}",
                    err["message"].as_str().unwrap_or("").chars().take(200).collect::<String>()
                ));
            }
            _ => last_type = ty,
        }
    }
    Err(anyhow::anyhow!(
        "llm stream ended without a response (last event: {})",
        if last_type.is_empty() { "none" } else { &last_type }
    ))
}

/// The JSON object in a reply: the whole text, or the outermost `{…}` in it (models that wrap
/// it in prose or a code fence).
pub fn lenient_json(text: &str) -> Option<Value> {
    let t = text.trim();
    if let Ok(v @ Value::Object(_)) = serde_json::from_str(t) {
        return Some(v);
    }
    let (start, end) = (t.find('{')?, t.rfind('}')?);
    match serde_json::from_str(&t[start..=end]) {
        Ok(v @ Value::Object(_)) => Some(v),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::admin::config::Secret;

    fn config(provider: &str, output: &str) -> LlmConfig {
        let p = preset_or_custom(provider);
        LlmConfig {
            enabled: true,
            allowed: true,
            provider: provider.into(),
            style: p.style.into(),
            base_url: if p.base_url.is_empty() {
                "http://x:8080/v1".into()
            } else {
                p.base_url.into()
            },
            model: "m".into(),
            api_key: Secret::new("sk-test-0123456789"),
            output: output.into(),
            anthropic_version: "2026-10-21".into(),
            effort: "low".into(),
            max_tokens: 2048,
            timeout_secs: 60,
            max_calls_per_scan: 200,
            votes: 1,
            extra_headers: vec![],
            headers_for: None,
            key_for: crate::llm::keys::Binding::new(provider, if p.base_url.is_empty() { "http://x:8080/v1" } else { p.base_url }),
        }
    }

    fn llm(provider: &str, output: &str) -> Llm {
        Llm::new(config(provider, output))
    }

    #[test]
    fn request_shapes() {
        let schema = json!({"type": "object"});
        let (url, b) = llm("anthropic", "schema").request("schema", "sys", "in", &schema);
        assert_eq!(url, "https://api.anthropic.com/v1/messages");
        assert_eq!(b["output_config"]["format"]["type"], "json_schema");
        assert_eq!(b["output_config"]["effort"], "low");
        assert!(b.get("temperature").is_none() && b.get("tool_choice").is_none());
        let (_, b) = llm("anthropic", "tool").request("tool", "sys", "in", &schema);
        assert_eq!(b["tool_choice"]["type"], "auto");
        assert_eq!(b["tools"][0]["strict"], true);
        let (url, b) = llm("custom", "schema").request("schema", "sys", "in", &schema);
        assert_eq!(url, "http://x:8080/v1/chat/completions");
        assert_eq!(b["response_format"]["type"], "json_schema");
        assert_eq!(b["max_tokens"], 2048);
        assert!(b.get("output_config").is_none() && b.get("reasoning_effort").is_none());
        let (_, b) = llm("custom", "prompt").request("prompt", "sys", "in", &schema);
        assert!(b["messages"][0]["content"].as_str().unwrap().contains("JSON schema"));
        // OpenAI: max_completion_tokens; reasoning effort only for reasoning models.
        let mut c = config("openai", "schema");
        let (url, b) = Llm::new(c.clone()).request("schema", "sys", "in", &schema);
        assert_eq!(url, "https://api.openai.com/v1/chat/completions");
        assert_eq!(b["max_completion_tokens"], 2048);
        assert!(b.get("max_tokens").is_none() && b.get("reasoning_effort").is_none());
        c.model = "gpt-5.4-mini".into();
        assert_eq!(Llm::new(c).request("schema", "sys", "in", &schema).1["reasoning_effort"], "low");
        // ChatGPT plan: Responses, streamed, nothing stored, no token limit or system message.
        let (url, b) = llm("chatgpt", "auto").request("schema", "sys", "in", &schema);
        assert_eq!(url, "https://api.openai.com/v1/responses");
        assert_eq!((b["store"].as_bool(), b["stream"].as_bool()), (Some(false), Some(true)));
        assert_eq!(b["instructions"], "sys");
        assert_eq!(b["text"]["format"]["type"], "json_schema");
        assert!(b.get("max_output_tokens").is_none() && b.get("max_tokens").is_none());
        let (_, b) = llm("chatgpt", "auto").request("tool", "sys", "in", &schema);
        assert_eq!(b["tools"][0]["name"], "verdict");
        assert_eq!(b["tool_choice"], "required");
    }

    #[test]
    fn parsing() {
        let a = llm("anthropic", "schema");
        let v = json!({"stop_reason": "end_turn", "content": [{"type": "thinking", "thinking": ""}, {"type": "text", "text": "{\"kind\":\"movie\"}"}], "usage": {"input_tokens": 5, "output_tokens": 7}});
        let got = a.parse("schema", &v).unwrap();
        assert_eq!(got.value["kind"], "movie");
        assert_eq!((got.input_tokens, got.output_tokens), (5, 7));
        let v = json!({"stop_reason": "max_tokens", "content": [], "usage": {}});
        assert!(a.parse("schema", &v).unwrap_err().downcast_ref::<NoVerdict>().is_some());
        let o = llm("custom", "prompt");
        let v = json!({"choices": [{"finish_reason": "stop", "message": {"content": "Sure:\n```json\n{\"kind\": \"episode\"}\n```"}}], "usage": {"prompt_tokens": 3, "completion_tokens": 4}});
        assert_eq!(o.parse("prompt", &v).unwrap().value["kind"], "episode");
        let v = json!({"choices": [{"finish_reason": "tool_calls", "message": {"tool_calls": [{"function": {"name": "verdict", "arguments": "{\"kind\":\"extra\"}"}}]}}]});
        assert_eq!(o.parse("tool", &v).unwrap().value["kind"], "extra");
        // Responses: the completed response from the stream.
        let r = llm("chatgpt", "auto");
        let stream = "event: response.created\ndata: {\"type\":\"response.created\"}\n\nevent: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"{\"}\n\n\
            event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"type\":\"reasoning\"},{\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"{\\\"kind\\\":\\\"movie\\\"}\"}]}],\"usage\":{\"input_tokens\":9,\"output_tokens\":2}}}\n\n";
        let v = responses_stream(stream).unwrap();
        let got = r.parse("schema", &v).unwrap();
        assert_eq!((got.value["kind"].as_str(), got.input_tokens), (Some("movie"), 9));
        let v = json!({"output": [{"type": "function_call", "name": "verdict", "arguments": "{\"kind\":\"extra\"}"}]});
        assert_eq!(r.parse("tool", &v).unwrap().value["kind"], "extra");
        let limit = "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\",\"message\":\"x\"}}}\n";
        assert!(format!("{:#}", responses_stream(limit).unwrap_err()).contains("limit"));
        let cut = "data: {\"type\":\"response.incomplete\",\"response\":{\"incomplete_details\":{\"reason\":\"max_output_tokens\"}}}\n";
        assert!(responses_stream(cut).unwrap_err().downcast_ref::<NoVerdict>().is_some());
        assert!(responses_stream("data: {\"type\":\"response.created\"}\n").is_err());
    }

    #[test]
    fn model_lists() {
        let openai =
            json!({"object": "list", "data": [{"id": "gpt-5.4-mini"}, {"id": "claude-haiku-5-5", "display_name": "Claude Haiku 5.5"}]});
        assert_eq!(
            model_list(&openai),
            vec![
                ("gpt-5.4-mini".into(), "gpt-5.4-mini".into()),
                ("claude-haiku-5-5".into(), "Claude Haiku 5.5".into())
            ]
        );
        let chatgpt = json!({"models": [{"slug": "a", "display_name": "A", "visibility": "list"}, {"slug": "b", "visibility": "hide"}]});
        assert_eq!(model_list(&chatgpt), vec![("a".into(), "A".into())]);
    }

    #[test]
    fn scrubbing_and_origins() {
        let o = llm("custom", "auto");
        assert_eq!(o.scrub("bad key sk-test-0123456789 given"), "bad key *** given");
        assert_eq!(
            o.scrub("Incorrect API key provided: sk-proj-****************abcd. You can"),
            "Incorrect API key provided: ***. You can"
        );
        assert_eq!(origin("https://API.openai.com/v1/"), "https://api.openai.com");
        assert_eq!(origin("http://192.168.1.5:8080"), "http://192.168.1.5:8080");
        assert_eq!(origin("nope"), "");
        assert!(format_problem(
            &HttpError {
                status: 400,
                body: String::new()
            }
            .into()
        ));
        assert!(!format_problem(
            &HttpError {
                status: 401,
                body: String::new()
            }
            .into()
        ));
        assert!(format_problem(&NoVerdict("x".into()).into()));
    }

    /// `auto` falls back from a format the server rejects and keeps the one that worked.
    #[tokio::test]
    async fn auto_falls_back_and_remembers() {
        use axum::{routing::post, Json, Router};
        let seen = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
        let s2 = seen.clone();
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move |Json(b): Json<Value>| {
                let seen = s2.clone();
                async move {
                    let mode = if b.get("response_format").is_some() {
                        "schema"
                    } else if b.get("tools").is_some() {
                        "tool"
                    } else {
                        "prompt"
                    };
                    seen.lock().unwrap().push(mode.to_string());
                    match mode {
                        "schema" => (
                            axum::http::StatusCode::BAD_REQUEST,
                            Json(json!({"error": "response_format not supported"})),
                        ),
                        // A server that ignores the tool and answers in prose.
                        "tool" => (
                            axum::http::StatusCode::OK,
                            Json(json!({"choices": [{"finish_reason": "stop", "message": {"content": "It is a movie."}}]})),
                        ),
                        _ => (
                            axum::http::StatusCode::OK,
                            Json(json!({"choices": [{"finish_reason": "stop", "message": {"content": "{\"kind\":\"movie\"}"}}]})),
                        ),
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut c = config("custom", "auto");
        c.base_url = format!("http://{addr}/v1");
        let l = Llm::new(c);
        let v = l.classify("sys", "in", &json!({"type": "object"})).await.unwrap();
        assert_eq!((v.value["kind"].as_str(), v.output.as_str()), (Some("movie"), "prompt"));
        assert_eq!(l.output(), "prompt");
        l.classify("sys", "in", &json!({"type": "object"})).await.unwrap();
        assert_eq!(*seen.lock().unwrap(), vec!["schema", "tool", "prompt", "prompt"]);
        assert!(l.describe().contains("auto: prompt"));
    }

    /// A server that answers 200 with a web page: a format problem, so `auto` moves on; a
    /// fixed format gets "no verdict" with a hint about the address.
    #[tokio::test]
    async fn html_answer_is_a_format_problem() {
        use axum::response::IntoResponse;
        use axum::{routing::post, Json, Router};
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|Json(b): Json<Value>| async move {
                if b.get("response_format").is_some() || b.get("tools").is_some() {
                    axum::response::Html("<!DOCTYPE html><html><body>Sign in to the portal</body></html>").into_response()
                } else {
                    Json(json!({"choices": [{"finish_reason": "stop", "message": {"content": "{\"kind\":\"movie\"}"}}]})).into_response()
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut c = config("custom", "auto");
        c.base_url = format!("http://{addr}/v1");
        let v = Llm::new(c.clone()).classify("sys", "in", &json!({"type": "object"})).await.unwrap();
        assert_eq!(v.output, "prompt");
        c.output = "schema".into();
        let e = Llm::new(c).classify("sys", "in", &json!({"type": "object"})).await.unwrap_err();
        assert!(e.downcast_ref::<NoVerdict>().is_some());
        assert!(format!("{e:#}").contains("web page"), "{e:#}");
    }

    /// A recording server: (path, the credentials each request carried).
    pub(crate) async fn recorder(redirect_to: Option<(u16, String)>) -> (String, std::sync::Arc<Mutex<Vec<String>>>) {
        use axum::{http::HeaderMap, response::IntoResponse, Router};
        let seen = std::sync::Arc::new(Mutex::new(Vec::<String>::new()));
        let s2 = seen.clone();
        let app = Router::new().fallback(move |h: HeaderMap, uri: axum::http::Uri| {
            let seen = s2.clone();
            let to = redirect_to.clone();
            async move {
                let creds: Vec<String> = ["authorization", "x-api-key", "x-org"].iter().filter_map(|k| h.get(*k).map(|v| format!("{k}={}", v.to_str().unwrap_or("")))).collect();
                seen.lock().unwrap().push(format!("{} {}", uri.path(), creds.join(",")));
                match to {
                    Some((code, to)) => (axum::http::StatusCode::from_u16(code).unwrap(), [(axum::http::header::LOCATION, format!("{to}{}", uri.path()))]).into_response(),
                    None => axum::Json(json!({"choices": [{"finish_reason": "stop", "message": {"content": "{\"kind\":\"movie\"}"}}], "data": [], "content": [{"type": "text", "text": "{\"kind\":\"movie\"}"}], "stop_reason": "end_turn"})).into_response(),
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), seen)
    }

    /// The request layer: a key or header goes only with a request of the provider and origin
    /// it is bound to, whatever the configuration says otherwise; redirects are never followed.
    #[tokio::test]
    async fn credentials_only_where_they_are_bound() {
        let (a, seen_a) = recorder(None).await;
        let (b, seen_b) = recorder(None).await;
        let ask = |c: LlmConfig| async move {
            let l = Llm::new(c);
            let _ = l.classify("sys", "in", &json!({"type": "object"})).await;
            let _ = l.models().await;
        };
        for provider in ["custom", "anthropic"] {
            let mut c = config(provider, "prompt");
            c.base_url = format!("{a}/v1");
            c.key_for = crate::llm::keys::Binding::new(provider, &a);
            c.extra_headers = vec![("X-Org".into(), Secret::new("org-secret-1"))];
            c.headers_for = c.key_for.clone();
            ask(c.clone()).await;
            let got = seen_a.lock().unwrap().drain(..).collect::<Vec<_>>();
            assert_eq!(got.len(), 2, "{got:?}");
            assert!(
                got.iter().all(|g| g.contains("sk-test-0123456789") && g.contains("org-secret-1")),
                "{got:?}"
            );
            // The same configuration at another address: nothing of them goes there.
            ask(LlmConfig {
                base_url: format!("{b}/v1"),
                ..c.clone()
            })
            .await;
            // Bound to another provider at this address: not sent either.
            ask(LlmConfig {
                key_for: crate::llm::keys::Binding::new("openrouter", &a),
                headers_for: crate::llm::keys::Binding::new("openrouter", &a),
                ..c.clone()
            })
            .await;
            // No binding: nowhere.
            ask(LlmConfig {
                key_for: None,
                headers_for: None,
                ..c.clone()
            })
            .await;
            let all: Vec<String> = seen_a.lock().unwrap().drain(..).chain(seen_b.lock().unwrap().drain(..)).collect();
            assert_eq!(all.len(), 6);
            assert!(all.iter().all(|g| g.ends_with(' ')), "no credential: {all:?}");
        }
    }

    /// A redirect from the key's own address (301, 302, 307, 308) to another: not followed, so
    /// nothing reaches the other address; the error says why.
    #[tokio::test]
    async fn redirects_are_not_followed() {
        let (catcher, caught) = recorder(None).await;
        for code in [301, 302, 303, 307, 308] {
            let (a, seen_a) = recorder(Some((code, catcher.clone()))).await;
            for provider in ["custom", "anthropic"] {
                let mut c = config(provider, "schema");
                c.base_url = format!("{a}/v1");
                c.key_for = crate::llm::keys::Binding::new(provider, &a);
                c.extra_headers = vec![("X-Org".into(), Secret::new("org-secret-1"))];
                c.headers_for = c.key_for.clone();
                let l = Llm::new(c);
                let e = l.classify("sys", "in", &json!({"type": "object"})).await.unwrap_err();
                assert!(format!("{e:#}").contains("doesn't follow redirects"), "{e:#}");
                assert!(format!("{e:#}").contains(&catcher.replace("http://", "")), "says where: {e:#}");
                assert!(l.models().await.is_err());
            }
            assert_eq!(seen_a.lock().unwrap().len(), 4);
        }
        assert!(
            caught.lock().unwrap().is_empty(),
            "nothing followed the redirect: {:?}",
            caught.lock().unwrap()
        );
    }
}
