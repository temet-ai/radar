//! Setup's flow, its look for keys, `radar setup <step>`, the rail in the
//! chrome, and what the Ratatui shell draws for it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::*;
use crate::health::{Health, Mark as HealthMark, Source};
use crate::host::{Event, Hit, MouseKind};
use crate::plugin::{Outcome, Plugin, SectionSpec};
use crate::view::{Intent, Key};

/// A part that syncs with a service: its `connection` section is its step.
struct Shelf {
    seen: Arc<Mutex<Vec<String>>>,
    row: usize,
    summary: Option<String>,
}

impl Plugin for Shelf {
    fn id(&self) -> &'static str {
        "shelf"
    }
    fn name(&self) -> &'static str {
        "Shelves"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn sections(&self) -> Vec<SectionSpec> {
        vec![
            SectionSpec { id: "connection", name: "connection", group: "shelf" },
            SectionSpec { id: "look", name: "look", group: "shelf" },
        ]
    }
    fn health(&self) -> Option<Health> {
        Some(Health {
            mark: HealthMark { nerd: "S", unicode: "S", emoji: "S" },
            sources: vec![Source::ok("boxes", None)],
            syncing: false,
            every: 60,
        })
    }
    fn setup_step(&self) -> Option<SetupStep> {
        let step = SetupStep::new("shelves", "Shelves", "connection").icon(Icon::Board).about("boxes, labels");
        Some(match &self.summary {
            Some(s) => step.summary(s.clone()),
            None => step,
        })
    }
    fn setup_finds(&self) -> SetupFinds {
        SetupFinds { env: vec!["SHELF_TOKEN", "LEDGER_"], files: vec![("shelfctl", ".config/shelfctl/hosts.yml")] }
    }
    fn section_view(&self, section: &str, _: &crate::view::ViewCx<'_>) -> Option<SectionView> {
        let rows = vec![
            ListRow::new("server", Text::plain("server   https://shelves.example")),
            ListRow::new("token", Text::plain("token    env:SHELF_TOKEN")),
        ];
        Some(SectionView {
            version: crate::view::VERSION,
            scope: section.into(),
            body: Node::List { rows, selected: Some(self.row), editing: None },
            detail_title: "get a token".into(),
            detail: vec![],
            detail_blocks: vec![Block::section_of(Some(Icon::Link), "a new token", None, vec![Text::plain("1 open the new-token page")])],
            actions: vec![],
        })
    }
    fn intent(&mut self, scope: &str, _: Pane, intent: &Intent) -> Outcome {
        let said = match intent {
            Intent::Select { target } => {
                self.row = usize::from(target.as_str() == "token");
                format!("{scope} select {}", target.as_str())
            }
            Intent::Activate { target } => format!("{scope} activate {}", target.as_str()),
            Intent::Key { chord } if chord.key == Key::Enter => format!("{scope} enter"),
            Intent::Key { chord } if chord.key == Key::Char(' ') => {
                self.seen.lock().unwrap().push(format!("{scope} space ignored"));
                return Outcome::Ignored;
            }
            other => format!("{scope} {other:?}"),
        };
        self.seen.lock().unwrap().push(said);
        Outcome::done()
    }
}

/// An agent part: its step is listed under agents.
struct Talk;

impl Plugin for Talk {
    fn id(&self) -> &'static str {
        "talk"
    }
    fn name(&self) -> &'static str {
        "Chat"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn sections(&self) -> Vec<SectionSpec> {
        vec![SectionSpec { id: "talk", name: "talk", group: "talk" }]
    }
    fn health(&self) -> Option<Health> {
        Some(Health { mark: HealthMark { nerd: "", unicode: "", emoji: "" }, sources: vec![], syncing: false, every: 0 })
    }
    fn setup_step(&self) -> Option<SetupStep> {
        Some(SetupStep::new("talk", "Chat", "talk").group(SetupGroup::Agent).about("conversations"))
    }
}

/// A part with no step of its own: it reads this machine.
struct Ledger;

