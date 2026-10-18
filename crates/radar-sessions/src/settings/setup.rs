//! The agents step of first-run setup (`radar setup agents`). Its card is
//! the Agents section, the same rows Settings edits: each CLI with its
//! state, a way to add a command, and the provider new sessions start with.
//! Its sheet is the help the selected row needs: how to install and sign in
//! to each agent, as commands that `y` copies, and the links.
//!
//! Nothing here reads a credential. [`finds`] names the sign-in files the
//! host may ask about (existence only); the state of each CLI is what its own
//! adapter says of it.

use radar_core::mnemonic;
use radar_core::plugin::{SetupFinds, SetupGroup, SetupOption, SetupStep};
use radar_core::ui::icon::Icon;
use radar_core::view::{Block, Ink, Seg, Text};

use super::{AgentsSettings, status::State};

/// radar's guide to the agents.
const GUIDE: &str = "https://github.com/temet-ai/radar/blob/main/docs/user-guide/agents.md";

/// What it takes to install one agent's program and sign in to it.
pub(crate) struct Help {
    /// The provider id (`claude`).
    pub id: &'static str,
    /// The command that installs it.
    pub install: &'static str,
    /// The command that signs in (the one `i` runs in a tab).
    pub login: &'static str,
    /// Where its own page says more.
    pub page: &'static str,
    pub page_words: &'static str,
    /// What else it needs, in muted words.
    pub needs: &'static [&'static str],
}

/// The native providers' help, in the order they are listed.
pub(crate) const HELP: [Help; 3] = [
    Help {
        id: "copilot",
        install: "npm i -g @github/copilot",
        login: "copilot login",
        page: "https://docs.github.com/en/copilot/how-tos/set-up/install-copilot-cli",
        page_words: "GitHub: install Copilot CLI",
        needs: &["Needs a Copilot plan on GitHub."],
    },
    Help {
        id: "codex",
        install: "npm i -g @openai/codex",
        login: "codex login",
        page: "https://github.com/openai/codex",
        page_words: "Codex: install and sign in",
        needs: &[],
    },
    Help {
        id: "claude",
        install: "npm i -g @anthropic-ai/claude-code",
        login: "claude auth login",
        page: "https://docs.claude.com/en/docs/claude-code/setup",
        page_words: "Claude Code: set up",
        // The Agent SDK overview's terms: a product may not offer claude.ai login.
        needs: &[
            "radar runs your own claude and offers no sign-in of its own.",
            "Anthropic does not allow a claude.ai login in a product: for one it asks for an API key (ANTHROPIC_API_KEY).",
        ],
    },
];

pub(crate) fn help(id: &str) -> Option<&'static Help> {
    HELP.iter().find(|h| h.id == id)
}

/// The native providers in the chooser's order: the one most people come
/// with first.
const CHOOSER: [&str; 3] = ["claude", "codex", "copilot"];

/// The step: one stop on the rail, and in the chooser one row per agent
/// under its AGENTS heading (each native provider, then the commands of
/// your own), each with its state; the look for keys ticks the ready ones.
/// `asked`: the CLIs not read yet are being asked (else nobody asked them).
pub(crate) fn step(s: &AgentsSettings, asked: bool) -> SetupStep {
    let mut step = SetupStep::new("agents", "agents", super::SECTION)
        .heading("Agents that work beside you.", "radar runs each agent's own program, signed in as you.")
        .icon(Icon::Bot)
        .about("Claude Code, Codex, Copilot")
        .group(SetupGroup::Agent);
    let natives = CHOOSER.iter().map(|id| (id.to_string(), Icon::Bot));
    let own = s.cfg.pty.iter().map(|c| (c.id.clone(), Icon::Terminal));
    for (id, icon) in natives.chain(own) {
        step = step.option(option(s, &id, icon, asked));
    }
    match summary(s) {
        Some(words) => step.summary(words),
        None => step,
    }
}

/// One agent's row in the chooser: its name, its icon, and its state.
fn option(s: &AgentsSettings, id: &str, icon: Icon, asked: bool) -> SetupOption {
    let state = s.state(id);
    SetupOption {
        label: s.name_of(id),
        icon: Some(icon),
        note: Some(note(&state, asked).into()),
        ready: state.usable(),
        id: id.to_string(),
    }
}

