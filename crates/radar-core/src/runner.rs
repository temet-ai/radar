//! The process around the host: CLI, terminal, event loop and the effects the
//! host asks for (network work, files, browser, clipboard).

use std::io::Write;
use std::path::PathBuf;

use crate::design::theme::Tier;
use anyhow::{Context, Result};
use ratatui::crossterm::event::{self as ct, KeyEventKind};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::host::{Event, Host, HostEffect};
use crate::journal::{
    ActionDescriptor, ActionId, Origin, Outcome as JournalOutcome, Reason, SessionOrigin,
    StableCode,
};
use crate::journal_runtime::{JournalRecord, Recording, Sink};
use crate::log;
use crate::motion;
use crate::plugin::{CliExecution, Plugin};
use crate::select;
use crate::system::{self, Done, Env};
use crate::task_completion::{complete_task, tracked_event};
use crate::terminal_host;

use crate::time;

fn interaction_input(event: &Event) -> bool {
    match event {
        Event::Tracked { event, .. } => interaction_input(event),
        Event::Key(_) | Event::Mouse { .. } | Event::Paste(_) | Event::Intent(_) => true,
        _ => false,
    }
}

pub fn config_path() -> PathBuf {
    config_home().join("config.toml")
}

/// radar's config folder, the same on every platform: `$XDG_CONFIG_HOME/radar`
/// when that is set, else `~/.config/radar` (macOS's `dirs` would say
/// `~/Library/Application Support`). A file that only exists in the old
/// macOS folder is still read from there ([`resolve_config_home`]).
pub fn config_home() -> PathBuf {
    let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
    let legacy = dirs::config_dir().map(|d| d.join("radar"));
    resolve_config_home(
        xdg,
        dirs::home_dir(),
        legacy,
        &["config.toml", "icons"],
        |p| p.exists(),
    )
}

/// The folder for the config: `xdg/radar`, else `home/.config/radar`; but when
/// none of `leaves` exists there and one does in the `legacy` folder (the
/// old macOS location), that one.
pub fn resolve_config_home(
    xdg: Option<PathBuf>,
    home: Option<PathBuf>,
    legacy: Option<PathBuf>,
    leaves: &[&str],
    exists: impl Fn(&std::path::Path) -> bool,
) -> PathBuf {
    let new = match (xdg.filter(|p| !p.as_os_str().is_empty()), home) {
        (Some(x), _) => x.join("radar"),
        (None, Some(h)) => h.join(".config").join("radar"),
        (None, None) => PathBuf::from(".").join("radar"),
    };
    match legacy {
        Some(old)
            if old != new
                && !leaves.iter().any(|l| exists(&new.join(l)))
                && leaves.iter().any(|l| exists(&old.join(l))) =>
        {
            old
        }
        _ => new,
    }
}