impl Plugin for Ledger {
    fn id(&self) -> &'static str {
        "ledger"
    }
    fn name(&self) -> &'static str {
        "Ledger"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
}

fn fixture() -> (Host, Arc<Mutex<Vec<String>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let shelf = Shelf { seen: seen.clone(), row: 0, summary: None };
    let host = Host::new(vec![Box::new(shelf), Box::new(Talk), Box::new(Ledger)], 0).unwrap();
    (host, seen)
}

/// A first run: no config file.
fn first_run() -> (Host, Arc<Mutex<Vec<String>>>) {
    let (mut host, seen) = fixture();
    host.load_config(None).unwrap();
    assert_eq!(at(&host), At::Welcome, "a first run opens on the welcome");
    press(&mut host, KeyCode::Enter);
    (host, seen)
}

fn press(host: &mut Host, code: KeyCode) -> Vec<HostEffect> {
    host.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
}

fn at(host: &Host) -> At {
    host.setup.as_ref().expect("setup shows").at
}

fn section_id(host: &Host) -> &'static str {
    host.current_section().unwrap().1.id
}

fn words(view: &SectionView) -> Vec<String> {
    match &view.body {
        Node::List { rows, .. } => rows
            .iter()
            .map(|r| {
                let right = r.right.as_ref().map(|t| format!(" | {}", t.words())).unwrap_or_default();
                format!("{}{right}", r.text.words()).trim_end().to_string()
            })
            .collect(),
        other => vec![format!("{other:?}")],
    }
}

fn shelf_found() -> Vec<Found> {
    vec![Found { part: "shelf".into(), env: vec!["env:SHELF_TOKEN".into()], signed_in: vec![] }]
}

// --- the look for keys -----------------------------------------------------

#[test]
fn the_scan_keeps_names_only_and_looks_only_where_a_part_says() {
    let asks: Asks = vec![(
        "shelf".into(),
        SetupFinds {
            env: vec!["SHELF_TOKEN", "LEDGER_"],
            files: vec![
                ("shelfctl", ".config/shelfctl/hosts.yml"),
                ("boxer", ".boxer/auth.json"),
                ("climber", "../outside/auth.json"),
                ("rooted", "/etc/auth.json"),
            ],
        },
    )];
    let home = Path::new("/home/vimes");
    let looked = Mutex::new(Vec::<PathBuf>::new());
    let exists = |p: &Path| {
        looked.lock().unwrap().push(p.to_path_buf());
        p == home.join(".config/shelfctl/hosts.yml")
    };
    // Only names are given: the values never reach the scan.
    let names = ["PATH", "SHELF_TOKEN", "LEDGER_URL", "LEDGER_KEY", "LEDGER_", "SHELF_TOKEN_OLD", "LEDGER_URL"];
    let found = scan(&asks, names, Some(home), exists);
    assert_eq!(
        found,
        vec![Found {
            part: "shelf".into(),
            env: vec!["env:LEDGER_KEY".into(), "env:LEDGER_URL".into(), "env:SHELF_TOKEN".into()],
            signed_in: vec!["shelfctl".into()],
        }]
    );
    assert_eq!(found[0].words().as_deref(), Some("4 found"));
    // Only the files under home a part named: never one climbing out of it.
    assert_eq!(
        *looked.lock().unwrap(),
        vec![home.join(".config/shelfctl/hosts.yml"), home.join(".boxer/auth.json")]
    );
    // No home: no files are looked at.
    let none = scan(&asks, ["SHELF_TOKEN"], None, |_| panic!("looked without a home"));
    assert_eq!(none[0].signed_in, Vec::<String>::new());
    assert_eq!(none[0].words().as_deref(), Some("token found"));
}

#[test]
fn the_scan_finds_a_sign_in_file_in_a_real_folder() {
    let home = std::env::temp_dir().join(format!("radar-setup-scan-{}", std::process::id()));
    let file = home.join(".config/shelfctl/hosts.yml");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, "secret: never read").unwrap();
    let asks: Asks = vec![("shelf".into(), SetupFinds { env: vec![], files: vec![("shelfctl", ".config/shelfctl/hosts.yml")] })];
    let found = scan(&asks, Vec::<String>::new(), Some(&home), |p| p.exists());
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(found[0].signed_in, vec!["shelfctl".to_string()]);
    assert_eq!(found[0].words().as_deref(), Some("signed in"));
    assert!(format!("{found:?}").find("secret").is_none());
}

