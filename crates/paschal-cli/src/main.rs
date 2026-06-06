//! `paschal` — CLI client for the Beacon.
//!
//! In production this role is split between the Sentinel agent (Tauri tray
//! app) and the principal's browser. The CLI stands in for both so a power
//! user can drive the full HTTP surface without a UI.
//!
//! Config & session persist to `~/.paschal/state.json`. A fresh `signup`
//! overwrites it.

use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
};

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use reqwest::{multipart, Client};
use serde::{Deserialize, Serialize};
use serde_json::json;

const DEFAULT_BEACON: &str = "http://127.0.0.1:8080";

#[derive(Parser, Debug)]
#[command(name = "paschal", about = "Paschal CLI", version)]
struct Cli {
    /// Override the Beacon base URL.
    #[arg(long, env = "PASCHAL_BEACON", default_value = DEFAULT_BEACON)]
    beacon: String,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Create a Principal + Subscription, and save a session.
    Signup {
        #[arg(long)]
        email: String,
        #[arg(long)]
        display_name: Option<String>,
        /// "monthly" (default) or "annual".
        #[arg(long, default_value = "monthly")]
        plan: String,
    },

    /// Vault operations.
    #[command(subcommand)]
    Vault(VaultCmd),

    /// Letter operations.
    #[command(subcommand)]
    Letter(LetterCmd),

    /// Send a Heartbeat to keep the trigger calm.
    Heartbeat,

    /// Manually trigger release on a Vault (skips the signal aggregator).
    ForceRelease {
        #[arg(long)]
        vault: String,
    },

    /// Cancel a release in cooling-off.
    Cancel {
        #[arg(long)]
        vault: String,
    },

    /// Run a Drill (rehearsal) on a Vault.
    Drill {
        #[arg(long)]
        vault: String,
    },

    /// Buddy operations.
    #[command(subcommand)]
    Buddy(BuddyCmd),

    /// Subscription operations.
    #[command(subcommand)]
    Subscription(SubCmd),

    /// Account operations.
    #[command(subcommand)]
    Account(AccountCmd),

    /// Microsoft Account observation (manual / scripted).
    MicrosoftObserve {
        #[arg(long)]
        last_sign_in_at: String,
    },

    /// Show the saved session + current Vault states.
    Status,
}

#[derive(Subcommand, Debug)]
enum VaultCmd {
    /// Create a Vault.
    Create {
        #[arg(long)]
        name: String,
        /// Override cooling-off length in seconds.
        #[arg(long)]
        cooling_off_seconds: Option<i32>,
    },
    /// List Vaults.
    List,
    /// Show one Vault.
    Get {
        #[arg(long)]
        vault: String,
    },
}

#[derive(Subcommand, Debug)]
enum LetterCmd {
    /// Seal a text-only Letter.
    Seal {
        #[arg(long)]
        vault: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        recipient_email: String,
        #[arg(long, conflicts_with = "body_file")]
        body: Option<String>,
        #[arg(long)]
        body_file: Option<PathBuf>,
        #[arg(long)]
        drill_body: Option<String>,
        /// ISO-8601 timestamp for a scheduled release.
        #[arg(long)]
        scheduled_release_at: Option<String>,
    },
    /// Seal a Letter with one or more attachments (multipart upload).
    Upload {
        #[arg(long)]
        vault: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        recipient_email: String,
        #[arg(long)]
        body: Option<String>,
        /// Path to a file to attach. May be repeated.
        #[arg(long = "file", action = clap::ArgAction::Append)]
        files: Vec<PathBuf>,
    },
    /// List Letters in a Vault.
    List {
        #[arg(long)]
        vault: String,
    },
}

#[derive(Subcommand, Debug)]
enum BuddyCmd {
    /// Invite a Buddy. Returns a confirmation link (development convenience).
    Invite {
        #[arg(long)]
        email: String,
        #[arg(long)]
        display_name: Option<String>,
        #[arg(long)]
        phone: Option<String>,
        #[arg(long, default_value = "90")]
        prompt_cadence_days: i32,
    },
    /// List Buddies.
    List,
    /// Confirm a Buddy invite (the Buddy uses this with the token from email).
    Confirm {
        #[arg(long)]
        token: String,
    },
    /// Submit a Buddy's response.
    Respond {
        #[arg(long)]
        buddy: String,
        /// One of WELL, WORRIED, UNABLE_TO_REACH.
        #[arg(long)]
        response: String,
    },
    /// Revoke a Buddy.
    Revoke {
        #[arg(long)]
        buddy: String,
    },
}