/// The folders radar keeps its config, state and cache under, with the
/// platform's own ones too (macOS's `dirs` answers differently from XDG).
pub fn folder_bases() -> Vec<PathBuf> {
    let env = |k: &str| {
        std::env::var_os(k)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    let home = dirs::home_dir();
    let mut bases: Vec<PathBuf> = [
        env("XDG_CONFIG_HOME"),
        home.as_ref().map(|h| h.join(".config")),
        dirs::config_dir(),
        env("XDG_STATE_HOME"),
        home.as_ref().map(|h| h.join(".local").join("state")),
        dirs::state_dir(),
        env("XDG_CACHE_HOME"),
        dirs::cache_dir(),
    ]
    .into_iter()
    .flatten()
    .collect();
    bases.dedup();
    bases
}

/// What the host kept in a folder called `gg` before the rename. Other programs
/// are called gg too and share those folders, so only these names move, with
/// each part's old `<id>.json` cache.
const OWN_UNDER_GG: [&str; 13] = [
    "config.toml", "icons", "recipes", "journal", "cache.sqlite3", "cache.sqlite3-wal",
    "cache.sqlite3-shm", "gg.log", "gg.log.1", "runtime", "images", "reviews", "terminals",
];

/// Before the rename to radar these folders were called `gg`. Each of radar's
/// own entries in `<base>/gg` ([`OWN_UNDER_GG`], and `<id>.json` for each id in
/// `parts`) moves to `<base>/radar`, once,
/// unless that name is already there; anything else in `<base>/gg` stays. The
/// `gg` folder is removed when that leaves it empty. Returns each move tried.
pub fn move_folders_from_gg(bases: &[PathBuf], parts: &[&str]) -> Vec<(PathBuf, std::io::Result<()>)> {
    let caches: Vec<String> = parts.iter().map(|id| format!("{id}.json")).collect();
    let names: Vec<&str> = OWN_UNDER_GG.iter().copied().chain(caches.iter().map(String::as_str)).collect();
    let mut done: Vec<(PathBuf, std::io::Result<()>)> = Vec::new();
    for base in bases {
        let (old, new) = (base.join("gg"), base.join("radar"));
        if !old.is_dir() || done.iter().any(|(o, _)| o.parent() == Some(old.as_path())) {
            continue;
        }
        for name in &names {
            let (from, to) = (old.join(name), new.join(name));
            if from.exists() && !to.exists() {
                let moved = std::fs::create_dir_all(&new).and_then(|()| std::fs::rename(&from, &to));
                done.push((from, moved));
            }
        }
        // Removed only when empty: remove_dir refuses a folder with anything left in it.
        let _ = std::fs::remove_dir(&old);
    }
    done
}

/// What `[theme] icons = "auto"` means here, and the user's icon packs,
/// with what was wrong in them logged.
fn icons_here() {
    use crate::ui::icon;
    let auto = icon::detect(&|k| std::env::var(k).ok());
    icon::set_auto(auto);
    let (packs, warnings) = icon::load_dir(&icon::dir());
    for w in warnings {
        log::warn(format!("icons: {w}"));
    }
    log::info(format!(
        "icons: auto is {auto} here; packs of yours: {}",
        if packs.is_empty() {
            "none".to_string()
        } else {
            packs
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        }
    ));
    icon::set_packs(packs);
}

/// `radar icons list | export <pack> | path`.
fn icons_cli(host: &Host, m: &clap::ArgMatches) -> Result<()> {
    use crate::ui::icon::{self, Icon};
    match m.subcommand() {
        Some(("export", m)) => {
            let pack = m.get_one::<String>("pack").map_or("", String::as_str);
            match icon::export(pack, &icon::user_packs()) {
                Some(text) => print!("{text}"),
                None => anyhow::bail!(
                    "no icon pack is called {pack:?}; the packs are: {}",
                    icon::pack_names().join(", ")
                ),
            }
        }
        Some(("path", _)) => println!("{}", icon::dir().display()),
        _ => {
            let cfg = &host.colours.cfg;
            let (icons, warnings) =
                icon::resolve(&cfg.icons, &icon::user_packs(), &cfg.icon_overrides);
            println!(
                "pack: {}{}",
                icons.pack,
                if cfg.icons == "auto" { " (auto)" } else { "" }
            );
            let mut group = "";
            for &i in Icon::ALL {
                if i.group() != group {
                    group = i.group();
                    println!("\n{group}");
                }
                let r = icons.get(i);
                let glyph = if r.glyph.is_empty() {
                    crate::ui::pad("·", icons.width.max(1))
                } else {
                    r.glyph.clone()
                };
                let mine = if r.from == "[icons]" { "  (yours)" } else { "" };
                println!(
                    "  {glyph}  {}{}{mine}",
                    crate::ui::pad(i.name(), 18),
                    i.about()
                );
            }
            for w in warnings {
                eprintln!("icons: {w}");
            }
        }
    }
    Ok(())
}

/// Build the host from these plugins with config and caches loaded.
pub fn host(plugins: Vec<Box<dyn Plugin>>) -> Result<Host> {
    let mut host = Host::new(plugins, time::now_secs())?;
    let path = config_path();
    host.config_place = Some(place_of(&path, dirs::home_dir().as_deref()));
    let text = path
        .exists()
        .then(|| std::fs::read_to_string(&path))
        .transpose()?;
    match &text {
        Some(t) => log::info(format!("config {} ({} bytes)", path.display(), t.len())),
        None => log::info(format!("no config at {}: first run", path.display())),
    }
    if let Err(e) = host.load_config(text.as_deref()) {
        log::error(format!("config {} unreadable: {e:#}", path.display()));
        return Err(e).with_context(|| format!("reading {}", path.display()));
    }
    host.open_on_a_shown_screen();
    // RADAR_MODE (or the old RADAR_NO_WRITES switches) for this run.
    host.set_env_mode(crate::writes::env_mode(std::env::vars()));
    log::info(format!(
        "mode {}{}",
        host.mode().word(),
        host.mode_env
            .as_ref()
            .map(|(_, v)| format!(" (set by {v})"))
            .unwrap_or_default()
    ));
    restore_caches(&mut host);
    Ok(host)
}

/// A path as the user would write it: under `home`, from `~`.
pub fn place_of(path: &std::path::Path, home: Option<&std::path::Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// Each part's cache from radar's database (its old JSON file once), then the
/// writer that stores what they persist from here on.
fn restore_caches(host: &mut Host) {
    use crate::plugin_cache::{self, Restored};
    let started = std::time::Instant::now();
    let dir = plugin_cache::legacy_dir();
    let mut db = plugin_cache::open_default();
    let now_ms = time::now_secs().saturating_mul(1000);
    let ids: Vec<&str> = host.plugins.iter().map(|p| p.id()).collect();
    let restored = plugin_cache::restore_all(db.as_mut(), &dir, &ids, now_ms);
    for (p, restored) in host.plugins.iter_mut().zip(restored) {
        let id = p.id();
        let from = match restored {
            Restored::Missing => {
                log::info(format!("no cache for {id}"));
                continue;
            }
            Restored::Dropped(why) => {
                log::warn(format!("cache {id} unreadable, dropped: {why}"));
                continue;
            }
            Restored::Stored(v) => ("the database", v),
            Restored::Imported(v) => ("its old JSON file, now imported", v),
            Restored::File(v) => ("its JSON file", v),
        };
        log::info(format!("cache {id} restored from {}", from.0));
        p.restore(from.1);
    }
    log::info(format!(
        "caches restored in {} ms",
        started.elapsed().as_millis()
    ));
    plugin_cache::install(plugin_cache::Writer::start(db, dir));
}

pub use crate::ipc::Renderer;

/// radar's licence and the notices of what a release carries (scripts/notices), for `radar licenses`.
const LICENSE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../LICENSE"));
const THIRD_PARTY_NOTICES: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../THIRD-PARTY-NOTICES.md"));

/// `radar` with these plugins: the TUI, `radar config …`, or a plugin's commands.
/// The TUI is drawn by `default` unless `RADAR_RENDERER=ink|ratatui` names
/// the other (the one switch).
pub async fn main(plugins: Vec<Box<dyn Plugin>>, default: Renderer) -> Result<()> {
    let mut cli = clap::Command::new("radar")
        .about("A terminal workspace, built from the parts compiled into it")
        .version(env!("RADAR_BUILD"))
        .subcommand(
            clap::Command::new("config")
                .about("The config file")
                .subcommand_required(true)
                .subcommand(clap::Command::new("path").about("Print its path"))
                .subcommand(clap::Command::new("show").about("Print it as radar would write it")),
        )
        .subcommand(
            clap::Command::new("icons")
                .about("The icons: their names, packs, and where your packs go")
                .subcommand_required(true)
                .subcommand(
                    clap::Command::new("list")
                        .about("Every icon's name, with its glyph in the pack in use"),
                )
                .subcommand(
                    clap::Command::new("export")
                        .about("A whole pack as TOML, a start for one of your own")
                        .arg(clap::Arg::new("pack").required(true).value_name("PACK")),
                )
                .subcommand(clap::Command::new("path").about("The folder your packs go in")),
        )
        .subcommand(
            clap::Command::new("setup")
                .about("Set radar up: from the start, or one step on its own")
                .arg(
                    clap::Arg::new("step")
                        .value_name("STEP")
                        .help("scan, choose, look, local, ready, or a part's step; an unknown one lists them"),
                ),
        )
        .subcommand(clap::Command::new("terminal").about("Show the terminal layers radar detected"))
        .subcommand(clap::Command::new("licenses").about("Print radar's licence and the licences of what it carries"))
        .subcommand(crate::shot::command())
        .subcommand(crate::work_cli::command());
    for p in &plugins {
        if let Some(cmd) = p.cli() {
            cli = cli.subcommand(cmd.name(p.id()));
        }
    }
    let matches = match cli.try_get_matches() {
        Ok(matches) => matches,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            error.print()?;
            return Ok(());
        }
        Err(_) => anyhow::bail!("invalid command arguments; use radar --help"),
    };
    // `radar shot` draws sample data: no config, caches, log or action journal.
    if let Some(("shot", m)) = matches.subcommand() {
        return crate::shot::run(plugins, m);
    }
    if let Some(("work", m)) = matches.subcommand() {
        return work_cli(m).await;
    }
    if matches.subcommand_name() == Some("licenses") {
        // Read through a pager or `head`: a closed pipe ends it quietly.
        use std::io::Write;
        let _ = write!(std::io::stdout().lock(), "{LICENSE}\n{THIRD_PARTY_NOTICES}");
        return Ok(());
    }
    if matches.subcommand_name() == Some("terminal") {
        let stack = terminal_host::Stack::detect();
        println!("running in: {}", stack.description());
        println!("mouse: radar requests terminal mouse reporting while running");
        if stack.herdr {
            println!("right-click: {}", if stack.herdr_pane { "radar will ask Herdr to route this pane on startup" } else { "Herdr pane identity unavailable" });
        } else {
            println!("right-click: requested; terminal controls delivery");
        }
        return Ok(());
    }
    log::init(log::default_path());
    log::info(format!(
        "radar {} starting: {} with {}",
        env!("RADAR_BUILD"),
        matches.subcommand_name().unwrap_or("tui"),
        plugins
            .iter()
            .map(|p| p.id())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    // `radar setup [STEP]` is the TUI, opened on setup.
    let cli_command = matches.subcommand().is_some_and(|(name, _)| name != "setup");
    let mut recording = start_recording(if cli_command {
        SessionOrigin::Cli
    } else {
        SessionOrigin::Tui
    });
    let sink = recording.as_ref().map(|recording| recording.sink.clone());
    let action = sink.as_ref().map(|sink| {
        sink.begin(
            if cli_command {
                "session.cli"
            } else {
                "session.tui"
            },
            Origin::Host,
            None,
        )
    });
    record_outcome(
        sink.as_ref(),
        action.as_ref(),
        JournalOutcome::Started,
        None,
    );
    let result = async {
        icons_here();
        let mut host = host(plugins)?;
        let command = sink
            .as_ref()
            .filter(|_| cli_command)
            .map(|sink| {
                let mut descriptor = cli_descriptor(&matches, &host.plugins);
                descriptor.metadata.mode = Some(journal_mode(host.mode()));
                sink.begin_descriptor(descriptor, action.clone())
            });
        record_outcome(
            sink.as_ref(),
            command.as_ref(),
            JournalOutcome::Started,
            None,
        );
        let result = async {
            if let Some(("setup", m)) = matches.subcommand() {
                let step = m.get_one::<String>("step").map(String::as_str);
                host.open_setup(step).map_err(|why| anyhow::anyhow!(why))?;
            }
            match matches.subcommand() {
                None | Some(("setup", _)) => {
                    let renderer =
                        Renderer::chosen(default, std::env::var("RADAR_RENDERER").ok().as_deref());
                    let result = match renderer {
                        Renderer::Ratatui => run_tui_recorded(host, sink.clone()).await,
                        Renderer::Ink => run_ink_recorded(host, sink.clone()).await,
                    };
                    match &result {
                        Ok(()) => log::info("radar quit"),
                        Err(e) => log::error(format!("radar stopped: {e:#}")),
                    }
                    result
                }
                Some(("config", m)) => {
                    match m.subcommand_name() {
                        Some("path") => println!("{}", config_path().display()),
                        _ => print!("{}", host.config_text()),
                    }
                    Ok(())
                }
                Some(("icons", m)) => icons_cli(&host, m),
                Some((id, m)) => {
                    let plugin = host
                        .plugins
                        .iter()
                        .find(|p| p.id() == id)
                        .context("unknown command")?;
                    print!(
                        "{}",
                        run_plugin_cli_recorded(
                            plugin.as_ref(),
                            m,
                            sink.as_ref(),
                            command.as_ref(),
                            host.mode(),
                        )
                        .await?
                    );
                    Ok(())
                }
            }
        }
        .await;
        record_outcome(
            sink.as_ref(),
            command.as_ref(),
            if result.is_ok() {
                JournalOutcome::Succeeded
            } else {
                JournalOutcome::Failed
            },
            None,
        );
        result
    }
    .await;
    record_outcome(
        sink.as_ref(),
        action.as_ref(),
        if result.is_ok() {
            JournalOutcome::Succeeded
        } else {
            JournalOutcome::Failed
        },
        None,
    );
    // The parts' last caches are stored before the journal session ends.
    crate::plugin_cache::finish();
    finish_recording(&mut recording);
    result
}

/// Data queries are audited. Their worker captures a cut before its own event.
/// ID generation and help open neither the log nor the journal.
async fn work_cli(matches: &clap::ArgMatches) -> Result<()> {
    let (name, _) = matches
        .subcommand()
        .ok_or(crate::work_cli::Error::InvalidInput)?;
    if name == "id" {
        println!("{}", crate::journal::work_map::ChangeId::new().as_str());
        return Ok(());
    }
    let command = crate::work_cli::parse(name, std::io::stdin().lock())?;
    let applied = matches!(&command, crate::work_cli::Command::Apply(_));
    let mode = crate::work_cli::mode()?;
    log::init(log::default_path());
    log::info(command.code());
    // Explicit annotations must never continue without durable attribution.
    let mut recording =
        Recording::open(SessionOrigin::Cli).map_err(|_| crate::work_cli::Error::Unavailable)?;
    let response = recording.sink.command(command, mode).await;
    recording.finish();
    if let Some(warning) = recording.sink.take_warning() {
        eprintln!("radar: {warning}");
        log::warn(warning);
    }
    let response = response?;
    // No wrapper, pretty printing or newline may inflate a handoff page budget.
    crate::work_cli::write_response(std::io::stdout().lock(), &response.bytes, applied)?;
    Ok(())
}

fn journal_mode(mode: crate::writes::Mode) -> crate::journal::WriteMode {
    match mode {
        crate::writes::Mode::ReadOnly => crate::journal::WriteMode::ReadOnly,
        crate::writes::Mode::Write => crate::journal::WriteMode::Write,
        crate::writes::Mode::Edit => crate::journal::WriteMode::Edit,
    }
}

fn cli_descriptor(matches: &clap::ArgMatches, plugins: &[Box<dyn Plugin>]) -> ActionDescriptor {
    let origin = matches
        .subcommand_name()
        .and_then(|id| plugins.iter().find(|p| p.id() == id))
        .map_or(Origin::Host, |p| Origin::Plugin(p.id().into()));
    let mut descriptor = ActionDescriptor::new(
        if matches.subcommand().is_some() {
            "cli.command"
        } else {
            "session.tui"
        },
        origin,
    );
    // clap command names are declared vocabulary; argument values never enter.
    let mut names = Vec::new();
    let mut cursor = matches;
    while let Some((name, nested)) = cursor.subcommand() {
        names.push(name);
        cursor = nested;
    }
    if !names.is_empty() {
        descriptor.metadata.command = StableCode::try_new(&names.join(".")).ok();
    }
    descriptor
}

/// Only completed envelopes reach this boundary. These records' timestamps are
/// host receipt times, not provider request/response times. Do not synthesize a
/// Started event, duration or intermediate progress from the completion facts.
async fn run_plugin_cli_recorded(
    plugin: &dyn Plugin,
    matches: &clap::ArgMatches,
    sink: Option<&Sink>,
    command: Option<&ActionId>,
    mode: crate::writes::Mode,
) -> Result<String> {
    let CliExecution {
        result,
        observations,
    } = plugin.run_cli_observed(matches).await;
    if let (Some(sink), Some(command)) = (sink, command) {
        let (observations, omitted) = observations.into_parts();
        for observation in observations {
            let (mut descriptor, outcome) = observation.into_parts();
            // Origin and execution mode come from the dispatched host context,
            // never from a provider's claim about another part or the host.
            descriptor.origin = Origin::Plugin(plugin.id().into());
            descriptor.metadata.mode = Some(journal_mode(mode));
            let metadata = descriptor.metadata.clone();
            let child = sink.begin_descriptor(descriptor, Some(command.clone()));
            sink.submit(JournalRecord::Outcome {
                id: child,
                outcome,
                metadata,
            });
        }
        if omitted > 0 {
            let mut descriptor = ActionDescriptor::new("cli.observations.omitted", Origin::Host);
            descriptor.metadata.mode = Some(journal_mode(mode));
            descriptor.metadata.truncated = true;
            descriptor.metadata.count = Some(omitted.min(u32::MAX as u64) as u32);
            let metadata = descriptor.metadata.clone();
            let child = sink.begin_descriptor(descriptor, Some(command.clone()));
            sink.submit(JournalRecord::Outcome {
                id: child,
                outcome: JournalOutcome::Partial,
                metadata,
            });
            // Each omitted envelope entry would otherwise produce a requested
            // and terminal record. Also mark the session's trace as incomplete.
            sink.submit(JournalRecord::Gap {
                dropped: omitted.saturating_mul(2),
            });
        }
    }
    // A command error does not erase its preceding explicit provider facts.
    // Recording disabled or unavailable never changes output or failure behavior.
    result
}

/// The screen's recording: the journal opens on its writer's thread, so the
/// first frame does not wait for its checks.
fn start_recording(origin: SessionOrigin) -> Option<Recording> {
    match Recording::open_in_background(origin) {
        Ok(recording) => Some(recording),
        Err(_) => {
            let warning = "action recording unavailable: could not open private journal storage";
            eprintln!("radar: {warning}");
            log::warn(warning);
            None
        }
    }
}

fn finish_recording(recording: &mut Option<Recording>) {
    if let Some(recording) = recording {
        recording.finish();
        if let Some(warning) = recording.sink.take_warning() {
            eprintln!("radar: {warning}");
            log::warn(warning);
        }
    }
}

pub async fn run_tui(host: Host) -> Result<()> {
    let mut recording = start_recording(SessionOrigin::Tui);
    let result = run_tui_recorded(host, recording.as_ref().map(|r| r.sink.clone())).await;
    finish_recording(&mut recording);
    result
}

/// The TUI drawn by the Ink renderer in a child process; where it cannot
/// run, one line says why and the Ratatui renderer runs instead.
async fn run_ink_recorded(mut host: Host, journal: Option<Sink>) -> Result<()> {
    use crate::ipc::supervisor::{self, Ended, Supervisor, TtyModes};
    let exe = std::env::current_exe().ok();
    let (program, args) = match supervisor::find_renderer(&|k| std::env::var(k).ok(), exe.as_deref()) {
        Ok(found) => found,
        Err(why) => {
            eprintln!("radar: {why}; using the Ratatui renderer");
            log::warn(format!("RADAR_RENDERER=ink: {why}; Ratatui runs"));
            return run_tui_recorded(host, journal).await;
        }
    };
    log::info(format!(
        "Ink renderer: {} {}",
        program.display(),
        args.iter().map(|a| a.to_string_lossy()).collect::<Vec<_>>().join(" ")
    ));
    host.terminal = Some(terminal_host::State::new(terminal_host::Stack::detect()));
    if journal.is_none() {
        host.notify_error("action recording unavailable for this session");
    }
    let (columns, rows) = ratatui::crossterm::terminal::size().unwrap_or((80, 24));
    let (tx, mut rx) = unbounded_channel::<Event>();
    let ticks = tx.clone();
    tokio::spawn(async move {
        let mut every = tokio::time::interval(std::time::Duration::from_millis(200));
        loop {
            every.tick().await;
            if ticks.send(Event::Tick(time::now_secs())).is_err() {
                break;
            }
        }
    });
    // A part's news (an agent's streamed text) as it comes, not at the next tick.
    tokio::spawn(crate::wake::pump(host.wake_hub(), tx.clone(), crate::wake::COALESCE, || Event::Wake(time::now_secs())));
    let mut bridge = crate::ipc::Bridge::new(host, columns, rows);
    bridge.show_splash();
    // The look is resolved at the tier the Ratatui runner would draw at.
    bridge.set_tier(Tier::detect());
    let socket = supervisor::socket_path(system::state_home(), std::process::id());
    let mut sup = Supervisor::new(
        Box::new(move || {
            let mut c = tokio::process::Command::new(&program);
            c.args(&args);
            c
        }),
        socket,
        std::io::stdout(),
    );
    // Raw mode is the renderer's while it runs; these settings put the
    // terminal back after it.
    sup.modes = TtyModes::save();
    let fx_tx = tx.clone();
    let sink = journal.clone();
    let mut perform_fx = |effect| perform(effect, &fx_tx, sink.as_ref());
    let clip = journal.clone();
    // The clipboard: the renderer owns the terminal, so it writes OSC 52.
    let mut filter = |event: Event, bridge: &mut crate::ipc::Bridge| -> Option<Event> {
        // As the Ratatui runner does: a recording that failed (a journal that
        // could not be opened, a write that failed) says so in the status line.
        if let Some(warning) = clip.as_ref().and_then(Sink::take_warning) {
            log::warn(warning);
            bridge.host_mut().notify_error(warning);
        }
        let (plain, action, semantic) = match event {
            Event::Tracked { action, semantic, event } => (*event, Some(action), semantic),
            event => (event, None, None),
        };
        let event = match plain {
            Event::ToClipboard { text, note } => {
                let sent = bridge.clipboard(text);
                // OSC52 has no acknowledgement from the receiving clipboard.
                record_outcome(
                    clip.as_ref(),
                    action.as_ref(),
                    if sent { JournalOutcome::Unknown } else { JournalOutcome::Failed },
                    Some(Reason::Transport),
                );
                if note.is_empty() {
                    return None;
                }
                Event::Notice(note)
            }
            other => other,
        };
        Some(tracked_event(event, action, semantic))
    };
    let ended = sup.run(&mut bridge, &mut rx, &mut perform_fx, &mut filter).await?;
    match ended {
        Ended::Quit => Ok(()),
        Ended::FallBack(why) => {
            // The words are in the status line already (the supervisor
            // put them there); stderr gets them too.
            supervisor::say_fall_back(&mut std::io::stderr(), &why);
            run_tui_recorded(bridge.into_host(), journal).await
        }
    }
}

async fn run_tui_recorded(mut host: Host, journal: Option<Sink>) -> Result<()> {
    let stack = terminal_host::Stack::detect();
    host.terminal = Some(terminal_host::State::new(stack.clone()));
    if journal.is_some() {
        host.enable_journal();
    } else {
        host.notify_error("action recording unavailable for this session");
    }
    let detected = Tier::detect();
    let mut theme = host.build_theme(detected);
    let mut theme_cfg = host.colours.cfg.clone();
    // Motion: effects over each frame, run on a frame tick only while one
    // runs (crate::motion).
    let ssh = Env::detect().ssh;
    let motion_level = |cfg: &crate::theme::ThemeConfig, theme: &crate::theme::Theme| {
        motion::effective(motion::Level::parse(&cfg.motion), theme.tier(), ssh)
    };
    let mut motion = motion::Engine::new(
        motion_level(&theme_cfg, &theme),
        Box::new(motion::SystemClock::default()),
    );
    host.set_motion_enabled(motion.level() == motion::Level::Full);
    log::info(format!(
        "motion {}{}",
        motion.level().word(),
        motion::held_back(theme.tier(), ssh).map_or(String::new(), |why| format!(" ({why})"))
    ));
    // The startup splash, over the first frames (crate::splash).
    let mut splash = crate::splash::Splash::new(crate::splash::Setting::parse(&theme_cfg.splash), motion.level());
    splash.roll(motion.level() == crate::motion::Level::Full, theme.icons().width == 2, host.start_facts());
    let splash_clock = std::time::Instant::now();
    let notice_clock_base = host.notices.now_ms();
    let (tx, mut rx) = unbounded_channel::<Event>();
    // The left button, kept by the runner: a press that does not move is a
    // click for the host, a drag selects text (crate::select).
    let (pointer_tx, mut pointer) = unbounded_channel::<Pointer>();

    // The terminal, drawn through the backend that writes links as OSC 8
    // hyperlinks, when they are wanted here.
    ratatui::crossterm::terminal::enable_raw_mode()?;
    ratatui::crossterm::execute!(
        std::io::stdout(),
        ratatui::crossterm::terminal::EnterAlternateScreen
    )?;
    let mut terminal = ratatui::Terminal::new(crate::ui::link::Links::new(std::io::stdout()))?;
    // Pictures: the terminal is asked which graphics it draws, before the
    // keys' thread reads what it answers.
    let picked = crate::ui::image::detect(&theme_cfg.images, &|k| std::env::var(k).ok());
    let pictures = |cfg: &crate::theme::ThemeConfig| match (&picked, cfg.images.as_str()) {
        (Some(p), on) if on != "off" => {
            crate::ui::image::start(
                p.clone(),
                system::cache_home().map(|d| d.join("radar").join("images")),
            );
            log::info(format!("images on: {:?}", p.protocol_type()));
        }
        _ => {
            crate::ui::image::stop();
            log::info(format!("images off ({})", cfg.images));
        }
    };
    pictures(&theme_cfg);

    // Keys on a thread of their own: nothing blocks the loop (TUI-P5).
    let keys = tx.clone();
    std::thread::spawn(move || {
        loop {
            let event = match ct::read() {
                Ok(ct::Event::Key(k)) if k.kind == KeyEventKind::Press => Event::Key(k),
                Ok(ct::Event::Resize(..)) => Event::Tick(time::now_secs()),
                Ok(ct::Event::Paste(text)) => Event::Paste(text),
                // The window lost focus: the pointer is somewhere else.
                Ok(ct::Event::FocusLost) => {
                    if pointer_tx.send(Pointer::Gone).is_err() {
                        break;
                    }
                    continue;
                }
                Ok(ct::Event::Mouse(m)) => {
                    use crate::host::MouseKind;
                    // What the terminal (or a multiplexer between) sent: a
                    // click is a press then a release, so both must arrive.
                    if !matches!(m.kind, ct::MouseEventKind::Moved) {
                        log::debug(|| {
                            format!(
                                "mouse {:?} at {},{} {:?}",
                                m.kind, m.column, m.row, m.modifiers
                            )
                        });
                    }
                    let at = ratatui::layout::Position::new(m.column, m.row);
                    // The pointer moved with no button down: what is under
                    // it is lit (the loop keeps only the latest move).
                    if matches!(m.kind, ct::MouseEventKind::Moved) {
                        if pointer_tx.send(Pointer::Move(at)).is_err() {
                            break;
                        }
                        continue;
                    }
                    let left = ct::MouseButton::Left;
                    let kind = match m.kind {
                        ct::MouseEventKind::Down(ct::MouseButton::Right) => MouseKind::Context,
                        ct::MouseEventKind::ScrollUp => MouseKind::ScrollUp,
                        ct::MouseEventKind::ScrollDown => MouseKind::ScrollDown,
                        ct::MouseEventKind::Down(b)
                        | ct::MouseEventKind::Drag(b)
                        | ct::MouseEventKind::Up(b)
                            if b == left =>
                        {
                            let p = match m.kind {
                                ct::MouseEventKind::Down(_) => Pointer::Down(at, now_ms()),
                                ct::MouseEventKind::Drag(_) => Pointer::Drag(at),
                                _ => Pointer::Up,
                            };
                            if pointer_tx.send(p).is_err() {
                                break;
                            }
                            continue;
                        }
                        // Other buttons: nothing.
                        _ => continue,
                    };
                    Event::Mouse {
                        kind,
                        column: m.column,
                        row: m.row,
                        at_ms: now_ms(),
                    }
                }
                Ok(_) => continue,
                Err(_) => break,
            };
            if keys.send(event).is_err() {
                break;
            }
        }
    });
    let ticks = tx.clone();
    tokio::spawn(async move {
        // Fast enough for a smooth spinner; plugins decide for themselves
        // when a tick is due for real work.
        let mut every = tokio::time::interval(std::time::Duration::from_millis(200));
        loop {
            every.tick().await;
            if ticks.send(Event::Tick(time::now_secs())).is_err() {
                break;
            }
        }
    });
    // A part's news (an agent's streamed text) as it comes, not at the next tick.
    tokio::spawn(crate::wake::pump(host.wake_hub(), tx.clone(), crate::wake::COALESCE, || Event::Wake(time::now_secs())));

    let links =|cfg: &crate::theme::ThemeConfig| {
        let on = crate::ui::link::wanted(&cfg.hyperlinks, &|k| std::env::var(k).ok());
        crate::ui::link::set(on);
        log::info(format!(
            "hyperlinks {} ({})",
            if on { "on" } else { "off" },
            cfg.hyperlinks
        ));
    };
    links(&theme_cfg);
    let pointers = |cfg: &crate::theme::ThemeConfig| {
        let on = crate::ui::pointer::wanted(&cfg.pointer, &|k| std::env::var(k).ok());
        crate::ui::pointer::set(on);
        log::info(format!(
            "mouse pointer shapes {} ({})",
            if on { "on" } else { "off" },
            cfg.pointer
        ));
    };
    pointers(&theme_cfg);
    // A panic gives the terminal back first (ratatui's own hook, which runs
    // after this one, restores it too), and lands in the log with where it
    // happened: nobody can read it on a screen that is being redrawn.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_pointer();
        let _ = ratatui::crossterm::execute!(
            std::io::stdout(),
            ratatui::crossterm::event::DisableMouseCapture,
            ratatui::crossterm::event::DisableFocusChange,
            ratatui::crossterm::event::DisableBracketedPaste
        );
        ratatui::restore();
        log::error(format!(
            "panic: {info}\n{}",
            std::backtrace::Backtrace::force_capture()
        ));
        eprintln!(
            "radar stopped unexpectedly; details in {}",
            log::default_path().display()
        );
        previous(info);
    }));
    // Pastes arrive as one event, not as keystrokes that could trigger keys.
    // The mouse: clicks, the wheel, and where the pointer is (crossterm's
    // capture asks for every move, 1003). Focus: the pointer left.
    let _ = ratatui::crossterm::execute!(
        std::io::stdout(),
        ratatui::crossterm::event::EnableBracketedPaste,
        ratatui::crossterm::event::EnableMouseCapture,
        ratatui::crossterm::event::EnableFocusChange
    );
    log::info(format!("terminal layers: {}", stack.description()));
    if stack.herdr_pane {
        let reply = tx.clone();
        tokio::spawn(async move {
            let result = terminal_host::route_herdr_right_click().await;
            match &result {
                Ok(()) => log::info("Herdr right-clicks routed to this pane"),
                Err(error) => log::warn(error),
            }
            let _ = reply.send(Event::TerminalRoute(result));
        });
    }
    // The left button, the selection it makes and the presses counted.
    let mut gesture = select::Gesture::default();
    let mut notice_gesture = NoticeGesture::default();
    let mut detail_gesture = DetailGesture::default();
    let bypass = select::bypass_key(std::env::var("TERM_PROGRAM").ok().as_deref());
    let mut shown = ratatui::buffer::Buffer::empty(ratatui::layout::Rect::ZERO);
    let mut shown_area = shown.area;
    // The pointer's shape as last sent, and a pointer event read while
    // moves were being gathered, to handle next.
    let mut shape = crate::ui::pointer::Shape::Default;
    let mut held: Option<Pointer> = None;
    let mut redraw = true;
    // The splash's picture is on the screen ([`crate::splash::picture`]).
    let mut pictured = false;
    let result = async {
        loop {
            host.set_ui_clock_ms(notice_clock_base.saturating_add(splash_clock.elapsed().as_millis() as u64));
            host.set_notice_motion_enabled(motion.level() != motion::Level::Off);
            if redraw {
                let started = std::time::Instant::now();
                // The splash: its frame now, or the hand-off to the first
                // screen, which fades in with the screen cue.
                let over = match splash.step(splash_clock.elapsed(), true, &host.splash_parts(theme.icons())) {
                    crate::splash::Step::Show(scene) => {
                        let size = terminal.size().unwrap_or_default();
                        let frame = crate::splash::frame(&theme, &scene, size.width, size.height);
                        if frame.is_none() {
                            splash.input();
                        }
                        frame
                    }
                    crate::splash::Step::HandOff => {
                        motion.cue(motion::Cue::Screen, &theme);
                        None
                    }
                    crate::splash::Step::Hidden => None,
                };
                // Where the terminal shows kitty's pictures, the scope in
                // pixels over its cells (written after the frame).
                let picture = over
                    .as_ref()
                    .zip(crate::ui::image::kitty_cell())
                    .and_then(|(frame, cell)| crate::splash::picture(frame, cell));
                terminal.draw(|f| {
                    crate::shell::render(&host, &theme, f);
                    // What is on screen before the selection is marked: the
                    // text a selection copies.
                    shown = f.buffer_mut().clone();
                    if over.is_some() { motion::frames::context(None, None); }
                    motion.draw(f.buffer_mut(), &theme);
                    if over.is_none() { crate::shell::render_notices(&host, &theme, f.buffer_mut()); }
                    // Over everything: the screen under it keeps its click
                    // targets, so a click acts on it.
                    if let Some(frame) = &over {
                        crate::splash::draw(frame, f.buffer_mut());
                        if let Some(pic) = &picture {
                            crate::splash::clear_under(pic, f.buffer_mut());
                        }
                    }
                    if let Some(s) = &gesture.selection {
                        select::paint(f.buffer_mut(), s, select::style(&theme));
                    }
                })?;
                // The splash's picture over the frame just drawn; once the
                // splash goes, the picture goes with it.
                match &picture {
                    Some(pic) => {
                        let _ = terminal.backend_mut().write_all(crate::splash::kitty_at(pic).as_bytes());
                        let _ = terminal.backend_mut().flush();
                        pictured = true;
                    }
                    None if pictured && over.is_none() => {
                        let _ = terminal.backend_mut().write_all(crate::splash::kitty_hide().as_bytes());
                        let _ = terminal.backend_mut().flush();
                        pictured = false;
                    }
                    None => {}
                }
                // The screen changed size: what was selected is not there.
                if shown.area != shown_area {
                    shown_area = shown.area;
                    detail_gesture.cancel(&mut gesture);
                    gesture = select::Gesture::default();
                    host.selecting = None;
                }
                if let Some(why) = motion.drew_in(started.elapsed()) {
                    host.set_motion_enabled(false);
                    log::info(why);
                    let _ = tx.send(Event::Notice(why.into()));
                }
            }
            redraw = true;
            // The pointer: a hand over what a click acts on, a text cursor
            // while a drag selects.
            let want = if gesture.selecting() {
                crate::ui::pointer::Shape::Text
            } else {
                host.pointer_shape()
            };
            if want != shape && crate::ui::pointer::on() {
                let mut out = std::io::stdout();
                let _ = out.write_all(crate::ui::pointer::sequence(want).as_bytes());
                let _ = out.flush();
                shape = want;
            }
            // A pointer event read while moves were gathered goes first.
            let input = match held.take() {
                Some(p) => Input::Pointer(p),
                None => tokio::select! {
                    // Only while an effect runs: redraw, then wait again.
                    () = frame_tick(motion.next_frame().or(splash.next_frame()).or_else(|| (host.notice_frame_needed(shown_area) && !splash.active()).then_some(motion::FRAME))) => continue,
                    e = rx.recv() => match e {
                        Some(e) => Input::Event(e),
                        None => break,
                    },
                    p = pointer.recv() => match p {
                        Some(p) => Input::Pointer(p),
                        None => break,
                    },
                },
            };
            host.set_ui_clock_ms(notice_clock_base.saturating_add(splash_clock.elapsed().as_millis() as u64));
            if let Input::Event(Event::Key(key)) = &input { host.notice_input(key.code == ct::KeyCode::Esc); }
            // Cancel before local selection handlers or clipboard conversion can
            // consume this input. Pointer motion alone remains decorative.
            let actionable = match &input {
                Input::Event(event) => interaction_input(event),
                Input::Pointer(Pointer::Down(..) | Pointer::Drag(..) | Pointer::Up) => true,
                _ => false,
            };
            if !actionable && matches!(&input, Input::Event(_)) { motion.background(); }
            if actionable {
                motion.input();
                splash.input();
                if let Some(line) = splash.take_early_line() { host.show_start_line(line); }
            }
            let mut event = match input {
                Input::Event(e) => e,
                Input::Pointer(p) => {
                    match p {
                        // Only the latest of the moves waiting counts, and
                        // the screen is drawn again only when what is lit
                        // changed.
                        Pointer::Move(mut at) => {
                            while let Ok(next) = pointer.try_recv() {
                                match next {
                                    Pointer::Move(n) => at = n,
                                    other => {
                                        held = Some(other);
                                        break;
                                    }
                                }
                            }
                            redraw = host.hover(Some(at));
                            continue;
                        }
                        Pointer::Gone => {
                            detail_gesture.cancel(&mut gesture);
                            host.selecting = gesture.selecting().then_some(bypass);
                            redraw = host.hover(None);
                            continue;
                        }
                        Pointer::Down(at, ms) => {
                            let hit = host.hit_at(at.x, at.y);
                            // A new press supersedes an unfinished control gesture.
                            detail_gesture.cancel(&mut gesture);
                            if notice_gesture.press(hit.clone()) {
                                gesture.clear();
                                host.selecting = None;
                                continue;
                            }
                            if let Some(intent) = detail_gesture.press(hit.as_ref(), &mut gesture) {
                                host.selecting = None;
                                Event::Intent(intent)
                            } else {
                                gesture.down(at, ms, &host.frontend.selectable());
                                continue;
                            }
                        }
                        Pointer::Drag(head) => {
                            if notice_gesture.owns_pointer() || detail_gesture.owns_pointer() { continue; }
                            gesture.drag(head);
                            // While a drag selects, the status line names the
                            // key that selects with the terminal instead.
                            host.selecting = gesture.selecting().then_some(bypass);
                            continue;
                        }
                        Pointer::Up => {
                            host.selecting = None;
                            if detail_gesture.release() { continue; }
                            if let Some(intent) = notice_gesture.release() { Event::Intent(intent) }
                            else { match gesture.up(&shown) {
                                select::Released::Copy(text) => {
                                    Event::Intent(crate::view::Intent::Copy { text })
                                }
                                select::Released::Click(at, at_ms) => {
                                    // Only a physical Down can activate a foot. A redraw
                                    // may have put one beneath an ordinary row's old press.
                                    let Some(event) = DetailGesture::row_release(
                                        at, at_ms, host.hit_at(at.x, at.y).as_ref(),
                                    ) else { continue; };
                                    event
                                },
                                select::Released::Nothing => continue,
                            } }
                        }
                    }
                }
            };
            // With a selection showing: shift+arrows move its head, ctrl+c
            // copies it again, any other key (or the wheel) ends it.
            match &event {
                Event::Key(k) => match gesture.key(k, host.capturing() || host.menu.is_some(), &shown) {
                    select::Keyed::Used => continue,
                    select::Keyed::Copy(text) => {
                        event = Event::Intent(crate::view::Intent::Copy { text });
                    }
                    select::Keyed::Pass => {}
                },
                Event::Mouse { kind, .. } if *kind != crate::host::MouseKind::Press => gesture.clear(),
                _ => {}
            }
            // Unwrap before handling terminal clipboard fallbacks; the
            // reducer still receives the same originating action afterward.
            let (plain, action, semantic) = match event {
                Event::Tracked {
                    action,
                    semantic,
                    event,
                } => (*event, Some(action), semantic),
                event => (event, None, None),
            };
            event = plain;
            // Only this loop writes to the terminal, between frames.
            if let Event::ToClipboard { text, note } = event {
                let mut out = std::io::stdout();
                let written = out
                    .write_all(system::osc52(&text, Env::detect().tmux).as_bytes())
                    .and_then(|_| out.flush());
                // OSC52 has no acknowledgement from the receiving clipboard.
                record_outcome(
                    journal.as_ref(),
                    action.as_ref(),
                    if written.is_ok() {
                        JournalOutcome::Unknown
                    } else {
                        JournalOutcome::Failed
                    },
                    Some(Reason::Transport),
                );
                if note.is_empty() {
                    continue;
                }
                event = Event::Notice(note);
            }
            let event = tracked_event(event, action, semantic);
            for effect in host.update(event) {
                perform(effect, &tx, journal.as_ref());
            }
            if let Some(warning) = journal.as_ref().and_then(Sink::take_warning) {
                log::warn(warning);
                host.notify_error(warning);
            }
            // Theme edits apply on the next frame, before they are saved.
            if host.colours.cfg != theme_cfg {
                if host.colours.cfg.hyperlinks != theme_cfg.hyperlinks {
                    links(&host.colours.cfg);
                }
                if host.colours.cfg.images != theme_cfg.images {
                    pictures(&host.colours.cfg);
                }
                if host.colours.cfg.pointer != theme_cfg.pointer {
                    if crate::ui::pointer::on() {
                        restore_pointer();
                        shape = crate::ui::pointer::Shape::Default;
                    }
                    pointers(&host.colours.cfg);
                }
                theme_cfg = host.colours.cfg.clone();
                theme = host.build_theme(detected);
                motion.set_level(motion_level(&theme_cfg, &theme));
                host.set_motion_enabled(motion.level() == motion::Level::Full);
                splash.set_level(motion.level());
                splash.set_setting(crate::splash::Setting::parse(&theme_cfg.splash));
            }
            for cue in host.take_cues() {
                motion.cue(cue, &theme);
            }
            if host.quit {
                break;
            }
        }
        anyhow::Ok(())
    }
    .await;
    restore_pointer();
    let _ = ratatui::crossterm::execute!(
        std::io::stdout(),
        ratatui::crossterm::event::DisableMouseCapture,
        ratatui::crossterm::event::DisableFocusChange,
        ratatui::crossterm::event::DisableBracketedPaste
    );
    ratatui::restore();
    result
}