// --- the flow ----------------------------------------------------------------

#[test]
fn a_first_run_opens_setup_at_the_scan_with_no_rail_and_no_settings_nav() {
    let (host, _) = first_run();
    assert_eq!(at(&host), At::Scan);
    assert_eq!(host.view, View::Settings);
    assert_eq!(host.panes(), vec![Pane::Middle]);
    let chrome = host.chrome_view(160, 40);
    let setup = chrome.setup.as_ref().unwrap();
    assert_eq!(setup.title, "look for keys?");
    assert_eq!(setup.heading, "May radar look for keys on this machine?");
    assert!(setup.rail.is_empty(), "no rail: progress is step n of m");
    assert!(chrome.settings.is_none(), "the settings nav is never sent while setup shows");
    assert!(chrome.top_bar.tabs.is_empty());
    assert_eq!(chrome.status.scope, "SETUP");
    let keys: Vec<_> = chrome.status.hints.iter().map(|h| format!("{} {}", h.keys, h.label)).collect();
    assert_eq!(keys[..4], ["⏎ continue", "↑↓ move", "esc no thanks", "y copy"]);
    let card = words(&host.setup_section_view().unwrap());
    assert!(card.contains(&"variables SHELF_TOKEN, LEDGER_…".to_string()), "{card:?}");
    assert!(card.iter().any(|w| w.contains("yes, look")));
}

#[test]
fn yes_looks_ticks_what_it_found_and_each_tick_is_one_step_in_order() {
    let (mut host, _) = first_run();
    let fx = press(&mut host, KeyCode::Enter);
    let asks = fx
        .iter()
        .find_map(|e| match e {
            HostEffect::Scan(asks) => Some(asks.clone()),
            _ => None,
        })
        .expect("a yes asks the runner to look");
    assert_eq!(asks.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["shelf"]);
    assert_eq!(at(&host), At::Choose);
    let card = words(&host.setup_section_view().unwrap());
    assert!(card.iter().any(|w| w.contains("Shelves") && w.ends_with("| looking…")), "{card:?}");
    host.update(Event::Scanned(shelf_found()));
    let card = words(&host.setup_section_view().unwrap());
    // (Icons are off in tests: the ticks are their words.)
    assert_eq!(
        card,
        [
            "THIS MACHINE",
            "○ local code       checkouts, branches",
            "",
            "SERVICES",
            "● Shelves          boxes, labels | token found",
            "",
            "AGENTS",
            "○ Chat             conversations",
            "",
            "OPTIONAL",
            "○ look & terminal  icons, theme, motion",
        ]
    );
    let ticks: Vec<bool> = host.setup.as_ref().unwrap().choices.iter().map(|c| c.ticked).collect();
    assert_eq!(ticks, [false, true, false, false], "what the scan found, nothing else");
    let hints = host.chrome_view(160, 40).status.hints;
    assert_eq!(hints[0].label, "start here");
    // ⏎ on Shelves starts there; it is the only tick.
    press(&mut host, KeyCode::Down);
    press(&mut host, KeyCode::Enter);
    assert_eq!(at(&host), At::Step(0));
    assert_eq!((host.current_section().unwrap().0, section_id(&host)), (Owner::Plugin(0), "connection"));
    // esc skips just this step; then the mode, chosen with ⏎; then ready.
    press(&mut host, KeyCode::Esc);
    assert_eq!(at(&host), At::Step(1));
    press(&mut host, KeyCode::Enter);
    assert_eq!(at(&host), At::Ready);
    let card = words(&host.setup_section_view().unwrap());
    assert_eq!(card[0], "SETUP");
    assert!(card[1].starts_with("Shelves") && card[1].ends_with("skipped"), "{card:?}");
    assert!(card.contains(&"SAVED IN  config.toml".to_string()), "{card:?}");
    assert!(card.iter().any(|w| w.contains("yes, show me around")));
    // Leaving saves as Settings does, and shows the first screen.
    let fx = press(&mut host, KeyCode::Enter);
    assert!(host.setup.is_none());
    assert!(fx.iter().any(|e| matches!(e, HostEffect::Save(_))));
    assert_eq!(host.panes(), vec![Pane::Nav, Pane::Middle, Pane::Detail]);
}