/// The chooser's word for a state: what it asks of you, if anything.
fn note(state: &State, asked: bool) -> &'static str {
    match state {
        State::Unread if !asked => "not checked",
        State::Unread => "checking…",
        State::SignedOut => "sign in needed",
        other => other.word(),
    }
}

/// `claude · default`: the provider new sessions start with, once a session
/// could start with it; `None` while it is not installed or not read yet.
pub(crate) fn summary(s: &AgentsSettings) -> Option<String> {
    let provider = &s.defaults().provider;
    s.state(provider).usable().then(|| format!("{provider} · default"))
}

/// Where setup may look, once asked: the sign-in files of `claude` and
/// `codex` (names only; the CLI says who is signed in). Claude Code keeps
/// its login in the Keychain on the Mac, so only Linux has a file to find;
/// Copilot keeps its token in the system's credential store and has none.
pub(crate) fn finds() -> SetupFinds {
    SetupFinds {
        env: vec![],
        files: vec![("claude", ".claude/.credentials.json"), ("codex", ".codex/auth.json")],
    }
}

/// The command `y` copies for a CLI: its install command while it is not
/// installed, else the one that signs in.
pub(crate) fn command_for(s: &AgentsSettings, id: &str) -> Option<String> {
    let missing = s.state(id) == State::Missing;
    match help(id) {
        Some(h) => Some((if missing { h.install } else { h.login }).to_string()),
        None if missing => None,
        None => match crate::login::method(id, &s.cfg.pty) {
            crate::login::Method::Command(argv) => Some(argv.join(" ")),
            _ => None,
        },
    }
}

/// The target of a copyable command line in the sheet: `copy:install:claude`.
pub(crate) fn copy_target(kind: &str, id: &str) -> String {
    format!("copy:{kind}:{id}")
}

/// The command a click on a command line copies.
pub(crate) fn copy_of(target: &str) -> Option<String> {
    let (kind, id) = target.strip_prefix("copy:")?.split_once(':')?;
    let h = help(id)?;
    match kind {
        "install" => Some(h.install.into()),
        "login" => Some(h.login.into()),
        _ => None,
    }
}

fn line(words: &str, ink: Ink) -> Block {
    Block::line(Text::of(words, ink))
}

/// A `$ command` line; clicking it copies it. A keyed line has an action
/// caption so its mnemonic never edits or relies on the literal command.
fn command(kind: &str, h: &Help, key: Option<&str>) -> Block {
    let cmd = if kind == "install" { h.install } else { h.login };
    let mut text = Text::new();
    if key.is_some() {
        text.add(Seg::new("copy command · ", Ink::Secondary));
    }
    text.add(Seg::new("$ ", Ink::Muted));
    text.add(Seg::new(cmd, Ink::Primary));
    let target = copy_target(kind, h.id);
    match key {
        Some(key) => Block::keyed(target, text, key),
        None => Block::action(target, text),
    }
}

/// A link: its original words and URL, with a keyed action caption when selected.
fn link(words: &str, url: &str, key: Option<&str>) -> Block {
    let caption = if key.is_some() { "open documentation · " } else { "" };
    let text = Text::new().push(Seg::icon(Icon::Link)).push(Seg::new(format!(" {caption}{words}"), Ink::Accent).linked(url));
    let mnemonic_at = key.and_then(|key| mnemonic::at_text(key, &text));
    let mnemonic_shift = mnemonic_at.is_some() && key.is_some_and(mnemonic::shifted);
    Block::Line { text, target: None, key: key.map(Into::into), mnemonic_at, mnemonic_shift }
}

