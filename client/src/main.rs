//! `teresa` — the terminal client binary.
//!
//! Owns exactly the things a library cannot: argv, stdin/stdout, the terminal
//! mode, the tokio runtime, and the wiring between threads. Everything with
//! logic in it lives in `teresa_client` and is tested there.

use std::io::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use ratatui::crossterm::event::{
    self as term_event, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste,
    EnableMouseCapture, Event as TermEvent, KeyEventKind,
};
use ratatui::crossterm::terminal::SetTitle;
use ratatui::crossterm::{execute, terminal};
use teresa_client::app::event::Event;
use teresa_client::app::update::update;
use teresa_client::app::{App, Effect};
use teresa_client::config::ClientConfig;
use teresa_client::intro::Intro;
use teresa_client::net::http::{AuthClient, AuthError};
use teresa_client::net::transfer::{TransferClient, TransferUpdate};
use teresa_client::net::ws::{self, NetEvent};
use teresa_client::session::Session;
use teresa_client::ui;
use tokio::sync::mpsc;

/// The animation/refresh heartbeat. ~30 Hz keeps spinners fluid at a fraction
/// of the cost of true 60; actual message traffic redraws immediately anyway.
const TICK: Duration = Duration::from_millis(33);

/// TERESA — a terminal chat for people who live in one.
#[derive(Debug, Parser)]
#[command(name = "teresa", version, about)]
struct Args {
    /// Config file (default: ~/.config/teresa/config.toml).
    #[arg(short, long)]
    config: Option<PathBuf>,
    /// Server host, overriding the config for this run.
    #[arg(long)]
    server: Option<String>,
    /// Server port, overriding the config for this run.
    #[arg(long)]
    port: Option<u16>,
    /// Force TLS on or off for this run.
    #[arg(long)]
    tls: Option<bool>,
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Revoke the stored session and forget it.
    Logout,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // One TLS/CSPRNG backend, chosen before any network code runs.
    rustls::crypto::ring::default_provider()
        .install_default()
        .ok();

    let config_path = args
        .config
        .clone()
        .unwrap_or_else(ClientConfig::default_path);
    let mut config = ClientConfig::load_or_init(&config_path)?;
    if let Some(server) = args.server {
        config.server = server;
    }
    if let Some(port) = args.port {
        config.port = port;
    }
    if let Some(tls) = args.tls {
        config.tls = tls;
    }