#[test]
fn no_looks_for_nothing_and_back_walks_the_screens_backwards() {
    let (mut host, _) = first_run();
    press(&mut host, KeyCode::Esc);
    assert_eq!(at(&host), At::Choose);
    assert_eq!(host.setup.as_ref().unwrap().scan, Scan::Declined);
    let ticks = |host: &Host| host.setup.as_ref().unwrap().choices.iter().map(|c| c.ticked).collect::<Vec<_>>();
    assert_eq!(ticks(&host), [false; 4], "with no scan nothing is ticked");
    // A scan answering anyway (it was never asked for) changes nothing.
    host.update(Event::Scanned(shelf_found()));
    assert_eq!(ticks(&host), [false; 4]);
    press(&mut host, KeyCode::Char('a'));
    assert_eq!(ticks(&host), [true; 4]);
    press(&mut host, KeyCode::Char('n'));
    assert_eq!(ticks(&host), [false; 4]);
    press(&mut host, KeyCode::Down);
    press(&mut host, KeyCode::Down);
    press(&mut host, KeyCode::Char(' '));
    assert_eq!(ticks(&host), [false, false, true, false]);
    press(&mut host, KeyCode::Char(' '));
    // ⏎ on the unticked row ticks it and goes to its step, the only one.
    press(&mut host, KeyCode::Enter);
    assert_eq!(at(&host), At::Step(0));
    assert_eq!(ticks(&host), [false, false, true, false]);
    press(&mut host, KeyCode::Left);
    assert_eq!(at(&host), At::Choose);
    press(&mut host, KeyCode::Left);
    assert_eq!(at(&host), At::Scan);
    // Before the scan, the welcome; nothing before that.
    press(&mut host, KeyCode::Left);
    assert_eq!(at(&host), At::Welcome);
    press(&mut host, KeyCode::Left);
    assert_eq!(at(&host), At::Welcome);
    // The scan is asked again (a no is not kept); esc on the chooser is
    // "not now": setup ends.
    press(&mut host, KeyCode::Enter);
    press(&mut host, KeyCode::Esc);
    let fx = press(&mut host, KeyCode::Esc);
    assert!(host.setup.is_none());
    assert!(fx.iter().any(|e| matches!(e, HostEffect::Save(_))));
}

#[test]
fn on_a_section_step_the_section_has_its_keys_and_setup_keeps_enter_esc_and_left() {
    let (mut host, seen) = first_run();
    host.open_setup(Some("shelves")).unwrap();
    press(&mut host, KeyCode::Char('j'));
    assert_eq!(seen.lock().unwrap().last().unwrap(), "connection select token");
    // space is the section's; where it does nothing there, it is the row's ⏎.
    press(&mut host, KeyCode::Char(' '));
    assert_eq!(seen.lock().unwrap()[1..], ["connection space ignored".to_string(), "connection enter".to_string()]);
    // No pane, tab, screen or history key leaves the card.
    for code in [KeyCode::Tab, KeyCode::Char('h'), KeyCode::Char('l'), KeyCode::Right, KeyCode::Char('1'), KeyCode::Char(',')] {
        press(&mut host, code);
        assert_eq!((host.focus, at(&host)), (Pane::Middle, At::Step(0)), "{code:?}");
    }
    press(&mut host, KeyCode::Char('H'));
    assert!(host.history.is_none());
    // ⏎ is setup's: on to ready, as the step was opened alone.
    press(&mut host, KeyCode::Enter);
    assert_eq!(at(&host), At::Ready);
    assert_eq!(seen.lock().unwrap().len(), 3, "⏎ never reached the section");
}