/// The terminal's own pointer back, when radar changed it.
fn restore_pointer() {
    if crate::ui::pointer::on() {
        let mut out = std::io::stdout();
        let _ = out
            .write_all(crate::ui::pointer::sequence(crate::ui::pointer::Shape::Default).as_bytes());
        let _ = out.flush();
    }
}

/// What the loop waits for: the host's events, or the mouse.
/// Captured notice ownership survives expiry, replacement and drag until release.
#[derive(Default)]
struct NoticeGesture { captured: Option<crate::view::Intent> }
impl NoticeGesture {
    fn press(&mut self, hit: Option<crate::host::Hit>) -> bool {
        use crate::host::{Hit, Host};
        use crate::view::{Intent, Target};
        self.captured = match hit {
            Some(Hit::Notice(id)) => Some(Intent::DismissNotice { id }),
            Some(Hit::NoticeHistoryOpen) => Some(Intent::Activate { target: Target::new("chrome:notices:open") }),
            Some(Hit::NoticeHistory(id)) => Some(Intent::Activate { target: Target::new(format!("chrome:notices:id:{id}")) }),
            Some(Hit::NoticeHistoryKey(key)) => Host::notice_history_key_target(key).map(|target| Intent::Activate { target }),
            _ => None,
        };
        self.owns_pointer()
    }
    fn owns_pointer(&self) -> bool { self.captured.is_some() }
    fn release(&mut self) -> Option<crate::view::Intent> { self.captured.take() }
}