#[derive(Subcommand, Debug)]
enum SubCmd {
    /// Show the current Subscription.
    Get,
    /// Cancel the Subscription (enters retention window).
    Cancel,
    /// Reactivate a canceled Subscription within retention.
    Reactivate,
}

#[derive(Subcommand, Debug)]
enum AccountCmd {
    /// Request account deletion (30-day cool-off).
    Delete,
    /// Cancel a pending deletion within its cool-off window.
    CancelDeletion,
}

// ---------------------------------------------------------------------------
// Local state — saved at ~/.paschal/state.json
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Serialize, Deserialize)]
struct LocalState {
    beacon: String,
    principal_id: Option<String>,
    session_token: Option<String>,
    email: Option<String>,
    last_magic_token: Option<String>,
}

fn state_dir() -> Result<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .ok_or_else(|| anyhow!("HOME / USERPROFILE not set"))?;
    let mut p = PathBuf::from(home);
    p.push(".paschal");
    Ok(p)
}

fn state_path() -> Result<PathBuf> {
    let mut p = state_dir()?;
    p.push("state.json");
    Ok(p)
}

fn load_state() -> Result<LocalState> {
    let path = state_path()?;
    if !path.exists() {
        return Ok(LocalState::default());
    }
    let raw = fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&raw)?)
}