#[test]
fn back_from_a_step_goes_to_the_one_before_then_the_chooser() {
    let (mut host, _) = first_run();
    press(&mut host, KeyCode::Esc);
    press(&mut host, KeyCode::Char('a'));
    press(&mut host, KeyCode::Enter);
    press(&mut host, KeyCode::Enter);
    press(&mut host, KeyCode::Enter);
    assert_eq!(at(&host), At::Step(2));
    press(&mut host, KeyCode::Left);
    assert_eq!(at(&host), At::Step(1));
    press(&mut host, KeyCode::Left);
    assert_eq!(at(&host), At::Step(0));
    // Local code has no part offering it here: the host's own card.
    assert!(host.setup_host_step());
    assert_eq!(host.setup_section_view().unwrap().scope, "setup.local");
    press(&mut host, KeyCode::Left);
    assert_eq!(at(&host), At::Choose);
}

// --- radar setup <step> ------------------------------------------------------

#[test]
fn radar_setup_with_a_parts_step_opens_there_alone_without_the_scan() {
    let (mut host, _) = fixture();
    host.load_config(Some("")).unwrap();
    assert!(host.setup.is_none(), "a config file: no setup by itself");
    host.open_setup(Some("shelves")).unwrap();
    let setup = host.setup.as_ref().unwrap();
    assert_eq!((setup.at, setup.single, &setup.scan), (At::Step(0), true, &Scan::NotAsked));
    assert_eq!((host.current_section().unwrap().0, section_id(&host)), (Owner::Plugin(0), "connection"));
    let chrome = host.chrome_view(160, 40).setup.unwrap();
    assert_eq!((chrome.title.as_str(), chrome.step.as_deref()), ("Shelves", Some("step 1 of 1")));
    press(&mut host, KeyCode::Left);
    assert_eq!(at(&host), At::Step(0), "nothing before a step opened alone");
    press(&mut host, KeyCode::Enter);
    assert_eq!(at(&host), At::Ready);
}

#[test]
fn an_unknown_step_is_refused_with_every_step_named() {
    let (mut host, _) = fixture();
    let why = host.open_setup(Some("vetinari")).unwrap_err();
    assert_eq!(why, "no setup step is called vetinari; the steps are: welcome, scan, choose, look, local, shelves, talk, ready, tour");
    assert!(host.setup.is_none());
    for step in host.setup_step_ids() {
        host.open_setup(Some(&step)).unwrap_or_else(|e| panic!("{step}: {e}"));
    }
}

// --- the rail ------------------------------------------------------------------

#[test]
fn enter_on_a_chooser_row_ticks_it_and_goes_to_its_step() {
    let (mut host, _) = first_run();
    press(&mut host, KeyCode::Esc);
    // Local code ticked, then down to Chat, not ticked.
    press(&mut host, KeyCode::Char(' '));
    for _ in 0..2 {
        press(&mut host, KeyCode::Down);
    }
    assert!(!host.setup.as_ref().unwrap().choices[2].ticked);
    press(&mut host, KeyCode::Enter);
    let setup = host.setup.as_ref().unwrap();
    assert!(setup.choices[2].ticked);
    let chat = setup.plan.len() - 2; // the mode comes after it
    assert_eq!(at(&host), At::Step(chat), "Chat's step, not the first");
    assert_eq!(host.chrome_view(160, 40).setup.unwrap().title, "Chat");
    // The steps before it are still there, back with ←.
    press(&mut host, KeyCode::Left);
    assert_eq!(at(&host), At::Step(chat - 1));
}

#[test]
fn quitting_during_setup_opens_it_again_at_the_next_start_and_leaving_it_does_not() {
    let (host, _) = first_run();
    let text = host.config_text();
    assert!(text.contains("setup = \"unfinished\""), "{text}");
    let (mut again, _) = fixture();
    again.load_config(Some(&text)).unwrap();
    assert_eq!(at(&again), At::Welcome, "setup again, from its start");
    // Left (finished or later): not again.
    let mut setup = again;
    setup.open_setup(Some("look")).unwrap();
    press(&mut setup, KeyCode::Char('f'));
    assert!(!setup.config_text().contains("unfinished"));
    let (mut next, _) = fixture();
    next.load_config(Some(&setup.config_text())).unwrap();
    assert!(next.setup.is_none());
}