/// The sheet of a CLI row (or the add row): a sign-in under way, what is
/// wrong when something is, then how to install and sign in to each agent;
/// `y` and `o` act on the selected one, and the keys are shown there.
pub(crate) fn sheet(s: &AgentsSettings, login: Vec<Text>, selected: Option<&str>) -> Vec<Block> {
    let mut blocks: Vec<Block> = login.into_iter().map(Block::line).collect();
    if !blocks.is_empty() {
        blocks.push(Block::line(Text::new()));
    }
    if let Some(id) = selected {
        let state = s.state(id);
        if matches!(state, State::Error(_) | State::PolicyBlocked(_)) {
            blocks.push(line(&state.advice(), Ink::Warn));
            blocks.push(Block::line(Text::new()));
        }
    }
    let mut refresh = Text::of("Each agent is its own program with its own sign-in. radar finds it on your PATH; ", Ink::Muted);
    refresh.extend(mnemonic::text("r", "read its status again", "text.muted"));
    refresh.add(Seg::new(".", Ink::Muted));
    blocks.push(Block::line(refresh));
    blocks.push(Block::line(Text::new()));
    for h in &HELP {
        let name = s.name_of(h.id);
        let state = s.state(h.id);
        let mine = selected == Some(h.id);
        // `y` copies the command still needed: the install while it is missing, else the sign-in.
        let missing = state == State::Missing;
        let key = |install: bool| (mine && install == missing).then_some("y");
        let mut body = vec![command("install", h, key(true)), command("login", h, key(false))];
        body.extend(h.needs.iter().map(|w| line(w, Ink::Muted)));
        body.push(link(h.page_words, h.page, mine.then_some("o")));
        blocks.push(Block::section(Some(Icon::Bot), &name, Some(state.word().into()), body));
    }
    blocks.push(Block::section(Some(Icon::Terminal), "any other CLI", None, vec![
        line("add a command takes what you would type in a terminal; radar runs it in a tab of the dock.", Ink::Secondary),
    ]));
    let mut permissions = Text::new();
    for (key, words) in [("a", "approve"), ("s", "session"), ("d", "deny"), ("v", "view diff")] {
        if !permissions.is_empty() {
            permissions.add(Seg::new(" · ", Ink::Muted));
        }
        permissions.extend(mnemonic::text(key, words, "text.muted"));
    }
    blocks.push(Block::section(Some(Icon::Warning), "nothing is approved for you", None, vec![
        line("A request waits in the dock for your answer:", Ink::Secondary),
        Block::line(permissions),
        line("alt-a switches the agent new sessions start as.", Ink::Muted),
    ]));
    blocks.push(Block::section(Some(Icon::Link), "more", None, vec![link("radar guide: agents", GUIDE, None)]));
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{AdapterStatus, AuthStatus, Capabilities};
    use crate::login::LoginEvent;
    use crate::settings::{SECTION, config::PtyCli};
    use radar_core::view::{Chord, Intent, Node, SectionView, Target, ViewCx};
    use radar_core::{Effect, Outcome};

    fn status(installed: bool, auth: AuthStatus) -> AdapterStatus {
        AdapterStatus {
            installed,
            version: installed.then(|| "1.0".into()),
            protocol: None,
            auth,
            error: None,
            capabilities: Capabilities::default(),
        }
    }

    fn signed_in(who: &str) -> AdapterStatus {
        status(true, AuthStatus::SignedIn { as_user: who.into() })
    }

    /// Claude Code and Codex ready, Copilot not installed.
    fn two_ready() -> AgentsSettings {
        let mut s = AgentsSettings::default();
        s.set_status("claude", signed_in("vimes@ankh-morpork.example"));
        s.set_status("codex", signed_in("vimes"));
        s.set_status("copilot", status(false, AuthStatus::Unknown));
        s
    }

    fn cx() -> ViewCx<'static> {
        ViewCx {
            now: 0,
            focus: radar_core::view::PaneId::Middle,
            width: 80,
            height: 24,
            parts: radar_core::Parts::none(),
            icon_lead: 0,
            setup: false,
        }
    }

    fn text_of(t: &radar_core::view::Text) -> String {
        t.0.iter().map(|s| s.text.as_str()).collect()
    }

    /// Every line of the sheet, in order, with its key when it has one.
    fn sheet_lines(blocks: &[Block]) -> Vec<String> {
        let mut out = vec![];
        for b in blocks {
            match b {
                Block::Line { text, key, .. } => out.push(match key {
                    Some(k) => format!("{} [{k}]", text_of(text)),
                    None => text_of(text),
                }),
                Block::Section { title, detail, blocks, .. } => {
                    out.push(format!("# {title} {}", detail.clone().unwrap_or_default()).trim_end().to_string());
                    out.extend(sheet_lines(blocks));
                }
                _ => {}
            }
        }
        out
    }

    fn view(s: &AgentsSettings) -> SectionView {
        s.section_view(SECTION, &cx()).unwrap()
    }

    fn select(s: &mut AgentsSettings, target: &str) {
        s.intent(SECTION, &Intent::Select { target: Target::new(target) });
    }

    fn key(s: &mut AgentsSettings, c: char) -> Vec<Effect> {
        match s.intent(SECTION, &Intent::Key { chord: Chord::char(c) }) {
            Outcome::Handled(fx) => fx,
            Outcome::Ignored => vec![],
        }
    }

    /// The setup card is the tools and "+ add a command" alone: none of the
    /// new-session defaults is drawn, and the cursor cannot land on one.
    #[test]
    fn the_setup_card_leaves_out_the_new_session_defaults_and_the_cursor_stays_on_what_is_drawn() {
        let hidden = [
            "def:provider", "def:model", "def:work_dir", "def:roots", "def:skills", "def:project_config",
            "def:allow", "def:deny", "def:local_only", "def:reviewer_commands", "rec:folder", "rec:reviews", "dock:notify",
        ];
        let targets = |v: &SectionView| match &v.body {
            Node::List { rows, .. } => rows.iter().filter(|r| !r.heading).map(|r| r.target.as_str().to_string()).collect::<Vec<_>>(),
            other => panic!("a list, not {other:?}"),
        };
        let mut s = two_ready();
        let settings = s.section_view(SECTION, &cx()).unwrap();
        let in_settings = targets(&settings);
        assert!(hidden.iter().all(|h| in_settings.iter().any(|t| t == h)), "Settings keeps every row: {in_settings:?}");

        let setup_cx = ViewCx { setup: true, ..cx() };
        let card = s.section_view(SECTION, &setup_cx).unwrap();
        let drawn = targets(&card);
        for gone in hidden {
            assert!(!drawn.iter().any(|t| t == gone), "{gone} is Settings' row: {drawn:?}");
        }
        assert_eq!(drawn, ["cli:copilot", "cli:codex", "cli:claude", "cli:add"], "the tools and the add row");
        let Node::List { rows, .. } = &card.body else { unreachable!() };
        assert!(rows.iter().all(|r| !text_of(&r.text).to_lowercase().contains("new sessions")), "no defaults heading");

        // Keys and clicks walk what is drawn: every drawn row can be selected,
        // and a hidden one cannot.
        for t in &drawn {
            select(&mut s, t);
            assert_eq!(&s.selected().target(), t);
        }
        for gone in hidden {
            select(&mut s, "cli:add");
            select(&mut s, gone);
            assert_eq!(s.selected().target(), "cli:add", "{gone} cannot be selected in setup");
        }

        // Back in Settings the rows are there again.
        let back = s.section_view(SECTION, &cx()).unwrap();
        assert_eq!(targets(&back), in_settings);
    }

    #[test]
    fn the_step_is_the_agents_section_and_says_which_agent_new_sessions_start_as() {
        let mut s = AgentsSettings::default();
        let step = step(&s, true);
        assert_eq!((step.id, step.label, step.section, step.group), ("agents", "agents", SECTION, SetupGroup::Agent));
        assert!(step.icon.is_some() && !step.about.is_empty());
        assert_eq!(step.summary, None, "nothing to say before a status is read");

        s = two_ready();
        s.on_menu("agents.provider", "claude").unwrap();
        assert_eq!(super::step(&s, true).summary.as_deref(), Some("claude · default"));
        // The default is not installed: nothing is set that a session could start with.
        s.on_menu("agents.provider", "copilot").unwrap();
        assert_eq!(summary(&s), None);
    }

    /// `(id, label, note, ready)` of each of the step's own rows.
    fn options(s: &AgentsSettings, asked: bool) -> Vec<(String, String, String, bool)> {
        step(s, asked).options.into_iter().map(|o| (o.id, o.label, o.note.unwrap_or_default(), o.ready)).collect()
    }

    #[test]
    fn the_chooser_has_a_row_per_agent_with_its_icon_and_state() {
        let row = |id: &str, label: &str, note: &str, ready: bool| (id.to_string(), label.to_string(), note.to_string(), ready);
        // Nothing read yet: every native agent is being checked, none is ready.
        let s = AgentsSettings::default();
        assert_eq!(options(&s, true), [
            row("claude", "Claude Agent", "checking…", false),
            row("codex", "Codex", "checking…", false),
            row("copilot", "Copilot", "checking…", false),
        ]);
        // Nobody asked them (no yes to setup's look, Settings not opened).
        assert!(options(&s, false).iter().all(|o| o.2 == "not checked"));
        assert!(step(&s, true).options.iter().all(|o| o.icon == Some(Icon::Bot)));

        // Signed in, signed out, not installed; a command of your own after them.
        let mut s = two_ready();
        s.set_status("codex", status(true, AuthStatus::SignedOut));
        s.set_path(Some(String::new()));
        s.on_menu("agents.pty_add", "aider --yes").unwrap();
        assert_eq!(options(&s, true)[..3], [
            row("claude", "Claude Agent", "signed in", true),
            row("codex", "Codex", "sign in needed", false),
            row("copilot", "Copilot", "not installed", false),
        ]);
        let own = &step(&s, true).options[3];
        assert_eq!((own.id.as_str(), own.label.as_str(), own.icon), ("aider", "aider", Some(Icon::Terminal)));
    }

    #[test]
    fn it_looks_for_the_sign_in_files_of_claude_and_codex_by_name_only() {
        let f = finds();
        assert!(f.env.is_empty());
        let programs: Vec<_> = f.files.iter().map(|(p, _)| *p).collect();
        assert_eq!(programs, ["claude", "codex"]);
        for (_, path) in &f.files {
            assert!(!path.starts_with('/') && !path.contains(".."), "{path} is under the home folder");
        }
        // Through the host's pure scan: a file that exists is a sign-in; nothing is read.
        let asks = vec![("sessions".to_string(), f)];
        let home = std::path::Path::new("/home/vimes");
        let found = radar_core::setup::scan(&asks, std::iter::empty::<&str>(), Some(home), |p| p == home.join(".codex/auth.json"));
        assert_eq!(found[0].signed_in, ["codex"]);
        assert!(found[0].env.is_empty());
    }

    #[test]
    fn a_cli_row_has_the_help_to_install_and_sign_in_with_the_keys_where_they_act() {
        let mut s = two_ready();
        select(&mut s, "cli:copilot");
        let v = view(&s);
        assert_eq!(v.detail_title, "install and sign in");
        let lines = sheet_lines(&v.detail_blocks);
        let text = lines.join("\n");
        for want in [
            "# COPILOT not installed",
            "$ npm i -g @github/copilot [y]",
            "$ copilot login",
            "# CODEX signed in",
            "$ npm i -g @openai/codex",
            "$ codex login",
            "# CLAUDE AGENT signed in",
            "$ claude auth login",
            "Claude Code: set up",
            "# ANY OTHER CLI",
            "# NOTHING IS APPROVED FOR YOU",
        ] {
            assert!(text.contains(want), "{want} in\n{text}");
        }
        // `y` is shown on the one command it copies, `o` on the selected agent's page only.
        assert_eq!(text.matches("[y]").count(), 1, "{text}");
        assert!(text.contains("GitHub: install Copilot CLI [o]") && text.matches("[o]").count() == 1, "{text}");
        // Selecting a signed-in one moves `y` to its sign-in command.
        select(&mut s, "cli:codex");
        let text = sheet_lines(&view(&s).detail_blocks).join("\n");
        assert!(text.contains("$ codex login [y]") && !text.contains("$ npm i -g @openai/codex [y]"), "{text}");
        // A default row has no sheet: its own detail shows.
        select(&mut s, "def:model");
        assert!(view(&s).detail_blocks.is_empty());
    }

    #[test]
    fn claude_is_run_as_your_own_claude_and_the_terms_say_so() {
        let text = sheet_lines(&sheet(&two_ready(), vec![], None)).join("\n");
        for want in ["offers no sign-in of its own", "claude.ai login", "API key"] {
            assert!(text.contains(want), "{want} in\n{text}");
        }
    }

    #[test]
    fn y_copies_the_install_command_while_missing_then_the_sign_in_and_o_opens_the_page() {
        let mut s = two_ready();
        select(&mut s, "cli:copilot");
        assert!(matches!(&key(&mut s, 'y')[..], [Effect::Notify(_), Effect::Copy(c)] if c == "npm i -g @github/copilot"));
        s.set_status("copilot", status(true, AuthStatus::SignedOut));
        assert!(matches!(&key(&mut s, 'y')[..], [Effect::Notify(_), Effect::Copy(c)] if c == "copilot login"));
        assert!(matches!(&key(&mut s, 'o')[..], [Effect::Open(u)] if u.contains("install-copilot-cli")));
        // A click on a command line copies that command, whichever row is selected.
        let fx = match s.intent(SECTION, &Intent::Activate { target: Target::new(setup_target("install", "claude")) }) {
            Outcome::Handled(fx) => fx,
            Outcome::Ignored => panic!("ignored"),
        };
        assert!(matches!(&fx[..], [Effect::Notify(_), Effect::Copy(c)] if c == "npm i -g @anthropic-ai/claude-code"));
        assert_eq!(s.selected().target(), "cli:copilot", "the click does not move the selection");
        assert_eq!(copy_of("copy:install:nonesuch"), None);
        assert_eq!(copy_of("cli:claude"), None);
    }

    mod mnemonic_tests {
        use super::*;

        #[test]
        fn command_and_documentation_captions_carry_mnemonics_without_changing_literal_payloads() {
            let original = [
                ("copilot", "npm i -g @github/copilot", "copilot login", "https://docs.github.com/en/copilot/how-tos/set-up/install-copilot-cli"),
                ("codex", "npm i -g @openai/codex", "codex login", "https://github.com/openai/codex"),
                ("claude", "npm i -g @anthropic-ai/claude-code", "claude auth login", "https://docs.claude.com/en/docs/claude-code/setup"),
            ];
            let mut s = two_ready();
            // CLI selection in setup does not discover recipes or inspect PATH.
            s.setup_card.set(true);
            for (h, (id, install, login, url)) in HELP.iter().zip(original) {
                assert_eq!((h.id, h.install, h.login, h.page), (id, install, login, url));
                for (kind, cmd) in [("install", install), ("login", login)] {
                    for shown_key in [None, Some("y")] {
                        let Block::Line { text, target, key, mnemonic_at, mnemonic_shift } = command(kind, h, shown_key) else { panic!("a command line") };
                        assert_eq!(text.0.last().unwrap().text, cmd, "the literal command stays its own unmodified segment");
                        assert_eq!(text.words(), format!("{}$ {cmd}", if shown_key.is_some() { "copy command · " } else { "" }));
                        assert_eq!(target.as_ref().map(Target::as_str), Some(format!("copy:{kind}:{id}").as_str()));
                        assert_eq!(key.as_deref(), shown_key);
                        assert_eq!(mnemonic_at, shown_key.map(|_| 3), "y is in copy, never manufactured inside the command");
                        assert!(!mnemonic_shift);
                        let fx = s.intent(SECTION, &Intent::Activate { target: target.unwrap() });
                        assert!(matches!(fx, Outcome::Handled(ref fx) if matches!(&fx[..], [Effect::Notify(_), Effect::Copy(value)] if value == cmd)));
                    }
                }
                for shown_key in [None, Some("o")] {
                    let Block::Line { text, target, key, mnemonic_at, mnemonic_shift } = link(h.page_words, url, shown_key) else { panic!("a documentation line") };
                    let linked = text.0.iter().find(|seg| seg.link.is_some()).unwrap();
                    assert_eq!(linked.link.as_deref(), Some(url));
                    assert_eq!(linked.text, format!(" {}{}", if shown_key.is_some() { "open documentation · " } else { "" }, h.page_words));
                    assert_eq!(key.as_deref(), shown_key);
                    assert_eq!(mnemonic_at, shown_key.map(|_| 1), "the icon has no source words; o starts the caption after one space");
                    assert!(!mnemonic_shift && target.is_none());
                }
                select(&mut s, &format!("cli:{id}"));
                for (installed, cmd) in [(false, install), (true, login)] {
                    s.set_status(id, status(installed, AuthStatus::SignedOut));
                    for intent in [Intent::Key { chord: Chord::char('y') }, Intent::Invoke { action: "sessions.agents.copy".into() }] {
                        assert!(matches!(s.intent(SECTION, &intent), Outcome::Handled(ref fx) if matches!(&fx[..], [Effect::Notify(_), Effect::Copy(value)] if value == cmd)));
                    }
                }
                for intent in [Intent::Key { chord: Chord::char('o') }, Intent::Invoke { action: "sessions.agents.open".into() }] {
                    assert!(matches!(s.intent(SECTION, &intent), Outcome::Handled(ref fx) if matches!(&fx[..], [Effect::Open(value)] if value == url)));
                }
            }
            let Block::Line { text, key, mnemonic_at, .. } = link("radar guide: agents", GUIDE, None) else { panic!("guide line") };
            assert_eq!(text.0.last().unwrap().link.as_deref(), Some("https://github.com/temet-ai/radar/blob/main/docs/user-guide/agents.md"));
            assert!(key.is_none() && mnemonic_at.is_none());
            assert!(s.take_requests().is_empty(), "copy/open effects do not invoke a provider");
        }

        #[test]
        fn handwritten_setup_controls_use_host_styled_words_and_keep_the_named_chord() {
            let blocks = sheet(&two_ready(), vec![], None);
            let Block::Line { text: refresh, .. } = &blocks[0] else { panic!("refresh instruction") };
            assert_eq!(refresh.words(), "Each agent is its own program with its own sign-in. radar finds it on your PATH; read its status again.");
            let underlined = |text: &Text| text.0.iter().filter(|seg| seg.look.as_ref().is_some_and(|look| look.split_whitespace().any(|word| word == "underline")))
                .map(|seg| seg.text.clone()).collect::<Vec<_>>();
            assert_eq!(underlined(refresh), ["r"]);
            assert_eq!(refresh.0.iter().filter_map(|seg| seg.key_hint.as_deref()).collect::<Vec<_>>(), ["r"]);
            let controls = blocks.iter().find_map(|block| match block {
                Block::Section { title, blocks, .. } if title == "NOTHING IS APPROVED FOR YOU" => Some(blocks),
                _ => None,
            }).expect("permission instructions");
            let Block::Line { text, .. } = &controls[1] else { panic!("permission controls") };
            assert_eq!(text.words(), "approve · session · deny · view diff");
            assert_eq!(underlined(text), ["a", "s", "d", "v"]);
            assert_eq!(text.0.iter().filter_map(|seg| seg.key_hint.as_deref()).collect::<Vec<_>>(), ["a", "s", "d", "v"]);
            assert!(text.0.iter().filter_map(|seg| seg.look.as_deref()).all(|look| look.starts_with("text.muted")));
            let Block::Line { text, .. } = &controls[2] else { panic!("agent-switch instruction") };
            assert_eq!(text.words(), "alt-a switches the agent new sessions start as.");
        }
    }

    fn setup_target(kind: &str, id: &str) -> String {
        copy_target(kind, id)
    }

    #[test]
    fn a_sign_in_under_way_is_first_in_the_sheet_and_a_broken_cli_says_why() {
        let mut s = two_ready();
        s.set_status("copilot", status(true, AuthStatus::SignedOut));
        select(&mut s, "cli:copilot");
        key(&mut s, 'i');
        s.login_event("copilot", LoginEvent::DeviceCode { url: "https://github.com/login/device".into(), code: "ABCD-1234".into() });
        let lines = sheet_lines(&view(&s).detail_blocks);
        assert!(lines.iter().take(4).any(|l| l.contains("ABCD-1234")), "{lines:?}");

        let mut s = two_ready();
        s.set_status("codex", AdapterStatus { error: Some("app-server did not answer".into()), ..signed_in("vimes") });
        select(&mut s, "cli:codex");
        let lines = sheet_lines(&view(&s).detail_blocks);
        assert!(lines[0].contains("app-server did not answer"), "{lines:?}");
    }

    #[test]
    fn a_command_of_your_own_is_added_from_the_card_and_kept_in_the_config() {
        let mut s = two_ready();
        s.set_path(Some(String::new()));
        let rows = |s: &AgentsSettings| match view(s).body {
            Node::List { rows, .. } => rows.iter().map(|r| (r.target.as_str().to_string(), text_of(&r.text))).collect::<Vec<_>>(),
            _ => panic!("a list"),
        };
        // The add row follows the CLIs and is the card's own row.
        let before = rows(&s);
        let at = before.iter().position(|(t, _)| t == "cli:add").expect("an add row");
        assert!(before[at].1.contains("add a command") && before[at - 1].0 == "cli:claude");

        select(&mut s, "cli:add");
        let fx = match s.intent(SECTION, &Intent::Activate { target: Target::new("cli:add") }) {
            Outcome::Handled(fx) => fx,
            Outcome::Ignored => panic!("ignored"),
        };
        assert!(matches!(&fx[..], [Effect::Menu(m)] if m.id == "agents.pty_add"));
        assert!(matches!(&view(&s).detail_blocks[..], [_, ..]), "the add row has the sheet");

        // As it is typed: what the id will be, and whether the program is there.
        let verdict = |s: &AgentsSettings, t: &str| s.typing("agents.pty_add", t).verdict;
        assert!(matches!(verdict(&s, "aider --yes"), Some(radar_core::Verdict::Unsure(w)) if w.contains("not found on PATH") && w.contains("adds aider")));
        assert!(matches!(verdict(&s, "claude"), Some(radar_core::Verdict::Bad(w)) if w.contains("listed already")));
        assert!(matches!(verdict(&s, "tool ghp_abcdefghijklmnopqrstuvwxyz0123456789"), Some(radar_core::Verdict::Bad(w)) if w.contains("token")));

        s.on_menu("agents.pty_add", "/usr/local/bin/Aider --yes --model small").unwrap();
        assert_eq!(
            s.pty_clis(),
            [PtyCli { id: "aider".into(), name: "aider".into(), command: "/usr/local/bin/Aider".into(), args: vec!["--yes".into(), "--model".into(), "small".into()], login: None }]
        );
        let after = rows(&s);
        assert!(after.iter().any(|(t, _)| t == "cli:aider"), "{after:?}");
        let table = s.config().unwrap().to_string();
        assert!(table.contains("aider") && table.contains("--yes"), "{table}");
        // The same name twice, a native one, and a token are refused and change nothing.
        for bad in ["aider", "codex --x", "x sk-abcdefghijklmnop"] {
            let fx = s.on_menu("agents.pty_add", bad).unwrap();
            assert!(matches!(&fx[..], [Effect::Notice(w)] if w.tone == radar_core::view::chrome::NoticeTone::Error && w.contains("not saved")), "{bad}: {fx:?}");
        }
        assert_eq!(s.pty_clis().len(), 1);
        assert!(s.on_menu("agents.pty_add", "  ").unwrap().is_empty());
        assert_eq!(key(&mut s, 'a').len(), 1, "a opens the field from any CLI row");
    }

    /// A host that offers the step, as the binary's does.
    struct Hosted(AgentsSettings);

    impl radar_core::Plugin for Hosted {
        fn id(&self) -> &'static str {
            "sessions"
        }
        fn name(&self) -> &'static str {
            "Agents"
        }
        fn configure(&mut self, t: Option<&toml::Value>) -> anyhow::Result<()> {
            self.0.configure(t)
        }
        fn config(&self) -> Option<toml::Value> {
            self.0.config()
        }
        fn sections(&self) -> Vec<radar_core::SectionSpec> {
            crate::settings::sections()
        }
        fn section_view(&self, section: &str, cx: &ViewCx<'_>) -> Option<SectionView> {
            self.0.section_view(section, cx)
        }
        fn intent(&mut self, scope: &str, _pane: radar_core::Pane, intent: &Intent) -> Outcome {
            self.0.intent(scope, intent)
        }
        fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
            self.0.on_menu(menu, value).unwrap_or_default()
        }
        fn setup_step(&self) -> Option<SetupStep> {
            Some(step(&self.0, true))
        }
        fn setup_finds(&self) -> SetupFinds {
            finds()
        }
    }

    #[test]
    fn radar_setup_agents_draws_the_card_and_the_sheet_at_both_sizes() {
        for (w, h) in [(80, 24), (160, 40)] {
            let mut s = two_ready();
            s.on_menu("agents.provider", "claude").unwrap();
            let mut host = radar_core::Host::new(vec![Box::new(Hosted(s))], radar_core::plugin::DEMO_NOW).unwrap();
            host.open_setup(Some("agents")).unwrap();
            assert!(host.setup_step_ids().contains(&"agents".to_string()), "{:?}", host.setup_step_ids());
            let text = radar_core::shell::to_text(&host, w, h);
            if std::env::var_os("RADAR_SHOW").is_some() {
                println!("{text}");
            }
            for want in ["agents", "Claude Agent", "signed in", "not installed", "add a command"] {
                assert!(text.contains(want), "{want} at {w}x{h}:\n{text}");
            }
        }
    }
}