fn save_state(state: &LocalState) -> Result<()> {
    let dir = state_dir()?;
    fs::create_dir_all(&dir)?;
    let path = state_path()?;
    let mut tmp = path.clone();
    tmp.set_extension("json.tmp");
    let mut f = fs::File::create(&tmp)?;
    f.write_all(serde_json::to_vec_pretty(state)?.as_slice())?;
    f.flush()?;
    fs::rename(tmp, path)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn http() -> Client {
    Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .expect("reqwest client builder")
}

async fn authed_request(
    client: &Client,
    state: &LocalState,
    method: reqwest::Method,
    url: String,
    body: Option<serde_json::Value>,
) -> Result<reqwest::Response> {
    let token = state
        .session_token
        .as_ref()
        .ok_or_else(|| anyhow!("not signed in — run `paschal signup` first"))?;
    let mut req = client.request(method, url).bearer_auth(token);
    if let Some(b) = body {
        req = req.json(&b);
    }
    Ok(req.send().await?)
}

async fn ok_json(resp: reqwest::Response) -> Result<serde_json::Value> {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if status.is_success() {
        Ok(if text.is_empty() {
            json!({})
        } else {
            serde_json::from_str::<serde_json::Value>(&text)
                .unwrap_or_else(|_| json!({ "raw": text }))
        })
    } else {
        Err(anyhow!("{}: {}", status, text))
    }
}

// ---------------------------------------------------------------------------
// Command handlers
// ---------------------------------------------------------------------------

async fn cmd_signup(beacon: &str, email: &str, display_name: Option<String>, plan: &str) -> Result<()> {
    let client = http();
    let url = format!("{}/v1/auth/signup", beacon.trim_end_matches('/'));
    let body = json!({ "email": email, "display_name": display_name, "plan": plan });
    let resp = client.post(&url).json(&body).send().await?;
    let v = ok_json(resp).await.context("signup failed")?;

    let session_token = v["session_token"]
        .as_str()
        .ok_or_else(|| anyhow!("server did not return session_token"))?
        .to_string();
    let state = LocalState {
        beacon: beacon.to_string(),
        principal_id: v["principal_id"].as_str().map(str::to_string),
        session_token: Some(session_token),
        email: Some(email.to_string()),
        last_magic_token: v["magic_token_DEV_ONLY"].as_str().map(str::to_string),
    };
    save_state(&state)?;

    println!("signed up");
    println!("  email             : {email}");
    if let Some(pid) = state.principal_id { println!("  principal_id      : {pid}"); }
    println!("  subscription_state: {}", v["subscription_state"]);
    println!("  trial_end_at      : {}", v["trial_end_at"]);
    Ok(())
}

async fn cmd_vault_create(beacon: &str, name: &str, cooling: Option<i32>) -> Result<()> {
    let state = load_state()?;
    let url = format!("{}/v1/vaults", beacon.trim_end_matches('/'));
    let mut body = json!({ "name": name, "tier": "HONEST_OPERATOR" });
    if let Some(c) = cooling { body["cooling_off_seconds"] = json!(c); }
    let resp = authed_request(&http(), &state, reqwest::Method::POST, url, Some(body)).await?;
    let v = ok_json(resp).await.context("create vault failed")?;
    println!("vault created");
    println!("  id  : {}", v["id"]);
    println!("  name: {}", v["name"]);
    Ok(())
}

async fn cmd_vault_list(beacon: &str) -> Result<()> {
    let state = load_state()?;
    let url = format!("{}/v1/vaults", beacon.trim_end_matches('/'));
    let resp = authed_request(&http(), &state, reqwest::Method::GET, url, None).await?;
    let v = ok_json(resp).await?;
    let arr = v.as_array().cloned().unwrap_or_default();
    if arr.is_empty() {
        println!("(no Vaults yet)");
        return Ok(());
    }
    for vlt in arr {
        println!(
            "{}\t{}\t{}\t{} sec",
            vlt["id"].as_str().unwrap_or(""),
            vlt["state"].as_str().unwrap_or(""),
            vlt["name"].as_str().unwrap_or(""),
            vlt["cooling_off_seconds"],
        );
    }
    Ok(())
}

async fn cmd_vault_get(beacon: &str, vault: &str) -> Result<()> {
    let state = load_state()?;
    let url = format!("{}/v1/vaults/{}", beacon.trim_end_matches('/'), vault);
    let resp = authed_request(&http(), &state, reqwest::Method::GET, url, None).await?;
    let v = ok_json(resp).await?;
    println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    Ok(())
}

async fn cmd_letter_seal(
    beacon: &str,
    vault: &str,
    title: &str,
    recipient: &str,
    body_inline: Option<String>,
    body_file: Option<PathBuf>,
    drill_body: Option<String>,
    scheduled_release_at: Option<String>,
) -> Result<()> {
    let body = read_body(body_inline, body_file)?;
    let state = load_state()?;
    let url = format!("{}/v1/vaults/{}/letters", beacon.trim_end_matches('/'), vault);
    let mut payload = json!({ "title": title, "recipient_email": recipient, "body": body });
    if let Some(d) = drill_body { payload["drill_body"] = json!(d); }
    if let Some(s) = scheduled_release_at { payload["scheduled_release_at"] = json!(s); }
    let resp = authed_request(&http(), &state, reqwest::Method::POST, url, Some(payload)).await?;
    let v = ok_json(resp).await.context("seal letter failed")?;
    println!("letter sealed");
    println!("  id       : {}", v["id"]);
    println!("  title    : {}", v["title"]);
    println!("  recipient: {}", v["recipient_email"]);
    println!("  sealed_at: {}", v["sealed_at"]);
    Ok(())
}

async fn cmd_letter_upload(
    beacon: &str,
    vault: &str,
    title: &str,
    recipient: &str,
    body: Option<String>,
    files: Vec<PathBuf>,
) -> Result<()> {
    let state = load_state()?;
    let token = state
        .session_token
        .as_ref()
        .ok_or_else(|| anyhow!("not signed in — run `paschal signup` first"))?;
    if files.is_empty() && body.is_none() {
        return Err(anyhow!("at least one --file or --body required"));
    }

    let mut form = multipart::Form::new()
        .text("title", title.to_string())
        .text("recipient_email", recipient.to_string());

    if let Some(b) = body { form = form.text("body", b); }

    for path in &files {
        let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("file").to_string();
        let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let mime = mime_guess_for(&filename);
        let part = multipart::Part::bytes(bytes).file_name(filename).mime_str(&mime)?;
        form = form.part("file", part);
    }

    let url = format!("{}/v1/vaults/{}/letters/multipart", beacon.trim_end_matches('/'), vault);
    let resp = http().post(&url).bearer_auth(token).multipart(form).send().await?;
    let v = ok_json(resp).await.context("upload failed")?;
    println!("letter sealed with attachments");
    println!("  id          : {}", v["id"]);
    println!("  title       : {}", v["title"]);
    if let Some(arr) = v["attachments"].as_array() {
        println!("  attachments :");
        for a in arr {
            println!(
                "    - {} ({} bytes -> {} bytes, {})",
                a["original_filename"].as_str().unwrap_or(""),
                a["original_size"],
                a["transformed_size"],
                a["transformed_mime"].as_str().unwrap_or(""),
            );
            if let Some(notes) = a["transformer_notes"]["notes"].as_array() {
                for n in notes {
                    println!("        * {}", n.as_str().unwrap_or(""));
                }
            }
        }
    }
    Ok(())
}

fn mime_guess_for(filename: &str) -> String {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "txt" | "log" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "json" => "application/json",
        "xml" => "application/xml",
        "html" | "htm" => "text/html",
        "pdf" => "application/pdf",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn read_body(inline: Option<String>, file: Option<PathBuf>) -> Result<String> {
    Ok(match (inline, file) {
        (Some(b), _) => b,
        (_, Some(p)) => fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))?,
        (None, None) => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            s
        }
    })
}