#[test]
fn finish_later_keeps_the_answers_and_says_where_setup_paused() {
    let (mut host, _) = fixture();
    host.open_setup(Some("look")).unwrap();
    let fx = press(&mut host, KeyCode::Char('f'));
    assert!(host.setup.is_none());
    assert!(fx.iter().any(|e| matches!(e, HostEffect::Save(_))), "{fx:?}");
    let chrome = host.chrome_view(160, 40);
    assert!(chrome.status.message.is_none(), "status hints stay visible");
    let message = &chrome.notices.active.last().unwrap().text;
    assert!(message.starts_with("setup paused at look & terminal"), "{message}");
}

#[test]
fn a_sections_enter_help_says_space_on_a_setup_step() {
    let mut view = sheet("x", Node::List { rows: vec![], selected: None, editing: None }, vec![Block::section_of(
        None,
        "if you use it",
        None,
        vec![Text::of("⏎ uses it", Ink::Muted), Text::of("⏎", Ink::Muted)],
    )]);
    view.detail = vec![Text::of("⏎ uses it", Ink::Muted)];
    space_for_enter(&mut view);
    assert_eq!(view.detail[0].words(), "space uses it");
    let Block::Section { blocks, .. } = &view.detail_blocks[0] else { panic!() };
    let words: Vec<String> = blocks.iter().map(|b| match b { Block::Line { text, .. } => text.words(), _ => String::new() }).collect();
    assert_eq!(words, ["space uses it", "⏎"]);
}

#[test]
fn each_new_step_of_setup_fades_and_sweeps_in() {
    let (mut host, _) = first_run();
    host.take_cues();
    press(&mut host, KeyCode::Down);
    assert_eq!(host.take_cues(), [], "moving on a step is not a new step");
    press(&mut host, KeyCode::Enter);
    assert_eq!(host.take_cues(), [crate::motion::Cue::Screen, crate::motion::Cue::Page]);
}

#[test]
fn the_look_step_chooses_icons_theme_and_motion_at_once() {
    let (mut host, _) = fixture();
    host.open_setup(Some("look")).unwrap();
    let card = words(&host.setup_section_view().unwrap());
    for words in ["TERMINAL", "ICONS", "THEME", "MOTION", "PREVIEW", "plain", "silkcircuit neon", "subtle"] {
        assert!(card.iter().any(|l| l.contains(words)), "{words} in {card:#?}");
    }
    // The cursor starts on the first icon pack; down twice is emoji.
    press(&mut host, KeyCode::Down);
    press(&mut host, KeyCode::Down);
    press(&mut host, KeyCode::Char(' '));
    assert_eq!(host.colours.cfg.icons, "emoji");
    // A click on a motion row chooses it.
    let n = host.look_rows().iter().position(|r| *r == LookRow::Motion(Level::Off)).unwrap();
    host.update(Event::Intent(Intent::Activate { target: Target::new(format!("{PREFIX}look:{n}")) }));
    assert_eq!(host.colours.cfg.motion, "off");
    assert!(host.chrome_view(160, 40).status.hints.iter().any(|h| h.label == "choose"));
    press(&mut host, KeyCode::Enter);
    assert_eq!(at(&host), At::Ready);
}