/// Foot controls activate at physical Down, while their exact displayed target
/// is still under the pointer. Drag/Up cannot re-hit a replacement frame.
#[derive(Default)]
struct DetailGesture { pressed: bool }

impl DetailGesture {
    fn target(hit: Option<&crate::host::Hit>) -> Option<crate::view::Target> {
        match hit {
            Some(crate::host::Hit::Plugin { id, .. })
                if id.starts_with(crate::view::detail::TARGET_PREFIX) => Some(crate::view::Target::new(id.clone())),
            _ => None,
        }
    }

    fn press(&mut self, hit: Option<&crate::host::Hit>, gesture: &mut select::Gesture) -> Option<crate::view::Intent> {
        let target = Self::target(hit)?;
        // `clear` leaves pending presses and multi-click history behind.
        *gesture = select::Gesture::default();
        self.pressed = true;
        Some(crate::view::Intent::Activate { target })
    }

    fn owns_pointer(&self) -> bool { self.pressed }
    fn release(&mut self) -> bool { std::mem::take(&mut self.pressed) }

    fn cancel(&mut self, gesture: &mut select::Gesture) {
        if self.release() { *gesture = select::Gesture::default(); }
    }

    fn row_release(at: ratatui::layout::Position, at_ms: u64, hit: Option<&crate::host::Hit>) -> Option<Event> {
        if Self::target(hit).is_some() { return None; }
        Some(Event::Mouse { kind: crate::host::MouseKind::Press, column: at.x, row: at.y, at_ms })
    }
}