async fn cmd_letter_list(beacon: &str, vault: &str) -> Result<()> {
    let state = load_state()?;
    let url = format!("{}/v1/vaults/{}/letters", beacon.trim_end_matches('/'), vault);
    let resp = authed_request(&http(), &state, reqwest::Method::GET, url, None).await?;
    let v = ok_json(resp).await?;
    for l in v.as_array().cloned().unwrap_or_default() {
        println!(
            "{}\t{}\t{}",
            l["id"].as_str().unwrap_or(""),
            l["recipient_email"].as_str().unwrap_or(""),
            l["title"].as_str().unwrap_or(""),
        );
    }
    Ok(())
}

async fn cmd_heartbeat(beacon: &str) -> Result<()> {
    let state = load_state()?;
    let url = format!("{}/v1/heartbeats", beacon.trim_end_matches('/'));
    let resp =
        authed_request(&http(), &state, reqwest::Method::POST, url, Some(json!({ "via": "CLI" }))).await?;
    let v = ok_json(resp).await?;
    println!("heartbeat received_at = {}", v["received_at"]);
    Ok(())
}

async fn cmd_force_release(beacon: &str, vault: &str) -> Result<()> {
    let state = load_state()?;
    let url = format!("{}/v1/vaults/{}/force-release", beacon.trim_end_matches('/'), vault);
    let resp = authed_request(&http(), &state, reqwest::Method::POST, url, None).await?;
    let v = ok_json(resp).await.context("force-release failed")?;
    println!("cooling-off started");
    println!("  release_event_id      : {}", v["release_event_id"]);
    println!("  cooling_off_started_at: {}", v["cooling_off_started_at"]);
    println!("  cooling_off_ends_at   : {}", v["cooling_off_ends_at"]);
    Ok(())
}

async fn cmd_cancel(beacon: &str, vault: &str) -> Result<()> {
    let state = load_state()?;
    let url = format!("{}/v1/vaults/{}/cancel", beacon.trim_end_matches('/'), vault);
    let resp = authed_request(&http(), &state, reqwest::Method::POST, url, None).await?;
    let v = ok_json(resp).await.context("cancel failed")?;
    println!("cancelled — vault state is now {}", v["state"]);
    Ok(())
}

async fn cmd_drill(beacon: &str, vault: &str) -> Result<()> {
    let state = load_state()?;
    let url = format!("{}/v1/vaults/{}/drills", beacon.trim_end_matches('/'), vault);
    let resp = authed_request(&http(), &state, reqwest::Method::POST, url, None).await?;
    let v = ok_json(resp).await.context("drill failed")?;
    println!("drill started");
    println!("  release_event_id      : {}", v["release_event_id"]);
    println!("  cooling_off_ends_at   : {}", v["cooling_off_ends_at"]);
    println!();
    println!("Recipients will receive a notification starting with 'Rehearsal —'.");
    Ok(())
}