#[test]
fn settings_look_section_chooses_motion_and_says_what_each_level_shows() {
    let (mut host, _) = fixture();
    host.load_config(Some("")).unwrap();
    host.view = crate::host::View::Settings;
    host.section = host.sections.iter().position(|(o, _)| *o == crate::host::Owner::Look).unwrap();
    host.focus = crate::Pane::Middle;
    assert!(host.setup.is_none());
    assert_eq!(host.colours.cfg.motion, "subtle", "subtle by default");
    let view = host.look_settings_view();
    let Node::List { rows, .. } = &view.body else { panic!("a list") };
    let mut in_order = vec![];
    for level in Level::ALL {
        let n = host.look_rows().iter().position(|r| *r == LookRow::Motion(level)).unwrap();
        let row = rows
            .iter()
            .find(|r| r.target.as_str() == format!("{PREFIX}look:{n}"))
            .unwrap_or_else(|| panic!("a row for {}", level.word()));
        assert!(row.text.words().contains(level.word()), "{}", row.text.words());
        assert_eq!(row.right.as_ref().map(|t| t.words()).as_deref(), Some(level.about()));
        in_order.push((n, level));
    }
    // Space or ⏎ on a row chooses it, and the file says so.
    for (n, level) in in_order {
        press(&mut host, KeyCode::Home);
        for _ in 0..n {
            press(&mut host, KeyCode::Down);
        }
        press(&mut host, if n % 2 == 0 { KeyCode::Char(' ') } else { KeyCode::Enter });
        assert_eq!(host.colours.cfg.motion, level.word());
        assert!(host.menu.is_none());
    }
    let back: crate::theme::ThemeConfig = toml::from_str(&toml::to_string(&host.colours.cfg).unwrap()).unwrap();
    assert_eq!(back.motion, "off", "it is saved under [theme]");
}

#[test]
fn a_renderer_presses_a_host_steps_rows_by_their_targets() {
    let (mut host, _) = first_run();
    let row = |s: &str| Intent::Activate { target: Target::new(s) };
    host.update(Event::Intent(Intent::Select { target: Target::new("setup:scan:1") }));
    assert_eq!(host.setup.as_ref().unwrap().row, 1);
    assert_eq!(at(&host), At::Scan, "a select only moves the choice");
    host.update(Event::Intent(row("setup:scan:1")));
    assert_eq!((at(&host), &host.setup.as_ref().unwrap().scan), (At::Choose, &Scan::Declined));
    host.update(Event::Intent(row("setup:choose:2")));
    assert!(host.setup.as_ref().unwrap().choices[2].ticked);
    // Another screen's row is not this one's.
    host.update(Event::Intent(row("setup:scan:0")));
    assert_eq!(at(&host), At::Choose);
}

// --- what Ratatui draws ------------------------------------------------------

fn screen(host: &Host, w: u16, h: u16) -> String {
    crate::shell::to_text(host, w, h)
}

#[test]
fn the_chooser_and_a_parts_step_drawn_at_160_by_40() {
    let (mut host, _) = first_run();
    press(&mut host, KeyCode::Enter);
    host.update(Event::Scanned(shelf_found()));
    insta::assert_snapshot!("choose_160x40", screen(&host, 160, 40));
    press(&mut host, KeyCode::Enter);
    press(&mut host, KeyCode::Enter);
    insta::assert_snapshot!("part_step_160x40", screen(&host, 160, 40));
}

#[test]
fn the_scan_and_ready_drawn_at_80_by_24() {
    let (mut host, _) = first_run();
    insta::assert_snapshot!("scan_80x24", screen(&host, 80, 24));
    press(&mut host, KeyCode::Esc);
    press(&mut host, KeyCode::Enter);
    press(&mut host, KeyCode::Enter);
    insta::assert_snapshot!("mode_80x24", screen(&host, 80, 24));
    press(&mut host, KeyCode::Enter);
    insta::assert_snapshot!("ready_80x24", screen(&host, 80, 24));
}

#[test]
fn a_click_ticks_a_chooser_row() {
    let (mut host, _) = first_run();
    press(&mut host, KeyCode::Esc);
    screen(&host, 160, 40);
    let area_of = |host: &Host, want: &dyn Fn(&Hit) -> bool| {
        host.hits.borrow().iter().rev().find(|(_, h)| want(h)).map(|(r, _)| *r).expect("drawn")
    };
    let r = area_of(&host, &|h| matches!(h, Hit::Setup(id) if id == "setup:choose:2"));
    host.update(Event::Mouse { kind: MouseKind::Press, column: r.x + 2, row: r.y, at_ms: 0 });
    assert!(host.setup.as_ref().unwrap().choices[2].ticked);
    host.update(Event::Mouse { kind: MouseKind::Press, column: r.x + 2, row: r.y, at_ms: 100 });
    assert!(!host.setup.as_ref().unwrap().choices[2].ticked, "each click ticks, never a double click's ⏎");
    assert_eq!(at(&host), At::Choose);
}