enum Input {
    Event(Event),
    Pointer(Pointer),
}

/// The mouse, as the runner keeps it: the left button, and where the
/// pointer is.
enum Pointer {
    Down(ratatui::layout::Position, u64),
    Drag(ratatui::layout::Position),
    Up,
    /// Moved with no button down.
    Move(ratatui::layout::Position),
    /// Left the window (the terminal lost focus).
    Gone,
}

/// The frame tick: after `every` while motion runs; never when it does not,
/// so an idle radar draws only on events.
async fn frame_tick(every: Option<std::time::Duration>) {
    match every {
        Some(d) => tokio::time::sleep(d).await,
        None => std::future::pending().await,
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn record_outcome(
    journal: Option<&Sink>,
    action: Option<&ActionId>,
    outcome: JournalOutcome,
    reason: Option<Reason>,
) {
    if let (Some(journal), Some(action)) = (journal, action) {
        journal.outcome(action, outcome, reason);
    }
}

fn open_web(
    env: &Env,
    url: String,
    tx: &UnboundedSender<Event>,
    journal: Option<&Sink>,
    action: Option<ActionId>,
    semantic: Option<ActionId>,
) {
    let ways = system::open_ways(env, &url);
    let done = system::try_ways(&ways, None);
    match &done {
        Ok(Done::Ran { .. }) => record_outcome(journal, action.as_ref(), JournalOutcome::Succeeded, None),
        Err(_) => record_outcome(journal, action.as_ref(), JournalOutcome::Failed, None),
        Ok(Done::UseTerminal { .. }) => {}
    }
    let event = match done {
        Ok(Done::Ran { .. }) => Event::Notice("opened in your browser".into()),
        Ok(Done::UseTerminal { why }) => Event::ToClipboard {
            note: if env.ssh {
                "on ssh, the link is on your own clipboard: paste it in your browser".into()
            } else if why.is_empty() {
                "no browser to open here: the link is on your clipboard".into()
            } else {
                format!("could not open it ({why}): the link is on your clipboard")
            },
            text: url,
        },
        Err(e) => Event::ErrorNotice(format!("could not open the link: {e}")),
    };
    let _ = tx.send(tracked_event(event, action, semantic));
}

/// What the status line says about text copied from the screen: how many
/// characters, and the way that took them (a program's name, or `terminal`
/// for OSC 52).
fn copied_note(characters: usize, way: &str) -> String {
    format!("copied {characters} characters \u{b7} {way}")
}

/// Put `text` on the clipboard in the first way that works. `report` says
/// the user selected it from the screen, so the status line names the way;
/// a copy a part asked for says what it copied itself.
fn copy_text(
    text: String,
    report: bool,
    tx: &UnboundedSender<Event>,
    journal: Option<&Sink>,
    action: Option<ActionId>,
    semantic: Option<ActionId>,
) {
    let ways = system::copy_ways(&Env::detect());
    let tx = tx.clone();
    let journal = journal.cloned();
    std::thread::spawn(move || {
        copy_through(&ways, text, report, &tx, journal.as_ref(), action, semantic)
    });
}

/// [`copy_text`] with the ways chosen: runs them in order (it blocks while a
/// program runs) and sends what the status line should say.
fn copy_through(
    ways: &[system::Way],
    text: String,
    report: bool,
    tx: &UnboundedSender<Event>,
    journal: Option<&Sink>,
    action: Option<ActionId>,
    semantic: Option<ActionId>,
) {
    let n = text.chars().count();
    match system::try_ways(ways, Some(&text)) {
        Ok(Done::Ran { way }) => {
            record_outcome(journal, action.as_ref(), JournalOutcome::Succeeded, None);
            if report {
                let _ = tx.send(tracked_event(
                    Event::Notice(copied_note(n, &way)),
                    action,
                    semantic,
                ));
            }
        }
        Ok(Done::UseTerminal { .. }) => {
            let note = if report {
                copied_note(n, "terminal")
            } else {
                String::new()
            };
            let _ = tx.send(tracked_event(
                Event::ToClipboard { text, note },
                action,
                semantic,
            ));
        }
        Err(e) => {
            record_outcome(journal, action.as_ref(), JournalOutcome::Failed, None);
            let _ = tx.send(tracked_event(
                Event::ErrorNotice(format!("could not copy: {e}")),
                action,
                semantic,
            ));
        }
    }
}

fn perform(effect: HostEffect, tx: &UnboundedSender<Event>, journal: Option<&Sink>) {
    perform_in(effect, tx, journal, None, None);
}

fn perform_in(
    effect: HostEffect,
    tx: &UnboundedSender<Event>,
    journal: Option<&Sink>,
    action: Option<ActionId>,
    semantic: Option<ActionId>,
) {
    match effect {
        HostEffect::Work {
            plugin,
            id,
            request,
            mode,
        } => {
            let tx = tx.clone();
            let journal = journal.cloned();
            tokio::spawn(async move {
                let completion = crate::task_completion::complete_work(
                    journal, plugin, id, *request, mode,
                )
                .await;
                let _ = tx.send(completion.event);
            });
        }
        HostEffect::History(request) => {
            let tx = tx.clone();
            let journal = journal.cloned();
            tokio::spawn(async move {
                let completion = crate::task_completion::complete_history(journal, *request).await;
                let _ = tx.send(completion.event);
            });
        }
        HostEffect::Journal(record) => {
            if let Some(journal) = journal {
                journal.submit(record);
            }
        }
        HostEffect::Tracked {
            action,
            semantic,
            effect,
        } => {
            record_outcome(journal, Some(&action), JournalOutcome::Started, None);
            perform_in(*effect, tx, journal, Some(action), semantic);
        }
        HostEffect::Spawn { plugin, task } => {
            let tx = tx.clone();
            let journal = journal.cloned();
            tokio::spawn(async move {
                let completion = complete_task(plugin, task, action, semantic).await;
                if let Some(journal) = journal {
                    for record in completion.records {
                        journal.submit(record);
                    }
                }
                let _ = tx.send(completion.event);
            });
        }
        HostEffect::Open(url) => {
            let env = Env::detect();
            let tx = tx.clone();
            let journal = journal.cloned();
            std::thread::spawn(move || {
                open_web(&env, url, &tx, journal.as_ref(), action, semantic)
            });
        }
        HostEffect::OpenApp { link, web } => {
            let env = Env::detect();
            let tx = tx.clone();
            let journal = journal.cloned();
            std::thread::spawn(move || {
                let ways = system::app_ways(&env, &link);
                if ways.is_empty() {
                    return open_web(&env, web, &tx, journal.as_ref(), action, semantic);
                }
                match system::try_ways(&ways, None) {
                    Ok(Done::Ran { .. }) => {
                        record_outcome(
                            journal.as_ref(),
                            action.as_ref(),
                            JournalOutcome::Succeeded,
                            None,
                        );
                        let _ = tx.send(tracked_event(
                            Event::Notice("opened in its app".into()),
                            action,
                            semantic,
                        ));
                    }
                    _ => open_web(&env, web, &tx, journal.as_ref(), action, semantic),
                }
            });
        }
        HostEffect::Copy(text) => copy_text(text, false, tx, journal, action, semantic),
        HostEffect::CopySelection(text) => copy_text(text, true, tx, journal, action, semantic),
        HostEffect::Save(text) => {
            let result = save(&text).map_err(|e| e.to_string());
            record_outcome(
                journal,
                action.as_ref(),
                if result.is_ok() {
                    JournalOutcome::Succeeded
                } else {
                    JournalOutcome::Failed
                },
                Some(Reason::Storage),
            );
            match &result {
                Ok(()) => log::info(format!("config saved to {}", config_path().display())),
                Err(e) => log::error(format!("config not saved: {e}")),
            }
            let _ = tx.send(tracked_event(Event::Saved(result), action, semantic));
        }
        // Stored on the cache's own thread (crate::plugin_cache), which
        // records the outcome once the write is done.
        HostEffect::WriteCache { plugin, value } => {
            let journal = journal.cloned();
            let done: crate::plugin_cache::Done = Box::new(move |stored| {
                record_outcome(
                    journal.as_ref(),
                    action.as_ref(),
                    if stored {
                        JournalOutcome::Succeeded
                    } else {
                        JournalOutcome::Failed
                    },
                    Some(Reason::Storage),
                );
            });
            crate::plugin_cache::save(plugin, value, Some(done));
        }
        // Setup's look for keys, which the user said yes to: the names of
        // the variables (their values are dropped unread) and which sign-in
        // files exist, off the UI thread.
        HostEffect::Scan(asks) => {
            let tx = tx.clone();
            tokio::task::spawn_blocking(move || {
                let names = std::env::vars_os().map(|(name, _)| name.to_string_lossy().into_owned());
                let found = crate::setup::scan(&asks, names, dirs::home_dir().as_deref(), |p| p.exists());
                let _ = tx.send(tracked_event(Event::Scanned(found), action, semantic));
            });
        }
    }
}

fn save(text: &str) -> Result<()> {
    write_atomic(&config_path(), text)
}

/// Write `text` to `path` whole or not at all: into a file beside it (mode
/// 0600 on Unix, since a typed-in token may be in it), flushed, then renamed
/// over it. A crash or a full disk leaves the old file as it was.
pub(crate) fn write_atomic(path: &std::path::Path, text: &str) -> Result<()> {
    // A config kept elsewhere and linked here stays a link: write where it
    // points.
    let path = &std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config");
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let written = (|| -> Result<()> {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

#[cfg(test)]
mod tests {
    mod detail_pointer_tests {
        use super::super::{DetailGesture, NoticeGesture};
        use crate::host::{Event, Hit};
        use crate::plugin::Pane;
        use crate::select::{Area, Gesture, Released};
        use crate::view::{Intent, Target};
        use ratatui::buffer::Buffer;
        use ratatui::layout::{Position, Rect};

        fn foot(serial: u64) -> Hit {
            Hit::Plugin { plugin: 0, scope: "books", pane: Pane::Detail, id: format!("detail-foot:{serial}:0") }
        }

        #[test]
        fn physical_down_activates_once_and_release_cannot_retarget_a_newer_frame() {
            let mut selection = Gesture::default();
            let mut control = DetailGesture::default();
            let old = foot(1);
            let newer = foot(2);
            let intent = control.press(Some(&old), &mut selection).unwrap();
            assert_eq!(intent, Intent::Activate { target: Target::new("detail-foot:1:0") });
            // Background replacement at the same cell has no role in Drag/Up.
            assert!(control.owns_pointer(), "drag stays with the already-activated control");
            assert!(control.release(), "Up is consumed, never resolved against the newer hit");
            assert!(!control.release(), "a repeated Up cannot fire again");
            let shown = Buffer::empty(Rect::new(0, 0, 20, 2));
            assert!(matches!(selection.up(&shown), Released::Nothing));
            assert_eq!(control.press(Some(&newer), &mut selection), Some(Intent::Activate { target: Target::new("detail-foot:2:0") }));
            assert!(control.release(), "a second physical press is exactly one new activation");
        }

        #[test]
        fn foot_press_cancels_pending_selection_and_word_double_click_history() {
            let shown = Buffer::with_lines(["watch books"]);
            let at = Position::new(1, 0);
            let areas = [Area { rect: shown.area, text: true }];
            let mut selection = Gesture::default();
            selection.down(at, 1, &areas);
            assert!(matches!(selection.up(&shown), Released::Click(..)));
            selection.down(at, 2, &areas);
            selection.drag(Position::new(5, 0));
            assert!(selection.selecting());
            let mut control = DetailGesture::default();
            assert!(control.press(Some(&foot(1)), &mut selection).is_some());
            assert!(!selection.selecting() && selection.selection.is_none());
            assert!(control.release());
            assert!(matches!(selection.up(&shown), Released::Nothing));
            selection.down(at, 3, &areas);
            assert!(matches!(selection.up(&shown), Released::Click(..)), "the old text click history was reset too");
        }

        #[test]
        fn cancel_resize_and_row_release_never_activate_a_new_footer() {
            let mut control = DetailGesture::default();
            let mut selection = Gesture::default();
            let at = Position::new(2, 0);
            let before = Buffer::with_lines(["before"]);
            let after = Buffer::empty(Rect::new(0, 0, 3, 2));
            control.press(Some(&foot(1)), &mut selection);
            control.cancel(&mut selection);
            assert!(!control.release());
            assert!(matches!(selection.up(&after), Released::Nothing));
            selection.down(at, 10, &[]);
            let Released::Click(at, ms) = selection.up(&before) else { panic!("ordinary row press"); };
            assert!(DetailGesture::row_release(at, ms, Some(&foot(2))).is_none(), "new footer at an old row cell is inert");
            assert!(matches!(DetailGesture::row_release(at, ms, Some(&Hit::Pane(Pane::Middle))), Some(Event::Mouse { .. })));
        }

        #[test]
        fn notice_ownership_and_unrelated_text_selection_remain_independent() {
            let mut control = DetailGesture::default();
            let mut notice = NoticeGesture::default();
            let mut selection = Gesture::default();
            let shown = Buffer::with_lines(["watch books"]);
            let areas = [Area { rect: shown.area, text: true }];
            selection.down(Position::new(0, 0), 1, &areas);
            selection.drag(Position::new(4, 0));
            assert!(control.press(Some(&Hit::Pane(Pane::Middle)), &mut selection).is_none());
            control.cancel(&mut selection);
            assert!(selection.selecting(), "non-control cancellation leaves ordinary selection alone");
            let hit = Hit::Notice(7);
            assert!(notice.press(Some(hit.clone())));
            assert!(control.press(Some(&hit), &mut selection).is_none());
            assert!(!control.owns_pointer());
            assert_eq!(notice.release(), Some(Intent::DismissNotice { id: 7 }));
        }
    }
    mod notice_pointer_tests {
        #[test]
        fn notice_pointer_capture_survives_expiry_replacement_and_drag_without_retargeting() {
            let mut notices = crate::notices::Notices::default(); let old = notices.enqueue("old".into());
            let mut gesture = crate::runner::NoticeGesture::default();
            assert!(gesture.press(Some(crate::host::Hit::Notice(old.id))));
            notices.advance_clock(crate::notices::INFO_MS); let new = notices.enqueue("new".into());
            assert!(gesture.owns_pointer());
            assert_eq!(gesture.release(), Some(crate::view::Intent::DismissNotice { id: old.id }));
            assert!(!notices.dismiss(old.id)); assert_eq!(notices.view().active, [new]); assert!(gesture.release().is_none());
        }
        #[test]
        fn history_hint_press_captures_its_stable_command_until_release() {
            use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
            for (code, target) in [(KeyCode::PageUp, "details_up"), (KeyCode::PageDown, "details_down"), (KeyCode::Esc, "close"), (KeyCode::Down, "older"), (KeyCode::Up, "newer")] {
                let mut gesture = crate::runner::NoticeGesture::default();
                assert!(gesture.press(Some(crate::host::Hit::NoticeHistoryKey(KeyEvent::new(code, KeyModifiers::NONE)))));
                assert!(gesture.owns_pointer());
                assert_eq!(gesture.release(), Some(crate::view::Intent::Activate { target: crate::view::Target::new(format!("chrome:notices:{target}")) }));
                assert!(gesture.release().is_none());
            }
        }
    }
    use super::{move_folders_from_gg, resolve_config_home, write_atomic};
    use std::path::{Path, PathBuf};

    #[test]
    fn radars_own_files_move_from_gg_once_and_another_programs_stay() {
        let root = std::env::temp_dir().join(format!("radar-move-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (config, state, shared, both) =
            (root.join("config"), root.join("state"), root.join("shared"), root.join("both"));
        let write = |p: std::path::PathBuf, s: &str| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, s).unwrap();
        };
        write(config.join("gg/config.toml"), "x");
        write(config.join("gg/icons/mine.toml"), "i");
        std::fs::create_dir_all(&state).unwrap();
        // Another program called gg keeps its own files in the same folder.
        write(shared.join("gg/journal/actions.sqlite3"), "j");
        write(shared.join("gg/state.json"), "theirs");
        write(shared.join("gg/config.yaml"), "theirs");
        write(shared.join("gg/owls.json"), "cache");
        write(both.join("gg/config.toml"), "old");
        write(both.join("radar/config.toml"), "new");
        let bases = [config.clone(), state.clone(), shared.clone(), both.clone(), config.clone()];

        let moved = move_folders_from_gg(&bases, &["owls"]);
        assert!(moved.iter().all(|(_, r)| r.is_ok()), "{moved:?}");
        assert_eq!(moved.len(), 4, "config.toml, icons, journal, a part's cache: {moved:?}");
        assert_eq!(std::fs::read_to_string(config.join("radar/config.toml")).unwrap(), "x");
        assert_eq!(std::fs::read_to_string(config.join("radar/icons/mine.toml")).unwrap(), "i");
        assert!(!config.join("gg").exists(), "emptied, so removed");
        assert!(!state.join("radar").exists(), "nothing to move, nothing made");
        assert_eq!(std::fs::read_to_string(shared.join("radar/journal/actions.sqlite3")).unwrap(), "j");
        assert_eq!(std::fs::read_to_string(shared.join("radar/owls.json")).unwrap(), "cache");
        assert_eq!(std::fs::read_to_string(shared.join("gg/state.json")).unwrap(), "theirs", "not radar's: stays");
        assert_eq!(std::fs::read_to_string(shared.join("gg/config.yaml")).unwrap(), "theirs");
        assert_eq!(std::fs::read_to_string(both.join("radar/config.toml")).unwrap(), "new", "never over radar's own");
        assert_eq!(std::fs::read_to_string(both.join("gg/config.toml")).unwrap(), "old");

        assert!(move_folders_from_gg(&bases, &["owls"]).is_empty(), "the second start moves nothing");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_config_folder_is_xdg_else_dot_config_on_every_platform() {
        let p = |s: &str| Some(PathBuf::from(s));
        let none = |_: &Path| false;
        let leaves = ["config.toml", "icons"];
        let mac = p("/Users/a/Library/Application Support/radar");
        assert_eq!(
            resolve_config_home(p("/x"), p("/Users/a"), mac.clone(), &leaves, none),
            PathBuf::from("/x/radar")
        );
        assert_eq!(
            resolve_config_home(None, p("/Users/a"), mac.clone(), &leaves, none),
            PathBuf::from("/Users/a/.config/radar")
        );
        // An empty XDG_CONFIG_HOME is not set.
        assert_eq!(
            resolve_config_home(p(""), p("/Users/a"), mac.clone(), &leaves, none),
            PathBuf::from("/Users/a/.config/radar")
        );
        // A config only in the old macOS folder is still read there ...
        let old_only =
            |q: &Path| q == Path::new("/Users/a/Library/Application Support/radar/config.toml");
        assert_eq!(
            resolve_config_home(None, p("/Users/a"), mac.clone(), &leaves, old_only),
            mac.clone().unwrap()
        );
        // ... until one exists in the new folder.
        let both = |q: &Path| old_only(q) || q == Path::new("/Users/a/.config/radar/config.toml");
        assert_eq!(
            resolve_config_home(None, p("/Users/a"), mac, &leaves, both),
            PathBuf::from("/Users/a/.config/radar")
        );
    }

    /// The runner's loop waits on `frame_tick(motion.next_frame())` beside
    /// its events: a tick comes only while an effect runs. On tokio's paused
    /// clock: a frame's sleep and the wait for it are virtual, so a loaded
    /// machine cannot make the frame miss its 120 ms.
    #[tokio::test(start_paused = true)]
    async fn the_frame_tick_comes_only_while_an_effect_runs() {
        use super::frame_tick;
        use crate::design::theme::Tier;
        use crate::motion::{Cue, Engine, Level, ManualClock, POPUP_MS, Region, begin, region};
        use crate::theme::{Theme, ThemeConfig};
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use std::time::Duration;

        let theme = Theme::build(&ThemeConfig::default(), Tier::TrueColor, &[]);
        let clock = ManualClock::default();
        let mut motion = Engine::new(Level::Subtle, Box::new(clock.clone()));
        let wait = Duration::from_millis(120);
        let ticks = |every| tokio::time::timeout(wait, frame_tick(every));
        assert!(ticks(motion.next_frame()).await.is_err(), "idle: no tick");
        motion.cue(Cue::Popup, &theme);
        assert!(
            ticks(motion.next_frame()).await.is_ok(),
            "an effect runs: ticks"
        );
        let area = Rect::new(0, 0, 20, 6);
        let mut frame = || {
            begin();
            region(Region::Popup, area);
            motion.draw(&mut Buffer::empty(area), &theme);
        };
        frame();
        clock.advance(u64::from(POPUP_MS));
        frame();
        frame();
        assert!(
            ticks(motion.next_frame()).await.is_err(),
            "it ended: idle again"
        );
    }

    #[test]
    fn a_save_replaces_the_file_whole_leaves_no_temp_file_and_keeps_a_link() {
        let dir = std::env::temp_dir().join(format!("radar-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("radar").join("config.toml");
        write_atomic(&path, "a = 1\n").unwrap();
        write_atomic(&path, "a = 2\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = 2\n");
        let left: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, vec![std::ffi::OsString::from("config.toml")]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
            let link = dir.join("linked.toml");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            write_atomic(&link, "a = 3\n").unwrap();
            assert!(
                std::fs::symlink_metadata(&link)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "a = 3\n");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn copy(ways: &[crate::system::Way], report: bool) -> Vec<crate::host::Event> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        super::copy_through(ways, "héllo".into(), report, &tx, None, None, None);
        let mut out = vec![];
        while let Ok(event) = rx.try_recv() {
            out.push(event);
        }
        out
    }

    #[test]
    fn copied_selection_says_which_program_took_it() {
        use crate::host::Event;
        crate::system::real_programs_on_this_thread();
        let by_program = [crate::system::Way::Run("cat".into(), vec![])];
        assert!(matches!(
            &copy(&by_program, true)[..],
            [Event::Notice(note)] if note == "copied 5 characters \u{b7} cat"
        ));
        // A copy a part asked for names what it copied itself.
        assert!(copy(&by_program, false).is_empty());
    }

    #[test]
    fn copied_selection_says_terminal_when_the_terminal_takes_it() {
        use crate::host::Event;
        assert!(matches!(
            &copy(&[crate::system::Way::Terminal], true)[..],
            [Event::ToClipboard { text, note }]
                if text == "héllo" && note == "copied 5 characters \u{b7} terminal"
        ));
        assert!(matches!(
            &copy(&[crate::system::Way::Terminal], false)[..],
            [Event::ToClipboard { note, .. }] if note.is_empty()
        ));
    }

    #[test]
    fn a_copy_no_way_can_do_says_so() {
        use crate::host::Event;
        let none = [crate::system::Way::Run("radar-no-such-program".into(), vec![])];
        assert!(matches!(
            &copy(&none, true)[..],
            [Event::ErrorNotice(note)] if note.starts_with("could not copy: ")
        ));
    }
}

#[cfg(test)]
#[path = "runner_journal_tests.rs"]
mod journal_tests;

#[cfg(test)]
#[path = "runner_cli_journal_tests.rs"]
mod cli_journal_tests;

#[cfg(test)]
#[path = "runner_history_tests.rs"]
mod history_tests;

#[cfg(test)]
#[path = "runner_work_tests.rs"]
mod work_tests;