    // Optional file logging; never stderr, which the TUI owns.
    if let Some(path) = &config.log_file {
        if let Ok(file) = std::fs::File::options()
            .create(true)
            .append(true)
            .open(path)
        {
            let subscriber = tracing_subscriber::fmt()
                .with_writer(file)
                .with_ansi(false)
                .finish();
            let _ = tracing::subscriber::set_global_default(subscriber);
        }
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("tokio runtime failed to start")?;

    match args.command {
        Some(CliCommand::Logout) => runtime.block_on(logout(&config)),
        None => {
            let session = runtime.block_on(ensure_session(&config))?;
            runtime.block_on(run(config, config_path, session))
        }
    }
}

// --- session bootstrap (plain terminal, before the TUI) --------------------

async fn logout(config: &ClientConfig) -> anyhow::Result<()> {
    let path = Session::default_path();
    match Session::load(&path, &config.server) {
        Some(session) => {
            AuthClient::new(config).logout(&session.token).await;
            Session::clear(&path)?;
            println!("session revoked. the wire forgets you.");
        }
        None => println!("no session stored for {}.", config.server),
    }
    Ok(())
}

/// Loads the stored session or walks the user through login / first-run
/// identity creation — animated when a real terminal is attached.
async fn ensure_session(config: &ClientConfig) -> anyhow::Result<Session> {
    let path = Session::default_path();
    if let Some(session) = Session::load(&path, &config.server) {
        return Ok(session);
    }

    let intro = Intro::new();
    intro.reveal_logo();
    intro.welcome(
        teresa_client::VERSION,
        &format!("{}:{}", config.server, config.port),
        config.tls,
    );

    let auth = AuthClient::new(config);

    // Real work behind the shimmer: prove the node is alive before asking a
    // human for credentials.
    if let Err(error) = intro.working("waking the node", auth.healthz()).await {
        match error {
            AuthError::Unreachable { server, source } => anyhow::bail!(
                "cannot reach {server}: {source}\n\
                 start the server (teresad) or point --server at one"
            ),
            other => anyhow::bail!("that node is not speaking TERESA: {other}"),
        }
    }

    intro.narrate("no identity found for this node.");
    println!();

    loop {
        let choice = prompt_line(&intro.prompt("[c]reate a new identity or [l]og in?"))?;
        let create = match choice.trim().to_lowercase().as_str() {
            "c" | "create" | "" => true,
            "l" | "login" | "log in" => false,
            _ => continue,
        };

        let username = prompt_line(&intro.prompt("username"))?;
        let password = prompt_password(&intro.prompt("password"))?;

        let result = if create {
            let display = prompt_line(&intro.prompt("display name (enter = username)"))?;
            let display = if display.trim().is_empty() {
                username.trim().to_owned()
            } else {
                display.trim().to_owned()
            };
            intro
                .working(
                    "forging identity",
                    auth.register(username.trim(), &password, &display),
                )
                .await
        } else {
            intro
                .working(
                    "verifying credentials",
                    auth.login(username.trim(), &password),
                )
                .await
        };

        match result {
            Ok(ok) => {
                let session = Session {
                    token: ok.token,
                    username: ok.user.username,
                    display_name: ok.user.display_name,
                    server: config.server.clone(),
                };
                session.store(&path)?;
                println!();
                intro.accepted("identity accepted — entering the grid");
                std::thread::sleep(std::time::Duration::from_millis(350));
                return Ok(session);
            }
            Err(AuthError::Unreachable { server, source }) => {
                anyhow::bail!(
                    "cannot reach {server}: {source}\n\
                     start the server (teresad) or point --server at one"
                );
            }
            Err(error) => intro.refused(&error.to_string()),
        }
    }
}

fn prompt_line(label: &str) -> anyhow::Result<String> {
    print!("{label}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}

/// Reads a password with echo replaced by dots, via raw mode.
fn prompt_password(label: &str) -> anyhow::Result<String> {
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    print!("{label}");
    std::io::stdout().flush()?;

    terminal::enable_raw_mode()?;
    let mut password = String::new();
    let result = loop {
        match term_event::read()? {
            TermEvent::Key(key) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Enter => break Ok(std::mem::take(&mut password)),
                KeyCode::Backspace => {
                    if password.pop().is_some() {
                        print!("\u{8} \u{8}");
                        std::io::stdout().flush()?;
                    }
                }
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    break Err(anyhow::anyhow!("cancelled"));
                }
                KeyCode::Char(c) => {
                    password.push(c);
                    print!("•");
                    std::io::stdout().flush()?;
                }
                _ => {}
            },
            _ => {}
        }
    };
    terminal::disable_raw_mode()?;
    println!();
    result
}

// --- the TUI proper --------------------------------------------------------

async fn run(config: ClientConfig, config_path: PathBuf, session: Session) -> anyhow::Result<()> {
    let truecolor = std::env::var("COLORTERM")
        .map(|v| v.contains("truecolor") || v.contains("24bit"))
        .unwrap_or(false);

    let mut app = App::new(config);
    if !truecolor {
        app.theme = app.theme.to_indexed();
    }

    // Event plumbing: one inbox, three producers.
    let (events_tx, mut events) = mpsc::channel::<Event>(256);

    // 1. Network supervisor (via a NetEvent → Event forwarder).
    let (net_tx, mut net_rx) = mpsc::channel::<NetEvent>(64);
    let net = ws::spawn(app.config.ws_url(), session.token.clone(), net_tx);
    {
        let events_tx = events_tx.clone();
        tokio::spawn(async move {
            while let Some(event) = net_rx.recv().await {
                if events_tx.send(Event::Net(event)).await.is_err() {
                    break;
                }
            }
        });
    }

    // 1b. File transfers report through their own pipe, forwarded likewise.
    let (transfer_tx, mut transfer_rx) = mpsc::channel::<TransferUpdate>(256);
    {
        let events_tx = events_tx.clone();
        tokio::spawn(async move {
            while let Some(update) = transfer_rx.recv().await {
                if events_tx.send(Event::Transfer(update)).await.is_err() {
                    break;
                }
            }
        });
    }
    let transfers = TransferClient::new(&app.config, &session.token);

    // 2. Terminal input, on a dedicated OS thread (crossterm reads block).
    {
        let events_tx = events_tx.clone();
        std::thread::spawn(move || loop {
            match term_event::read() {
                Ok(TermEvent::Key(key)) => {
                    // Windows delivers press+release; only presses count.
                    if key.kind != KeyEventKind::Release
                        && events_tx.blocking_send(Event::Key(key)).is_err()
                    {
                        return;
                    }
                }
                Ok(TermEvent::Paste(text)) => {
                    if events_tx.blocking_send(Event::Paste(text)).is_err() {
                        return;
                    }
                }
                Ok(TermEvent::Mouse(mouse)) => {
                    if events_tx.blocking_send(Event::Mouse(mouse)).is_err() {
                        return;
                    }
                }
                Ok(TermEvent::Resize(..)) => {
                    if events_tx.blocking_send(Event::Resize).is_err() {
                        return;
                    }
                }
                Ok(_) => {}
                Err(_) => return,
            }
        });
    }

    // 3. The animation heartbeat.
    {
        let events_tx = events_tx.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(TICK);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tick.tick().await;
                // A full inbox means the app is busy redrawing already;
                // dropping a tick is the correct behaviour.
                if events_tx.try_send(Event::Tick).is_err() && events_tx.is_closed() {
                    break;
                }
            }
        });
    }

    // Terminal up. ratatui::init installs panic hooks that restore the screen.
    let mut terminal = ratatui::init();
    let _ = execute!(
        std::io::stdout(),
        EnableMouseCapture,
        EnableBracketedPaste,
        SetTitle("✦ teresa")
    );

    let auth = AuthClient::new(&app.config);
    let mut title_flagged = false;
    let outcome = loop {
        let Some(event) = events.recv().await else {
            break Ok(());
        };
        update(&mut app, event);
        // Coalesce whatever else is queued before paying for a draw.
        while let Ok(extra) = events.try_recv() {
            update(&mut app, extra);
        }

        // Perform the reducer's requested effects.
        for effect in std::mem::take(&mut app.effects) {
            match effect {
                Effect::Send(frame) => {
                    // A full or closed pipe while offline: the supervisor
                    // already tells the user; dropping beats blocking the UI.
                    let _ = net.outgoing.try_send(frame);
                }
                Effect::PersistConfig => {
                    if let Err(error) = app.config.write(&config_path) {
                        tracing::warn!(%error, "could not persist config");
                    }
                    if !truecolor {
                        app.theme = app.theme.to_indexed();
                    }
                }
                Effect::ReconnectNow => {
                    let _ = net.reconnect_now.try_send(());
                }
                Effect::Bell => {
                    let mut out = std::io::stdout();
                    let _ = out.write_all(b"\x07");
                    let _ = out.flush();
                }
                Effect::Upload {
                    id,
                    room,
                    path,
                    zip,
                } => {
                    transfers.spawn_upload(id, room, path, zip, transfer_tx.clone());
                }
                Effect::Download { id, card } => {
                    transfers.spawn_download(id, card, transfer_tx.clone());
                }
                Effect::Logout => {
                    auth.logout(&session.token).await;
                    if let Err(error) = Session::clear(&Session::default_path()) {
                        tracing::warn!(%error, "could not clear session file");
                    }
                }
            }
        }

        if app.quit {
            break Ok(());
        }

        // Title flash tracks unseen activity, changed only on transitions.
        if app.config.title_flash && app.unseen_activity != title_flagged {
            title_flagged = app.unseen_activity;
            let title = if title_flagged {
                "● teresa"
            } else {
                "✦ teresa"
            };
            let _ = execute!(std::io::stdout(), SetTitle(title));
        }

        if app.dirty {
            app.dirty = false;
            if let Err(error) = terminal.draw(|frame| ui::render(frame, &app)) {
                break Err(error.into());
            }
        }
    };

    // Teardown, reverse order of setup.
    net.shutdown.cancel();
    let _ = execute!(
        std::io::stdout(),
        DisableMouseCapture,
        DisableBracketedPaste,
        SetTitle("")
    );
    ratatui::restore();
    println!("uplink closed.");
    outcome
}