async fn cmd_buddy_invite(
    beacon: &str,
    email: &str,
    display_name: Option<String>,
    phone: Option<String>,
    prompt_cadence_days: i32,
) -> Result<()> {
    let state = load_state()?;
    let url = format!("{}/v1/principals/me/buddies", beacon.trim_end_matches('/'));
    let body = json!({
        "email": email,
        "display_name": display_name,
        "phone": phone,
        "prompt_cadence_days": prompt_cadence_days,
    });
    let resp = authed_request(&http(), &state, reqwest::Method::POST, url, Some(body)).await?;
    let v = ok_json(resp).await.context("invite buddy failed")?;
    println!("buddy invited");
    println!("  id          : {}", v["buddy"]["id"]);
    println!("  email       : {}", v["buddy"]["email"]);
    if let Some(tok) = v["confirmation_token_DEV_ONLY"].as_str() {
        println!("  confirmation token (dev): {}", tok);
    }
    Ok(())
}

async fn cmd_buddy_list(beacon: &str) -> Result<()> {
    let state = load_state()?;
    let url = format!("{}/v1/principals/me/buddies", beacon.trim_end_matches('/'));
    let resp = authed_request(&http(), &state, reqwest::Method::GET, url, None).await?;
    let v = ok_json(resp).await?;
    for b in v.as_array().cloned().unwrap_or_default() {
        println!(
            "{}\t{}\t{}\t{}",
            b["id"].as_str().unwrap_or(""),
            b["email"].as_str().unwrap_or(""),
            if b["confirmed"].as_bool().unwrap_or(false) { "confirmed" } else { "pending" },
            b["last_response"].as_str().unwrap_or("(no response)"),
        );
    }
    Ok(())
}

async fn cmd_buddy_confirm(beacon: &str, token: &str) -> Result<()> {
    let url = format!("{}/v1/buddies/confirm", beacon.trim_end_matches('/'));
    let resp = http().post(&url).json(&json!({ "token": token })).send().await?;
    let v = ok_json(resp).await.context("confirm failed")?;
    println!("confirmed: {}", v["email"]);
    Ok(())
}

async fn cmd_buddy_respond(beacon: &str, buddy: &str, response: &str) -> Result<()> {
    let url = format!("{}/v1/buddies/{}/responses", beacon.trim_end_matches('/'), buddy);
    let resp = http().post(&url).json(&json!({ "response": response })).send().await?;
    let v = ok_json(resp).await.context("respond failed")?;
    println!("response recorded: {}", v["last_response"]);
    Ok(())
}

async fn cmd_buddy_revoke(beacon: &str, buddy: &str) -> Result<()> {
    let state = load_state()?;
    let url = format!("{}/v1/buddies/{}", beacon.trim_end_matches('/'), buddy);
    let resp = authed_request(&http(), &state, reqwest::Method::DELETE, url, None).await?;
    let v = ok_json(resp).await?;
    println!("revoked: {}", v);
    Ok(())
}

async fn cmd_subscription(beacon: &str, action: &str) -> Result<()> {
    let state = load_state()?;
    let path = match action {
        "get" => "/v1/principals/me/subscription",
        "cancel" => "/v1/principals/me/subscription/cancel",
        "reactivate" => "/v1/principals/me/subscription/reactivate",
        _ => unreachable!(),
    };
    let url = format!("{}{}", beacon.trim_end_matches('/'), path);
    let method = if action == "get" { reqwest::Method::GET } else { reqwest::Method::POST };
    let resp = authed_request(&http(), &state, method, url, None).await?;
    let v = ok_json(resp).await.context("subscription op failed")?;
    println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    Ok(())
}

async fn cmd_account_delete(beacon: &str) -> Result<()> {
    let state = load_state()?;
    let url = format!("{}/v1/principals/me", beacon.trim_end_matches('/'));
    let resp = authed_request(&http(), &state, reqwest::Method::DELETE, url, None).await?;
    let v = ok_json(resp).await.context("delete request failed")?;
    println!("account deletion requested");
    println!("  scheduled_for: {}", v["deletion_scheduled_for"]);
    println!();
    println!("This is reversible until the scheduled time. Run:");
    println!("  paschal account cancel-deletion");
    Ok(())
}

async fn cmd_account_cancel_deletion(beacon: &str) -> Result<()> {
    let state = load_state()?;
    let url = format!(
        "{}/v1/principals/me/cancel-deletion",
        beacon.trim_end_matches('/')
    );
    let resp = authed_request(&http(), &state, reqwest::Method::POST, url, None).await?;
    let v = ok_json(resp).await?;
    println!("deletion cancelled: {}", v);
    Ok(())
}