#[test]
fn a_steps_sheet_is_its_help_not_the_rows_settings_detail() {
    let mut view = sheet("x", Node::List { rows: vec![], selected: None, editing: None }, vec![
        Block::line(Text::of("CHECKED", Ink::Muted)),
        Block::line(Text::of("not checked yet", Ink::Primary)),
        Block::section_of(None, "get a token", None, vec![Text::of("1 open the page", Ink::Primary)]),
    ]);
    help_only(&mut view);
    assert_eq!(view.detail_blocks.len(), 1);
    assert!(matches!(&view.detail_blocks[0], Block::Section { title, .. } if title == "GET A TOKEN"));
    // No help: the row's own detail stays.
    let mut plain = sheet("x", Node::List { rows: vec![], selected: None, editing: None }, vec![Block::line(Text::of("NOW", Ink::Muted))]);
    help_only(&mut plain);
    assert_eq!(plain.detail_blocks.len(), 1);
}

#[test]
fn the_welcome_says_what_radar_and_setup_are_before_anything_is_asked() {
    let (mut host, _) = fixture();
    host.load_config(None).unwrap();
    assert_eq!(at(&host), At::Welcome);
    let chrome = host.chrome_view(160, 40).setup.unwrap();
    assert_eq!((chrome.title.as_str(), chrome.heading.as_str()), ("welcome", "Let's get you set up."));
    let view = host.setup_section_view().unwrap();
    let card = words(&view);
    assert_eq!(card.len(), 2, "two choices, nothing else: {card:#?}");
    assert!(card[0].contains("yes, let's set it up") && card[1].contains("not now"), "{card:#?}");
    assert_eq!(view.detail_title, "about radar");
    // Nothing is looked for from here: ⏎ goes to the question.
    let fx = press(&mut host, KeyCode::Enter);
    assert!(!fx.iter().any(|e| matches!(e, HostEffect::Scan(_))));
    assert_eq!(at(&host), At::Scan);
    press(&mut host, KeyCode::Left);
    assert_eq!(at(&host), At::Welcome);
    // Not now: setup ends and saves.
    press(&mut host, KeyCode::Down);
    let fx = press(&mut host, KeyCode::Enter);
    assert!(host.setup.is_none());
    assert!(fx.iter().any(|e| matches!(e, HostEffect::Save(_))));
}

#[test]
fn every_setup_ends_with_the_mode_and_the_user_chooses_it() {
    let (mut host, _) = first_run();
    press(&mut host, KeyCode::Esc); // no look for keys
    press(&mut host, KeyCode::Enter); // ⏎ on local code
    let setup = host.setup.as_ref().unwrap();
    assert_eq!(setup.plan.last(), Some(&Step::Mode), "the mode is always the last step");
    let last = setup.plan.len() - 1;
    for _ in 0..last {
        press(&mut host, KeyCode::Enter);
    }
    assert_eq!(at(&host), At::Step(last));
    assert_eq!(host.chrome_view(160, 40).setup.unwrap().heading, "What may radar send?");
    let card = words(&host.setup_section_view().unwrap());
    assert!(card.iter().all(|w| !w.starts_with('●')), "nothing chosen yet: {card:?}");
    // esc does not skip it.
    press(&mut host, KeyCode::Esc);
    assert_eq!(at(&host), At::Step(last));
    // ⏎ chooses the row under the cursor (read only, the first) and goes on.
    press(&mut host, KeyCode::Enter);
    assert_eq!(host.mode(), crate::writes::Mode::ReadOnly);
    assert_eq!(at(&host), At::Ready);
    assert!(host.config_text().contains("mode = \"read-only\""), "{}", host.config_text());
}