async fn cmd_microsoft_observe(beacon: &str, last_sign_in_at: &str) -> Result<()> {
    let state = load_state()?;
    let url = format!("{}/v1/signals/microsoft/observe", beacon.trim_end_matches('/'));
    let resp = authed_request(
        &http(),
        &state,
        reqwest::Method::POST,
        url,
        Some(json!({ "last_sign_in_at": last_sign_in_at })),
    )
    .await?;
    let v = ok_json(resp).await?;
    println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    Ok(())
}

async fn cmd_status(beacon: &str) -> Result<()> {
    let state = load_state()?;
    if state.session_token.is_none() {
        println!("not signed in. Run: paschal signup --email you@example.com");
        return Ok(());
    }
    println!("Local state");
    println!("  beacon       : {}", state.beacon);
    println!("  email        : {}", state.email.as_deref().unwrap_or("?"));
    println!("  principal_id : {}", state.principal_id.as_deref().unwrap_or("?"));
    println!();
    println!("Vaults");
    let _ = cmd_vault_list(beacon).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,paschal=info")),
        )
        .compact()
        .init();

    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Signup { email, display_name, plan } => {
            cmd_signup(&cli.beacon, &email, display_name, &plan).await
        }
        Cmd::Vault(VaultCmd::Create { name, cooling_off_seconds }) => {
            cmd_vault_create(&cli.beacon, &name, cooling_off_seconds).await
        }
        Cmd::Vault(VaultCmd::List) => cmd_vault_list(&cli.beacon).await,
        Cmd::Vault(VaultCmd::Get { vault }) => cmd_vault_get(&cli.beacon, &vault).await,
        Cmd::Letter(LetterCmd::Seal {
            vault, title, recipient_email, body, body_file, drill_body, scheduled_release_at,
        }) => {
            cmd_letter_seal(
                &cli.beacon, &vault, &title, &recipient_email, body, body_file, drill_body,
                scheduled_release_at,
            )
            .await
        }
        Cmd::Letter(LetterCmd::Upload { vault, title, recipient_email, body, files }) => {
            cmd_letter_upload(&cli.beacon, &vault, &title, &recipient_email, body, files).await
        }
        Cmd::Letter(LetterCmd::List { vault }) => cmd_letter_list(&cli.beacon, &vault).await,
        Cmd::Heartbeat => cmd_heartbeat(&cli.beacon).await,
        Cmd::ForceRelease { vault } => cmd_force_release(&cli.beacon, &vault).await,
        Cmd::Cancel { vault } => cmd_cancel(&cli.beacon, &vault).await,
        Cmd::Drill { vault } => cmd_drill(&cli.beacon, &vault).await,
        Cmd::Buddy(BuddyCmd::Invite { email, display_name, phone, prompt_cadence_days }) => {
            cmd_buddy_invite(&cli.beacon, &email, display_name, phone, prompt_cadence_days).await
        }
        Cmd::Buddy(BuddyCmd::List) => cmd_buddy_list(&cli.beacon).await,
        Cmd::Buddy(BuddyCmd::Confirm { token }) => cmd_buddy_confirm(&cli.beacon, &token).await,
        Cmd::Buddy(BuddyCmd::Respond { buddy, response }) => {
            cmd_buddy_respond(&cli.beacon, &buddy, &response).await
        }
        Cmd::Buddy(BuddyCmd::Revoke { buddy }) => cmd_buddy_revoke(&cli.beacon, &buddy).await,
        Cmd::Subscription(SubCmd::Get) => cmd_subscription(&cli.beacon, "get").await,
        Cmd::Subscription(SubCmd::Cancel) => cmd_subscription(&cli.beacon, "cancel").await,
        Cmd::Subscription(SubCmd::Reactivate) => cmd_subscription(&cli.beacon, "reactivate").await,
        Cmd::Account(AccountCmd::Delete) => cmd_account_delete(&cli.beacon).await,
        Cmd::Account(AccountCmd::CancelDeletion) => cmd_account_cancel_deletion(&cli.beacon).await,
        Cmd::MicrosoftObserve { last_sign_in_at } => {
            cmd_microsoft_observe(&cli.beacon, &last_sign_in_at).await
        }
        Cmd::Status => cmd_status(&cli.beacon).await,
    }
}
