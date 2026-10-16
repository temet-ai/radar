use std::collections::VecDeque;

use super::*;

// ------------------------------------------------------------------ helpers

const DAY: i64 = 86_400;
/// A Tuesday.
const TUE: Date = Date {
    year: 2026,
    month: 10,
    day: 13,
};

/// Chance that says what a test tells it to, then `rest` for ever.
struct Rolls {
    queue: VecDeque<f64>,
    rest: f64,
}

impl Rolls {
    fn new(rolls: &[f64]) -> Self {
        Self {
            queue: rolls.iter().copied().collect(),
            rest: 0.5,
        }
    }

    fn always(value: f64) -> Self {
        Self {
            queue: VecDeque::new(),
            rest: value,
        }
    }
}

impl Rng for Rolls {
    fn next_u64(&mut self) -> u64 {
        let value = self.queue.pop_front().unwrap_or(self.rest);
        (value * 18_446_744_073_709_551_616.0) as u64
    }
}

/// Motion full, emoji drawn, at `hour:minute` of `date`.
fn at(date: Date, hour: u32, minute: u32) -> Context {
    let epoch = date.days() * DAY + i64::from(hour) * 3600 + i64::from(minute) * 60;
    let mut ctx = Context::at(epoch, 0);
    ctx.motion_full = true;
    ctx.emoji_ok = true;
    ctx
}

fn now() -> Context {
    at(TUE, 9, 5)
}

/// A state whose last start was today (not the first of the day).
fn state_today(ctx: &Context) -> State {
    State {
        starts: 10,
        last_day: Some(ctx.date.iso()),
        starts_today: 3,
        ..State::default()
    }
}

fn fresh_facts(ctx: &Context) -> Facts {
    Facts::new(ctx.epoch)
}

fn go(
    dict: &Dictionary,
    state: &mut State,
    facts: &Facts,
    ctx: &Context,
    rolls: &mut Rolls,
) -> Option<Picked> {
    pick(dict, &UserConfig::default(), state, facts, ctx, rolls)
}

/// A dictionary of a few groups of one's own.
fn tiny(lines: &str) -> Dictionary {
    let head = r#"
version = 1
[draw]
weights = { alpha = 10, beta = 10, facts = 10, time = 10, tips = 10 }
same_group_max = 0
[widest]
who = "xxxxxxxxxxxxxxxx"
short = "x"
time = "03:00"
"#;
    Dictionary::parse(&format!("{head}\n{lines}")).expect("the test dictionary parses")
}

fn line(group: &str, id: &str, text: &str, extra: &str) -> String {
    format!("[[line]]\ngroup = \"{group}\"\nid = \"{id}\"\ntext = \"{text}\"\n{extra}\n")
}

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("radar-start-lines-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("a temp folder");
    dir
}

// ------------------------------------------------------- the shipped dictionary

#[test]
fn the_shipped_dictionary_parses_with_unique_ids_and_every_group() {
    let dict = Dictionary::parse(SHIPPED).expect("parses");
    let ids: BTreeSet<&str> = dict.lines().iter().map(|l| l.id.as_str()).collect();
    assert_eq!(ids.len(), dict.lines().len(), "an id is used twice");
    let mut groups: BTreeMap<&str, usize> = BTreeMap::new();
    for l in dict.lines() {
        *groups.entry(l.group.as_str()).or_default() += 1;
    }
    let expected = [
        ("plain", 6),
        ("first_run", 1),
        ("day", 22),
        ("animated", 5),
        ("rare", 11),
        ("scope", 20),
        ("work", 26),
        ("facts", 24),
        ("agents", 14),
        ("time", 15),
        ("glyph", 8),
        ("tips", 17),
    ];
    assert_eq!(groups, expected.into_iter().collect::<BTreeMap<_, _>>());
    assert_eq!(dict.lines().len(), 169);
}

#[test]
fn every_shipped_line_fits_56_cells_at_its_widest() {
    let dict = Dictionary::shipped();
    let too_wide: Vec<_> = dict
        .lines()
        .iter()
        .filter(|l| l.cells > MAX_CELLS)
        .map(|l| &l.id)
        .collect();
    assert!(too_wide.is_empty(), "too wide: {too_wide:?}");
    // The widths agree with the design's own checker: the same total, the same widest.
    let total: usize = dict.lines().iter().map(|l| l.cells).sum();
    let widest = dict.lines().iter().map(|l| l.cells).max();
    assert_eq!((total, widest), (7135, Some(56)));
    // Five lines use all 56 cells.
    let full: BTreeSet<&str> = dict
        .lines()
        .iter()
        .filter(|l| l.cells == 56)
        .map(|l| l.id.as_str())
        .collect();
    assert_eq!(
        full,
        BTreeSet::from([
            "crashed",
            "first-try",
            "holidays",
            "it-depends",
            "temporary"
        ])
    );
}

#[test]
fn plain_lines_are_never_a_joke_or_an_emoji() {
    let dict = Dictionary::shipped();
    for l in dict.lines().iter().filter(|l| l.group == "plain") {
        assert!(!l.emoji && l.anim.is_none(), "{} is not plain", l.id);
    }
}

#[test]
fn the_shipped_odds_and_weights_are_the_designs() {
    let dict = Dictionary::shipped();
    assert_eq!(
        dict.rare,
        Odds {
            chance: 0.01,
            min_days_between: 7
        }
    );
    assert_eq!(
        dict.animated,
        Odds {
            chance: 0.0033,
            min_days_between: 30
        }
    );
    let weights: Vec<_> = [
        "scope", "work", "facts", "tips", "agents", "time", "glyph", "mine",
    ]
    .iter()
    .map(|g| dict.draw.weights[*g])
    .collect();
    assert_eq!(weights, [20, 24, 22, 10, 10, 9, 5, 20]);
    assert_eq!(
        (dict.draw.tips_early, dict.draw.same_group_max),
        (Some(25), 2)
    );
    assert!(dict.draw.facts_unchanged_waits);
}

// -------------------------------------------------------------------- scenarios

#[derive(Clone, Copy)]
enum V {
    N(i64),
    B(bool),
    S(&'static str),
}

impl From<V> for Value {
    fn from(v: V) -> Self {
        match v {
            V::N(n) => Value::Num(n),
            V::B(b) => Value::Bool(b),
            V::S(s) => Value::Str(s.to_string()),
        }
    }
}

struct Scenario {
    label: &'static str,
    facts: &'static [(&'static str, V)],
    first_run: bool,
    /// The group whose bag has the expected line next.
    bag: Option<&'static str>,
    rolls: &'static [f64],
    id: &'static str,
    group: &'static str,
    text: Option<&'static str>,
    anim: Option<&'static str>,
}

const BASE: Scenario = Scenario {
    label: "",
    facts: &[],
    first_run: false,
    bag: None,
    rolls: &[],
    id: "",
    group: "",
    text: None,
    anim: None,
};

/// A fortnight of starts: what each held, and the line the rules give.
/// The rolls are those of the order's chance; the draw's land on the group.
fn scenarios() -> Vec<Scenario> {
    use V::{B, N, S};
    vec![
        Scenario {
            label: "1 first start ever",
            facts: &[("start.count", N(1))],
            first_run: true,
            id: "first-contact",
            group: "first_run",
            text: Some("First contact. Let's see what's out there."),
            ..BASE
        },
        Scenario {
            label: "2 nothing special: scope's bag",
            facts: &[
                ("start.count", N(2)),
                ("reviews.count", N(3)),
                ("hour", N(9)),
                ("weekday", S("tue")),
            ],
            bag: Some("scope"),
            rolls: &[0.12],
            id: "horizon",
            group: "scope",
            ..BASE
        },
        Scenario {
            label: "9 first start of the day after ten",
            facts: &[
                ("start.count", N(9)),
                ("start.first_today", B(true)),
                ("hour", N(10)),
                ("weekday", S("wed")),
            ],
            bag: Some("time"),
            rolls: &[0.90],
            id: "late-morning",
            group: "time",
            ..BASE
        },
        Scenario {
            label: "12 tips weigh 25 early",
            facts: &[
                ("start.count", N(12)),
                ("hour", N(14)),
                ("weekday", S("wed")),
            ],
            bag: Some("tips"),
            rolls: &[0.71],
            id: "tip-goto",
            group: "tips",
            ..BASE
        },
        Scenario {
            label: "17 friday afternoon",
            facts: &[
                ("start.count", N(17)),
                ("hour", N(15)),
                ("weekday", S("fri")),
                ("mine.open", N(4)),
            ],
            bag: Some("time"),
            rolls: &[0.90],
            id: "friday-pm",
            group: "time",
            ..BASE
        },
        Scenario {
            label: "19 a long weekend away: the gap",
            facts: &[
                ("start.count", N(19)),
                ("start.first_today", B(true)),
                ("start.gap_days", N(3)),
                ("changes.since_last", N(23)),
                ("hour", N(8)),
                ("weekday", S("mon")),
            ],
            id: "gap-long",
            group: "day",
            text: Some("Welcome back. 23 things happened. radar read them all."),
            ..BASE
        },
        Scenario {
            label: "25 a pipeline failed: plain",
            facts: &[
                ("start.count", N(25)),
                ("pipelines.failed_new", N(1)),
                ("hour", N(11)),
                ("weekday", S("mon")),
            ],
            id: "pipes-failed",
            group: "plain",
            text: Some("1 pipeline of yours failed since your last start."),
            ..BASE
        },
        Scenario {
            label: "31 one of yours can merge, with its key",
            facts: &[
                ("start.count", N(31)),
                ("mine.ready", N(1)),
                ("mr", S("!412")),
                ("hour", N(13)),
                ("weekday", S("tue")),
            ],
            bag: Some("facts"),
            rolls: &[0.49],
            id: "ready-to-merge",
            group: "facts",
            text: Some("!412 is approved and green. m merges it."),
            ..BASE
        },
        Scenario {
            label: "42 a milestone on the first start of its day",
            facts: &[
                ("start.count", N(42)),
                ("start.first_today", B(true)),
                ("hour", N(9)),
                ("weekday", S("wed")),
            ],
            id: "start-42",
            group: "day",
            ..BASE
        },
        Scenario {
            label: "48 agents, and this one is Claude",
            facts: &[
                ("start.count", N(48)),
                ("agents.setup", N(2)),
                ("agents.has_claude", B(true)),
                ("agents.has_codex", B(true)),
                ("hour", N(16)),
                ("weekday", S("wed")),
            ],
            bag: Some("agents"),
            rolls: &[0.75],
            id: "claude-right",
            group: "agents",
            ..BASE
        },
        Scenario {
            label: "55 a rare line, a week since the last",
            facts: &[
                ("start.count", N(55)),
                ("hour", N(11)),
                ("weekday", S("thu")),
            ],
            bag: Some("rare"),
            rolls: &[0.004],
            id: "plot-twist",
            group: "rare",
            ..BASE
        },
        Scenario {
            label: "63 an event: fire",
            facts: &[
                ("start.count", N(63)),
                ("pipelines.failing", N(0)),
                ("hour", N(16)),
                ("weekday", S("fri")),
            ],
            rolls: &[0.002],
            id: "this-is-fine",
            group: "animated",
            text: Some("This is fine."),
            anim: Some("fire"),
            ..BASE
        },
        Scenario {
            label: "70 an emoji on its own",
            facts: &[
                ("start.count", N(70)),
                ("hour", N(9)),
                ("weekday", S("mon")),
            ],
            bag: Some("glyph"),
            rolls: &[0.97],
            id: "coffee",
            group: "glyph",
            text: Some("☕"),
            ..BASE
        },
        Scenario {
            label: "77 the work group's bag",
            facts: &[
                ("start.count", N(77)),
                ("hour", N(10)),
                ("weekday", S("tue")),
            ],
            bag: Some("work"),
            rolls: &[0.33],
            id: "narrator",
            group: "work",
            ..BASE
        },
        Scenario {
            label: "80 a date line, made better by a fact",
            facts: &[
                ("start.count", N(80)),
                ("start.first_today", B(true)),
                ("date", S("10-31")),
                ("git.stalest_branch_days", N(212)),
                ("hour", N(10)),
                ("weekday", S("sat")),
            ],
            id: "halloween-stale",
            group: "day",
            text: Some("🎃 A branch untouched for 212 days. It stirs."),
            ..BASE
        },
        Scenario {
            label: "91 a name cut to 16 cells",
            facts: &[
                ("start.count", N(91)),
                ("git.repos", N(6)),
                ("git.stashes", N(4)),
                ("repo", S("ankh-morpork/watch")),
                ("hour", N(10)),
                ("weekday", S("wed")),
            ],
            bag: Some("facts"),
            rolls: &[0.55],
            id: "stashes",
            group: "facts",
            text: Some("ankh-morpork/wa…: 4 stashes. One matters."),
            ..BASE
        },
    ]
}

/// The state and the facts the scenario describes, and the pick it gives.
fn run_scenario(s: &Scenario, dict: &Dictionary) -> Option<Picked> {
    let ctx = {
        let mut ctx = now();
        ctx.first_run = s.first_run;
        ctx
    };
    let mut facts = fresh_facts(&ctx);
    for (name, v) in s.facts {
        facts.set(name, *v);
    }
    let count = s
        .facts
        .iter()
        .find_map(|(n, v)| (*n == "start.count").then_some(*v));
    let first_today = s
        .facts
        .iter()
        .any(|(n, v)| *n == "start.first_today" && matches!(v, V::B(true)));
    let mut state = state_today(&ctx);
    // A draw is preceded by the rare line's roll, which misses.
    let rolls: Vec<f64> = if s.bag.is_some() && s.group != "rare" {
        [&[0.99][..], s.rolls].concat()
    } else {
        s.rolls.to_vec()
    };
    if let Some(V::N(n)) = count {
        state.starts = n as u64 - 1;
    }
    if first_today {
        state.last_day = Some(Date::from_days(ctx.date.days() - 1).iso());
    }
    if s.group == "rare" {
        state.last_rare = Some(Date::from_days(ctx.date.days() - 19).iso());
    }
    if let Some(group) = s.bag {
        state.bags.insert(group.to_string(), vec![s.id.to_string()]);
    }
    go(dict, &mut state, &facts, &ctx, &mut Rolls::new(&rolls))
}

#[test]
fn the_design_sessions_worked_starts_give_the_lines_it_expects() {
    let dict = Dictionary::shipped();
    for s in scenarios() {
        let got = run_scenario(&s, &dict).unwrap_or_else(|| panic!("{}: no line", s.label));
        assert_eq!(
            (got.id.as_str(), got.group.as_str()),
            (s.id, s.group),
            "{}",
            s.label
        );
        assert_eq!(got.anim.as_deref(), s.anim, "{}", s.label);
        if let Some(text) = s.text {
            assert_eq!(got.text(), text, "{}", s.label);
        }
        assert!(
            got.cells() <= MAX_CELLS,
            "{}: {} cells",
            s.label,
            got.cells()
        );
    }
}

#[test]
fn a_scenarios_line_is_the_first_the_order_gives_not_just_an_eligible_one() {
    // Start 19 and start 80 hold facts a draw group could use too; the day
    // line still wins. Start 25 holds a day's facts and a failure: plain wins.
    let dict = Dictionary::shipped();
    let ctx = now();
    let mut facts = fresh_facts(&ctx);
    facts
        .set("start.first_today", true)
        .set("start.gap_days", 3)
        .set("changes.since_last", 23);
    facts.set("pipelines.failed_new", 1);
    let mut state = state_today(&ctx);
    state.last_day = Some("2026-10-01".into());
    let got = go(&dict, &mut state, &facts, &ctx, &mut Rolls::always(0.0)).expect("a line");
    assert_eq!(got.id, "pipes-failed");
}

#[test]
fn the_key_in_a_line_is_a_span_of_its_own() {
    let dict = Dictionary::shipped();
    let s = &scenarios()[7];
    let got = run_scenario(s, &dict).expect("a line");
    assert_eq!(
        got.spans,
        vec![
            Span {
                text: "!412".into(),
                kind: SpanKind::Value
            },
            Span {
                text: " is approved and green. ".into(),
                kind: SpanKind::Text
            },
            Span {
                text: "m".into(),
                kind: SpanKind::Key
            },
            Span {
                text: " merges it.".into(),
                kind: SpanKind::Text
            },
        ]
    );
}

#[test]
fn the_event_that_is_not_built_never_shows() {
    // Start 88 held an empty review queue, which a tumbleweed animation suits;
    // only fire is built, so the event is not eligible and nothing animates.
    let dict = Dictionary::shipped();
    let ctx = now();
    let mut facts = fresh_facts(&ctx);
    facts.set("reviews.count", 0);
    for _ in 0..40 {
        let mut state = state_today(&ctx);
        let got = go(&dict, &mut state, &facts, &ctx, &mut Rolls::always(0.0)).expect("a line");
        assert_ne!(got.id, "tumbleweed");
        assert_eq!(got.anim, None);
        assert_ne!(got.group, "animated");
    }
    for l in dict
        .lines()
        .iter()
        .filter(|l| l.anim.as_deref().is_some_and(|a| !BUILT_ANIMS.contains(&a)))
    {
        let mut state = state_today(&ctx);
        let mut facts = fresh_facts(&ctx);
        facts
            .set("start.first_today", true)
            .set("date", "12-25")
            .set("start.count", 1000);
        facts.set("hour", 8);
        let got = go(&dict, &mut state, &facts, &ctx, &mut Rolls::always(0.0));
        assert_ne!(
            got.map(|p| p.id),
            Some(l.id.clone()),
            "{} animates with nothing built",
            l.id
        );
    }
}

#[test]
fn start_18_is_time_again_and_start_19_cannot_be_a_third_time() {
    let dict = Dictionary::shipped();
    let ctx = at(TUE, 15, 31);
    let mut facts = fresh_facts(&ctx);
    facts
        .set("hour", 15)
        .set("weekday", "fri")
        .set("start.since_quit_min", 1);
    let mut state = state_today(&ctx);
    state.recent = vec!["time".into()];
    state.shown.insert(
        "friday-pm".into(),
        Shown {
            day: ctx.date.iso(),
            values: BTreeMap::new(),
        },
    );
    state.bags.insert("time".into(), vec!["off-on".into()]);
    // The first roll is the rare line's, and misses.
    let got = go(
        &dict,
        &mut state,
        &facts,
        &ctx,
        &mut Rolls::new(&[0.99, 0.88]),
    )
    .expect("a line");
    assert_eq!((got.id.as_str(), got.group.as_str()), ("off-on", "time"));
    assert_eq!(state.recent, ["time", "time"]);
    // The time group's bag still gives; the rule is what stops it.
    state
        .bags
        .insert("time".into(), vec!["off-on".into(), "weekend".into()]);
    for _ in 0..30 {
        let mut then = state.clone();
        let got = go(
            &dict,
            &mut then,
            &facts,
            &ctx,
            &mut Rolls::new(&[0.99, 0.88]),
        )
        .expect("a line");
        assert_ne!(got.group, "time");
    }
}

// ----------------------------------------------------------------- order rules

#[test]
fn plain_beats_first_run_beats_day_beats_the_rest() {
    let dict = Dictionary::shipped();
    let mut ctx = now();
    ctx.first_run = true;
    let mut facts = fresh_facts(&ctx);
    facts.set("pipelines.failed_new", 2).set("start.count", 42);
    let mut state = State::default();
    let got = go(&dict, &mut state, &facts, &ctx, &mut Rolls::always(0.0)).expect("a line");
    assert_eq!(got.id, "pipes-failed");
    assert_eq!(
        got.text(),
        "2 pipelines of yours failed since your last start."
    );

    // Nothing failed: the first run is its own welcome, over the milestone.
    let mut facts = fresh_facts(&ctx);
    facts.set("start.count", 42);
    let got = go(
        &dict,
        &mut State::default(),
        &facts,
        &ctx,
        &mut Rolls::always(0.0),
    )
    .expect("a line");
    assert_eq!(got.id, "first-contact");

    // Not the first run: the first start of the day gives the milestone.
    ctx.first_run = false;
    let got = go(
        &dict,
        &mut State::default(),
        &facts,
        &ctx,
        &mut Rolls::always(0.0),
    )
    .expect("a line");
    assert_eq!((got.id.as_str(), got.group.as_str()), ("start-42", "day"));
}

#[test]
fn the_most_important_plain_line_wins_in_file_order() {
    let dict = Dictionary::shipped();
    let ctx = now();
    let mut facts = fresh_facts(&ctx);
    facts
        .set("pipelines.failed_new", 1)
        .set("net.offline", true)
        .set("auth.expired", true);
    facts.set("service", "Tavern");
    let got = go(
        &dict,
        &mut state_today(&ctx),
        &facts,
        &ctx,
        &mut Rolls::always(0.0),
    )
    .expect("a line");
    assert_eq!(got.id, "signed-out");
    assert_eq!(
        got.text(),
        "Tavern wants you to sign in again: Settings / Tavern."
    );
}

#[test]
fn a_day_line_shows_on_the_first_start_of_a_local_day_only() {
    let dict = Dictionary::shipped();
    let ctx = now();
    let mut facts = fresh_facts(&ctx);
    facts.set("start.count", 100);
    let mut state = state_today(&ctx);
    let got = go(&dict, &mut state, &facts, &ctx, &mut Rolls::always(0.5)).expect("a line");
    assert_ne!(got.group, "day", "the second start of the day");
    // The next morning is a first start.
    let tomorrow = at(Date::from_days(TUE.days() + 1), 8, 0);
    let mut facts = fresh_facts(&tomorrow);
    facts.set("start.count", 100);
    let got = go(
        &dict,
        &mut state,
        &facts,
        &tomorrow,
        &mut Rolls::always(0.5),
    )
    .expect("a line");
    assert_eq!(got.id, "start-100");
    // And a start after it, the same day, is not.
    let got = go(
        &dict,
        &mut state,
        &facts,
        &tomorrow,
        &mut Rolls::always(0.5),
    )
    .expect("a line");
    assert_ne!(got.group, "day");
}

#[test]
fn a_day_counts_in_the_users_zone_not_in_utc() {
    // 23:30 UTC is already the next day three hours east.
    let epoch = TUE.days() * DAY + 23 * 3600 + 30 * 60;
    let east = Context::at(epoch, 3 * 3600);
    let utc = Context::at(epoch, 0);
    assert_eq!((east.date.day, east.hour, east.minute), (14, 2, 30));
    assert_eq!((utc.date.day, utc.hour), (13, 23));
    let mut state = State {
        last_day: Some(utc.date.iso()),
        starts: 5,
        ..State::default()
    };
    let tick = state.tick(&east);
    assert!(tick.first_today);
    assert_eq!(tick.gap_days, 1);
}

#[test]
fn the_echo_takes_the_rare_line_and_no_event() {
    let dict = Dictionary::shipped();
    let mut ctx = now();
    ctx.echo_rare = true;
    let mut facts = fresh_facts(&ctx);
    facts.set("pipelines.failing", 0);
    let mut state = state_today(&ctx);
    let mut rolls = Rolls::always(0.0);
    let got = go(&dict, &mut state, &facts, &ctx, &mut rolls).expect("a line");
    assert_eq!(
        (got.id.as_str(), got.group.as_str(), got.anim),
        ("not-a-plane", "rare", None)
    );
    assert_eq!(state.last_event, None, "the fire did not run");
    assert_eq!(
        state.last_rare, None,
        "the echo is not the start's rare roll"
    );

    // Without the echo the same facts and the same luck give the event.
    ctx.echo_rare = false;
    let got = go(
        &dict,
        &mut state_today(&ctx),
        &facts,
        &ctx,
        &mut Rolls::always(0.0),
    )
    .expect("a line");
    assert_eq!(
        (got.id.as_str(), got.anim.as_deref()),
        ("this-is-fine", Some("fire"))
    );
    // And the echo's line is never a plain rare draw.
    for i in 0..60 {
        let mut state = state_today(&ctx);
        let got = pick(
            &dict,
            &UserConfig::default(),
            &mut state,
            &fresh_facts(&ctx),
            &ctx,
            &mut SplitMix64::new(i),
        );
        assert_ne!(got.map(|p| p.id), Some("not-a-plane".to_string()));
    }
}

#[test]
fn an_event_needs_full_motion_calm_pipelines_and_a_month() {
    let dict = Dictionary::shipped();
    let ctx = now();
    let mut calm = fresh_facts(&ctx);
    calm.set("pipelines.failing", 0);
    let event = |ctx: &Context, facts: &Facts, last: Option<i64>, luck: f64| {
        let mut state = state_today(ctx);
        state.last_event = last.map(|d| Date::from_days(ctx.date.days() - d).iso());
        let got = go(&dict, &mut state, facts, ctx, &mut Rolls::new(&[luck]));
        (got, state)
    };

    let (got, state) = event(&ctx, &calm, None, 0.0);
    assert_eq!(got.expect("a line").anim.as_deref(), Some("fire"));
    assert_eq!(state.last_event.as_deref(), Some("2026-10-13"));
    // The roll is 0.0033 and no more.
    assert!(
        event(&ctx, &calm, None, 0.0032)
            .0
            .is_some_and(|p| p.anim.is_some())
    );
    assert!(
        event(&ctx, &calm, None, 0.0034)
            .0
            .is_none_or(|p| p.anim.is_none())
    );
    // A month apart.
    assert!(
        event(&ctx, &calm, Some(30), 0.0)
            .0
            .is_some_and(|p| p.anim.is_some())
    );
    assert!(
        event(&ctx, &calm, Some(29), 0.0)
            .0
            .is_none_or(|p| p.anim.is_none())
    );
    // Motion full only.
    let mut subtle = ctx.clone();
    subtle.motion_full = false;
    assert!(
        event(&subtle, &calm, None, 0.0)
            .0
            .is_none_or(|p| p.anim.is_none())
    );
    // Never while a pipeline fails, even when the line's own condition would
    // let it: a line with no condition of its own.
    let mut failing = fresh_facts(&ctx);
    failing.set("pipelines.failing", 2);
    assert!(
        event(&ctx, &failing, None, 0.0)
            .0
            .is_none_or(|p| p.anim.is_none())
    );
    let own = Dictionary::shipped()
        .with_user("[[animated]]\nid = \"blaze\"\ntext = \"Blaze.\"\nanim = \"fire\"\n")
        .expect("a user event");
    let mut state = state_today(&ctx);
    let got = go(
        &own,
        &mut state,
        &failing,
        &ctx,
        &mut Rolls::new(&[0.0, 0.0]),
    );
    assert!(got.is_none_or(|p| p.anim.is_none()));
    // Never over a plain line: that is the line.
    let mut plain = calm.clone();
    plain.set("pipelines.failed_new", 1);
    let got = event(&ctx, &plain, None, 0.0).0.expect("a line");
    assert_eq!((got.group.as_str(), got.anim), ("plain", None));
}

#[test]
fn a_rare_line_is_one_in_a_hundred_and_a_week_apart() {
    let dict = Dictionary::shipped();
    let ctx = now();
    let facts = fresh_facts(&ctx);
    let rare = |last: Option<i64>, luck: f64| {
        let mut state = state_today(&ctx);
        state.last_rare = last.map(|d| Date::from_days(ctx.date.days() - d).iso());
        let got = go(&dict, &mut state, &facts, &ctx, &mut Rolls::new(&[luck]));
        (got.is_some_and(|p| p.group == "rare"), state)
    };
    let (shown, state) = rare(None, 0.0099);
    assert!(shown);
    assert_eq!(state.last_rare.as_deref(), Some("2026-10-13"));
    assert!(!rare(None, 0.0101).0);
    assert!(rare(Some(7), 0.0).0);
    assert!(!rare(Some(6), 0.0).0);
}

#[test]
fn the_rare_lines_go_round_before_one_repeats() {
    let dict = Dictionary::shipped();
    let ctx = now();
    let facts = fresh_facts(&ctx);
    let mut state = state_today(&ctx);
    let mut shown = Vec::new();
    // Nine can show: the echo's line and the one that needs 200 starts cannot.
    for _ in 0..9 {
        state.last_rare = None;
        let got = go(&dict, &mut state, &facts, &ctx, &mut Rolls::always(0.0)).expect("a line");
        assert_eq!(got.group, "rare");
        shown.push(got.id);
    }
    let unique: BTreeSet<_> = shown.iter().collect();
    assert_eq!(unique.len(), 9, "{shown:?}");
    assert!(
        !unique.contains(&"not-a-plane".to_string()) && !unique.contains(&"nervous".to_string())
    );
}

// -------------------------------------------------------------------- the draw

#[test]
fn the_draw_walks_the_weights_of_the_groups_that_have_a_line() {
    // alpha 10, beta 10 (the head's weights): the roll lands by position.
    let dict = tiny(&(line("alpha", "a", "A", "") + &line("beta", "b", "B", "")));
    let ctx = now();
    let facts = fresh_facts(&ctx);
    for (roll, group) in [
        (0.0, "alpha"),
        (0.49, "alpha"),
        (0.5, "beta"),
        (0.99, "beta"),
    ] {
        let got = go(
            &dict,
            &mut state_today(&ctx),
            &facts,
            &ctx,
            &mut Rolls::new(&[roll]),
        )
        .expect("a line");
        assert_eq!(got.group, group, "roll {roll}");
    }
    // A group with no eligible line takes no part.
    let dict =
        tiny(&(line("alpha", "a", "A", "when = [\"x.y >= 1\"]") + &line("beta", "b", "B", "")));
    for roll in [0.0, 0.5, 0.99] {
        let got = go(
            &dict,
            &mut state_today(&ctx),
            &facts,
            &ctx,
            &mut Rolls::new(&[roll]),
        )
        .expect("a line");
        assert_eq!(got.group, "beta");
    }
}

#[test]
fn tips_weigh_25_for_the_first_30_starts_then_10() {
    let dict = Dictionary::shipped();
    let cfg = UserConfig::default();
    let early = weight_of(&dict, &cfg, "tips", 30);
    let later = weight_of(&dict, &cfg, "tips", 31);
    assert_eq!((early, later), (25, 10));
    let own = UserConfig {
        weights: BTreeMap::from([("tips".to_string(), 3)]),
        ..UserConfig::default()
    };
    assert_eq!(
        (
            weight_of(&dict, &own, "tips", 5),
            weight_of(&dict, &own, "mine", 5)
        ),
        (3, 20)
    );
}

#[test]
fn a_bag_is_used_up_before_a_line_repeats() {
    let dict = tiny(
        &(line("alpha", "a1", "One", "")
            + &line("alpha", "a2", "Two", "")
            + &line("alpha", "a3", "Three", "")),
    );
    let ctx = now();
    let facts = fresh_facts(&ctx);
    let mut state = state_today(&ctx);
    let mut luck = SplitMix64::new(7);
    let mut shown = Vec::new();
    for _ in 0..9 {
        let got = pick(
            &dict,
            &UserConfig::default(),
            &mut state,
            &facts,
            &ctx,
            &mut luck,
        )
        .expect("a line");
        shown.push(got.id);
    }
    for round in shown.chunks(3) {
        let unique: BTreeSet<_> = round.iter().collect();
        assert_eq!(unique.len(), 3, "{shown:?}");
    }
    // Across the seam, the line shown last does not come first again.
    for pair in shown.windows(2) {
        assert_ne!(pair[0], pair[1], "{shown:?}");
    }
}

#[test]
fn a_bag_goes_on_where_it_stopped_after_a_restart() {
    let dict = tiny(
        &(line("alpha", "a1", "One", "")
            + &line("alpha", "a2", "Two", "")
            + &line("alpha", "a3", "Three", "")),
    );
    let ctx = now();
    let facts = fresh_facts(&ctx);
    let dir = temp_dir();
    let path = dir.join("state").join("start-lines.toml");
    let mut state = state_today(&ctx);
    let first = go(&dict, &mut state, &facts, &ctx, &mut Rolls::always(0.3)).expect("a line");
    save_state(&path, &state).expect("saved");
    let mut state = load_state(&path);
    let second = go(&dict, &mut state, &facts, &ctx, &mut Rolls::always(0.3)).expect("a line");
    assert_ne!(first.id, second.id);
    assert_eq!(state.bags["alpha"].len(), 1);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn never_three_starts_running_from_one_group() {
    let dict = Dictionary::parse(
        &(String::from("[draw]\nweights = { alpha = 100, beta = 1 }\nsame_group_max = 2\n")
            + &line("alpha", "a1", "One", "")
            + &line("alpha", "a2", "Two", "")
            + &line("alpha", "a3", "Three", "")
            + &line("beta", "b1", "Bee", "")
            + &line("beta", "b2", "Bees", "")),
    )
    .expect("parses");
    let ctx = now();
    let facts = fresh_facts(&ctx);
    let mut state = state_today(&ctx);
    let mut groups = Vec::new();
    for _ in 0..9 {
        // Luck that always asks for the first group: the rule is the only thing against it.
        let got = go(&dict, &mut state, &facts, &ctx, &mut Rolls::always(0.0)).expect("a line");
        groups.push(got.group);
    }
    assert_eq!(
        groups,
        [
            "alpha", "alpha", "beta", "alpha", "alpha", "beta", "alpha", "alpha", "beta"
        ]
    );
}

#[test]
fn the_rule_gives_way_rather_than_show_nothing() {
    let dict = tiny(&line("alpha", "a1", "One", ""))
        .with_user("[draw]\nsame_group_max = 2\n")
        .expect("merges");
    assert_eq!(dict.draw.same_group_max, 2);
    let ctx = now();
    let facts = fresh_facts(&ctx);
    let mut state = state_today(&ctx);
    for _ in 0..6 {
        assert!(go(&dict, &mut state, &facts, &ctx, &mut Rolls::always(0.0)).is_some());
    }
}

#[test]
fn a_time_line_shows_once_a_day() {
    let dict = tiny(&(line("time", "t1", "Tuesday.", "")));
    let ctx = now();
    let facts = fresh_facts(&ctx);
    let mut state = state_today(&ctx);
    assert!(go(&dict, &mut state, &facts, &ctx, &mut Rolls::always(0.0)).is_some());
    assert!(go(&dict, &mut state, &facts, &ctx, &mut Rolls::always(0.0)).is_none());
    let later = at(TUE, 17, 0);
    assert!(go(&dict, &mut state, &facts, &later, &mut Rolls::always(0.0)).is_none());
    let tomorrow = at(Date::from_days(TUE.days() + 1), 9, 0);
    assert!(
        go(
            &dict,
            &mut state,
            &fresh_facts(&tomorrow),
            &tomorrow,
            &mut Rolls::always(0.0)
        )
        .is_some()
    );
}

#[test]
fn another_time_line_may_show_the_same_day() {
    let dict = tiny(&(line("time", "t1", "One.", "") + &line("time", "t2", "Two.", "")));
    let ctx = now();
    let facts = fresh_facts(&ctx);
    let mut state = state_today(&ctx);
    let a = go(&dict, &mut state, &facts, &ctx, &mut Rolls::always(0.0)).expect("a line");
    let b = go(&dict, &mut state, &facts, &ctx, &mut Rolls::always(0.0)).expect("a line");
    assert_ne!(a.id, b.id);
    assert!(go(&dict, &mut state, &facts, &ctx, &mut Rolls::always(0.0)).is_none());
}

#[test]
fn a_facts_line_waits_while_its_numbers_are_those_of_the_last_time() {
    let text = "{n:review} waiting";
    let dict = tiny(&line("facts", "q", text, "when = [\"reviews.count >= 1\"]"));
    let ctx = now();
    let mut state = state_today(&ctx);
    let queue = |n: i64| fresh_facts(&ctx).with("reviews.count", n);
    let mut show =
        |n: i64| go(&dict, &mut state, &queue(n), &ctx, &mut Rolls::always(0.0)).map(|p| p.text());
    assert_eq!(show(3).as_deref(), Some("3 reviews waiting"));
    assert_eq!(show(3), None, "the same numbers wait");
    assert_eq!(show(4).as_deref(), Some("4 reviews waiting"));
    assert_eq!(
        show(3).as_deref(),
        Some("3 reviews waiting"),
        "back to an earlier number shows again"
    );
    assert_eq!(show(3), None);
}

#[test]
fn a_facts_line_that_reads_the_clock_does_not_wait_on_the_clock() {
    let dict = tiny(&line(
        "facts",
        "late",
        "Late.",
        "when = [\"reviews.count >= 1\", \"hour >= 13\"]",
    ));
    let mut state = State {
        starts: 4,
        ..State::default()
    };
    let first = at(TUE, 13, 0);
    let facts = fresh_facts(&first).with("reviews.count", 2);
    state.last_day = Some(first.date.iso());
    assert!(go(&dict, &mut state, &facts, &first, &mut Rolls::always(0.0)).is_some());
    let later = at(TUE, 15, 0);
    let facts = fresh_facts(&later).with("reviews.count", 2);
    assert!(
        go(&dict, &mut state, &facts, &later, &mut Rolls::always(0.0)).is_none(),
        "the hour is not a number it read"
    );
}

#[test]
fn facts_unchanged_show_lets_a_facts_line_repeat() {
    let dict = tiny(&line(
        "facts",
        "q",
        "{n} here",
        "when = [\"reviews.count >= 1\"]",
    ))
    .with_user("[draw]\nfacts_unchanged = \"show\"\n")
    .expect("merges");
    assert!(!dict.draw.facts_unchanged_waits);
    let ctx = now();
    let mut state = state_today(&ctx);
    let facts = fresh_facts(&ctx).with("reviews.count", 3);
    assert!(go(&dict, &mut state, &facts, &ctx, &mut Rolls::always(0.0)).is_some());
    assert!(go(&dict, &mut state, &facts, &ctx, &mut Rolls::always(0.0)).is_some());
}

#[test]
fn a_tip_retires_once_its_action_was_used_three_times() {
    let dict = tiny(&line("tips", "t", "{key:/} filters.", "used = \"filter\""));
    let mut ctx = now();
    let facts = fresh_facts(&ctx);
    for (uses, shows) in [(0, true), (2, true), (3, false), (40, false)] {
        ctx.tip_uses.insert("filter".into(), uses);
        let got = go(
            &dict,
            &mut state_today(&ctx),
            &facts,
            &ctx,
            &mut Rolls::always(0.0),
        );
        assert_eq!(got.is_some(), shows, "used {uses} times");
    }
    // An action the context does not count has not been used.
    ctx.tip_uses.clear();
    assert!(
        go(
            &dict,
            &mut state_today(&ctx),
            &facts,
            &ctx,
            &mut Rolls::always(0.0)
        )
        .is_some()
    );
}

#[test]
fn facts_older_than_a_day_are_not_held() {
    let dict = tiny(&line(
        "alpha",
        "a",
        "Queue.",
        "when = [\"reviews.count >= 1\"]",
    ));
    let ctx = now();
    let shows = |facts: Facts| {
        go(
            &dict,
            &mut state_today(&ctx),
            &facts,
            &ctx,
            &mut Rolls::always(0.0),
        )
        .is_some()
    };
    let held = |age: i64| Facts::new(ctx.epoch - age).with("reviews.count", 3);
    assert!(shows(held(0)));
    assert!(shows(held(DAY)), "a day exactly is still a day");
    assert!(!shows(held(DAY + 1)));
    // One fact's own time wins over the rest's.
    let mut mixed = Facts::new(ctx.epoch);
    mixed.set_at("reviews.count", 3, ctx.epoch - 2 * DAY);
    assert!(!shows(mixed));
    let mut mixed = Facts::new(ctx.epoch - 5 * DAY);
    mixed.set_at("reviews.count", 3, ctx.epoch - 60);
    assert!(shows(mixed));
    // A fact the caller does not hold at all.
    assert!(!shows(Facts::new(ctx.epoch)));
}

#[test]
fn a_placeholder_whose_fact_is_missing_or_stale_makes_the_line_ineligible() {
    let dict = tiny(&line("alpha", "a", "{who} is here", ""));
    let ctx = now();
    let none = go(
        &dict,
        &mut state_today(&ctx),
        &fresh_facts(&ctx),
        &ctx,
        &mut Rolls::always(0.0),
    );
    assert!(none.is_none());
    let stale = Facts::new(ctx.epoch - 2 * DAY).with("who", "Vimes");
    assert!(
        go(
            &dict,
            &mut state_today(&ctx),
            &stale,
            &ctx,
            &mut Rolls::always(0.0)
        )
        .is_none()
    );
    let fresh = fresh_facts(&ctx).with("who", "Vimes");
    let got = go(
        &dict,
        &mut state_today(&ctx),
        &fresh,
        &ctx,
        &mut Rolls::always(0.0),
    )
    .expect("a line");
    assert_eq!(got.text(), "Vimes is here");
}

#[test]
fn emoji_lines_wait_for_a_terminal_that_draws_them() {
    let dict = tiny(
        &(line("alpha", "plain", "Plain.", "when = [\"no_emoji\"]")
            + &line("alpha", "pretty", "Pretty. ✨", "emoji = true")),
    );
    let mut ctx = now();
    let facts = fresh_facts(&ctx);
    for _ in 0..20 {
        ctx.emoji_ok = false;
        let got = go(
            &dict,
            &mut State::default(),
            &facts,
            &ctx,
            &mut Rolls::always(0.5),
        )
        .expect("a line");
        assert_eq!(got.id, "plain");
        ctx.emoji_ok = true;
        let got = go(
            &dict,
            &mut State::default(),
            &facts,
            &ctx,
            &mut Rolls::always(0.5),
        )
        .expect("a line");
        assert_eq!(got.id, "pretty", "no_emoji does not hold when emoji draw");
    }
}

#[test]
fn a_line_with_an_animation_that_is_not_built_is_never_drawn() {
    let dict = tiny(&line("alpha", "a", "Snow.", "anim = \"snow\""));
    let ctx = now();
    assert!(
        go(
            &dict,
            &mut state_today(&ctx),
            &fresh_facts(&ctx),
            &ctx,
            &mut Rolls::always(0.0)
        )
        .is_none()
    );
    let dict = tiny(&line("alpha", "a", "Fire.", "anim = \"fire\""));
    let got = go(
        &dict,
        &mut state_today(&ctx),
        &fresh_facts(&ctx),
        &ctx,
        &mut Rolls::always(0.0),
    )
    .expect("a line");
    assert_eq!(got.anim.as_deref(), Some("fire"));
    let mut still = ctx.clone();
    still.motion_full = false;
    assert!(
        go(
            &dict,
            &mut state_today(&still),
            &fresh_facts(&still),
            &still,
            &mut Rolls::always(0.0)
        )
        .is_none()
    );
}

#[test]
fn the_agents_group_needs_an_agent_set_up() {
    let dict = Dictionary::shipped();
    let ctx = now();
    let facts = fresh_facts(&ctx);
    for i in 0..80 {
        let mut state = state_today(&ctx);
        let got = go(
            &dict,
            &mut state,
            &facts,
            &ctx,
            &mut Rolls::new(&[0.9, f64::from(i) / 80.0]),
        )
        .expect("a line");
        assert_ne!(got.group, "agents");
    }
    let facts = fresh_facts(&ctx)
        .with("agents.setup", 1)
        .with("agent", "Copilot");
    let mut seen = BTreeSet::new();
    for i in 0..300 {
        let mut state = state_today(&ctx);
        let mut luck = SplitMix64::new(i);
        let got = pick(
            &dict,
            &UserConfig::default(),
            &mut state,
            &facts,
            &ctx,
            &mut luck,
        )
        .expect("a line");
        seen.insert(got.group);
    }
    assert!(seen.contains("agents"));
}

#[test]
fn lines_new_in_an_update_go_to_the_front_of_their_bag() {
    let old = line("alpha", "a1", "One", "")
        + &line("alpha", "a2", "Two", "")
        + &line("alpha", "a3", "Three", "");
    let new = old.clone() + &line("alpha", "a4", "Four", "") + &line("beta", "b1", "Bee", "");
    let (dict, ctx) = (tiny(&new), now());
    let mut state = state_today(&ctx);
    state.known = ["a1", "a2", "a3"].map(String::from).to_vec();
    state
        .bags
        .insert("alpha".into(), vec!["a3".into(), "a1".into()]);
    let facts = fresh_facts(&ctx);
    let got = go(&dict, &mut state, &facts, &ctx, &mut Rolls::always(0.0)).expect("a line");
    assert_eq!(got.id, "a4");
    let got = go(&dict, &mut state, &facts, &ctx, &mut Rolls::always(0.0)).expect("a line");
    assert_eq!(got.id, "a3");
    // Known now: the ids are those of the dictionary.
    assert_eq!(state.known, ["a1", "a2", "a3", "a4", "b1"]);
}

#[test]
fn the_first_ever_start_has_no_new_lines() {
    let dict = tiny(&(line("alpha", "a1", "One", "") + &line("alpha", "a2", "Two", "")));
    let ctx = now();
    let mut state = State::default();
    let _ = go(
        &dict,
        &mut state,
        &fresh_facts(&ctx),
        &ctx,
        &mut Rolls::always(0.0),
    );
    assert_eq!(state.known, ["a1", "a2"]);
}

// ------------------------------------------------------------ the user's side

#[test]
fn the_user_can_turn_everything_off() {
    let dict = Dictionary::shipped();
    let ctx = now();
    let cfg = UserConfig {
        show: false,
        ..UserConfig::default()
    };
    let mut state = State::default();
    let mut first = ctx.clone();
    first.first_run = true;
    let facts = fresh_facts(&ctx).with("pipelines.failed_new", 1);
    assert_eq!(
        pick(
            &dict,
            &cfg,
            &mut state,
            &facts,
            &first,
            &mut Rolls::always(0.0)
        ),
        None
    );
    assert_eq!(state.starts, 1, "the start still counts");
}

#[test]
fn a_user_turns_off_tips_or_a_group() {
    let dict = tiny(&(line("tips", "t", "Tip.", "") + &line("alpha", "a", "Alpha.", "")));
    let ctx = now();
    let facts = fresh_facts(&ctx);
    let tips_off = UserConfig {
        tips: false,
        ..UserConfig::default()
    };
    let group_off = UserConfig {
        off: vec!["alpha".into()],
        ..UserConfig::default()
    };
    for i in 0..40 {
        let mut luck = SplitMix64::new(i);
        let got = pick(
            &dict,
            &tips_off,
            &mut State::default(),
            &facts,
            &ctx,
            &mut luck,
        )
        .expect("a line");
        assert_eq!(got.group, "alpha");
        let got = pick(
            &dict,
            &group_off,
            &mut State::default(),
            &facts,
            &ctx,
            &mut luck,
        )
        .expect("a line");
        assert_eq!(got.group, "tips");
    }
    // Plain lines are not a group to switch off.
    let shipped = Dictionary::shipped();
    let cfg = UserConfig {
        off: vec!["plain".into()],
        ..UserConfig::default()
    };
    let facts = fresh_facts(&ctx).with("pipelines.failed_new", 1);
    let got = pick(
        &shipped,
        &cfg,
        &mut state_today(&ctx),
        &facts,
        &ctx,
        &mut Rolls::always(0.0),
    )
    .expect("a line");
    assert_eq!(got.id, "pipes-failed");
}

#[test]
fn the_users_weights_replace_the_dictionarys() {
    let dict = tiny(&(line("alpha", "a", "Alpha.", "") + &line("beta", "b", "Beta.", "")));
    let ctx = now();
    let facts = fresh_facts(&ctx);
    let cfg = UserConfig {
        weights: BTreeMap::from([("alpha".to_string(), 0)]),
        ..UserConfig::default()
    };
    for roll in [0.0, 0.5, 0.99] {
        let got = pick(
            &dict,
            &cfg,
            &mut State::default(),
            &facts,
            &ctx,
            &mut Rolls::new(&[roll]),
        )
        .expect("a line");
        assert_eq!(got.group, "beta");
    }
    let cfg = UserConfig {
        weights: BTreeMap::from([("beta".to_string(), 30)]),
        ..UserConfig::default()
    };
    let got = pick(
        &dict,
        &cfg,
        &mut State::default(),
        &facts,
        &ctx,
        &mut Rolls::new(&[0.3]),
    )
    .expect("a line");
    assert_eq!(got.group, "beta", "beta now has 30 of 40");
}

#[test]
fn the_users_own_lines_are_a_group_of_their_own() {
    let dict = Dictionary::parse("").expect("an empty dictionary");
    let ctx = now();
    let cfg = UserConfig {
        mine: vec![
            "Remember: lunch is a feature.".into(),
            "Mind the {key:esc} key.".into(),
        ],
        ..UserConfig::default()
    };
    let mut state = state_today(&ctx);
    let mut seen = Vec::new();
    for _ in 0..4 {
        let got = pick(
            &dict,
            &cfg,
            &mut state,
            &fresh_facts(&ctx),
            &ctx,
            &mut Rolls::always(0.0),
        )
        .expect("a line");
        assert_eq!(got.group, "mine");
        seen.push(got);
    }
    // Two lines, a bag of two: each shows once before either repeats.
    assert_ne!(seen[0].id, seen[1].id);
    assert_ne!(seen[2].id, seen[3].id);
    let texts: BTreeSet<String> = seen.iter().map(Picked::text).collect();
    assert_eq!(
        texts,
        BTreeSet::from([
            "Remember: lunch is a feature.".to_string(),
            "Mind the esc key.".to_string()
        ])
    );
    assert!(
        seen.iter()
            .flat_map(|p| &p.spans)
            .any(|s| s.kind == SpanKind::Key && s.text == "esc")
    );
}

#[test]
fn a_line_of_the_users_that_cannot_show_is_left_out() {
    let dict = Dictionary::parse("").expect("an empty dictionary");
    let ctx = now();
    let cfg = UserConfig {
        mine: vec![
            "x".repeat(MAX_CELLS + 1),
            "Broken {nothing}.".into(),
            "Fine.".into(),
        ],
        ..UserConfig::default()
    };
    for _ in 0..5 {
        let got = pick(
            &dict,
            &cfg,
            &mut state_today(&ctx),
            &fresh_facts(&ctx),
            &ctx,
            &mut Rolls::always(0.0),
        );
        assert_eq!(got.map(|p| p.text()).as_deref(), Some("Fine."));
    }
}

#[test]
fn the_users_lines_keep_their_ids_when_the_list_changes() {
    let dict = Dictionary::parse("").expect("an empty dictionary");
    let id = |mine: &[&str]| {
        let cfg = UserConfig {
            mine: mine.iter().map(|s| s.to_string()).collect(),
            ..UserConfig::default()
        };
        let mine = cfg.mine_lines(&dict);
        mine.iter().map(|l| l.id.clone()).collect::<Vec<_>>()
    };
    let one = id(&["Alpha line.", "Beta line."]);
    let other = id(&["Beta line.", "Gamma line.", "Alpha line."]);
    assert_eq!(one[0], other[2]);
    assert_eq!(one[1], other[0]);
}

#[test]
fn the_users_config_table_reads_from_toml() {
    let cfg: UserConfig = toml::from_str(
        "show = true\ntips = false\noff = [\"time\"]\nmine = [\"Hello.\"]\n[weights]\nscope = 5\n",
    )
    .expect("reads");
    assert!(cfg.show && !cfg.tips);
    assert_eq!(
        (cfg.off, cfg.mine, cfg.weights["scope"]),
        (vec!["time".to_string()], vec!["Hello.".to_string()], 5)
    );
    let none: UserConfig = toml::from_str("").expect("reads");
    assert!(none.show && none.tips);
    assert!(
        toml::from_str::<UserConfig>("shown = true").is_err(),
        "a typo is an error, not silence"
    );
}

// ------------------------------------------------------------ the user's file

#[test]
fn a_user_dictionary_adds_lines_replaces_by_id_and_turns_groups_off() {
    let shipped = Dictionary::shipped();
    let user = r#"
off = ["time"]
[draw]
weights = { scope = 1 }
[[line]]
group = "scope"
id = "mine-1"
text = "Hello, {key:q}."
[[line]]
group = "work"
id = "meeting"
text = "Replaced."
"#;
    let merged = shipped.with_user(user).expect("merges");
    assert_eq!(merged.lines().len(), shipped.lines().len() + 1);
    assert_eq!(
        merged.line("meeting").map(|l| l.text.as_str()),
        Some("Replaced.")
    );
    assert_eq!(
        merged.line("mine-1").map(|l| l.group.as_str()),
        Some("scope")
    );
    assert_eq!(merged.draw.weights["scope"], 1);
    assert_eq!(
        merged.draw.weights["work"], 24,
        "what it did not name stays"
    );
    // The shipped one was not changed by merging.
    assert!(shipped.line("mine-1").is_none());
    // Position kept: the replaced line is where it was.
    let at = |d: &Dictionary| d.lines().iter().position(|l| l.id == "meeting");
    assert_eq!(at(&merged), at(&shipped));
    // The group is off: a day of starts shows no time line.
    let ctx = now();
    let facts = fresh_facts(&ctx).with("hour", 22);
    for i in 0..200 {
        let got = pick(
            &merged,
            &UserConfig::default(),
            &mut state_today(&ctx),
            &facts,
            &ctx,
            &mut SplitMix64::new(i),
        )
        .expect("a line");
        assert_ne!(got.group, "time");
    }
}

#[test]
fn a_user_can_make_a_group_of_their_own() {
    let user = "[draw]\nweights = { pubs = 40 }\n[[line]]\ngroup = \"pubs\"\nid = \"p\"\ntext = \"The Mended Drum.\"\n";
    let merged = Dictionary::shipped().with_user(user).expect("merges");
    assert_eq!(merged.line("p").map(|l| l.group.as_str()), Some("pubs"));
    let cfg = UserConfig {
        weights: BTreeMap::from([
            ("scope".into(), 0),
            ("work".into(), 0),
            ("facts".into(), 0),
            ("tips".into(), 0),
            ("agents".into(), 0),
            ("time".into(), 0),
            ("glyph".into(), 0),
        ]),
        ..UserConfig::default()
    };
    let ctx = now();
    let got = pick(
        &merged,
        &cfg,
        &mut state_today(&ctx),
        &fresh_facts(&ctx),
        &ctx,
        &mut Rolls::new(&[0.5, 0.0]),
    )
    .expect("a line");
    assert_eq!(got.group, "pubs");
}

#[test]
fn a_bad_user_dictionary_is_an_error_that_says_what_and_where() {
    let shipped = Dictionary::shipped();
    let cases: [(&str, &str); 14] = [
        ("[[line]\nid=", "not valid TOML"),
        ("[[line]]\ngroup = \"scope\"\nid = \"x\"\n", "has no text"),
        ("[[line]]\ngroup = \"scope\"\ntext = \"x\"\n", "has no id"),
        ("[[line]]\nid = \"x\"\ntext = \"x\"\n", "has no group"),
        (
            "[[line]]\ngroup = \"nowhere\"\nid = \"x\"\ntext = \"x\"\n",
            "group `nowhere` has no weight",
        ),
        (
            "[[line]]\ngroup = \"scope\"\nid = \"x\"\ntext = \"x\"\nwhen = [\"a >> b\"]\n",
            "condition `a >> b`",
        ),
        (
            "[[line]]\ngroup = \"scope\"\nid = \"x\"\ntext = \"{oops}\"\n",
            "`{oops}` is neither a count nor listed in [widest]",
        ),
        (
            "[[line]]\ngroup = \"scope\"\nid = \"x\"\ntext = \"{n:thing}\"\n",
            "`{n}` needs a fact in `when`",
        ),
        (
            "[[line]]\ngroup = \"scope\"\nid = \"x\"\ntext = \"oh {unclosed\"\n",
            "without a `}`",
        ),
        (
            "[[line]]\ngroup = \"scope\"\nid = \"x\"\ntext = \"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\"\n",
            "62 cells at its widest; the most is 56",
        ),
        (
            "[[line]]\ngroup = \"scope\"\nid = \"x\"\ntext = \"a\"\n[[line]]\ngroup = \"work\"\nid = \"x\"\ntext = \"b\"\n",
            "`x`: the id is used twice",
        ),
        (
            "[[line]]\ngroup = \"scope\"\nid = \"x\"\ntext = \"a\"\nwen = []\n",
            "unknown field `wen`",
        ),
        ("version = 2\n", "version 2"),
        (
            "[odds]\nrare = { chance = 3.0, min_days_between = 7 }\n",
            "chance is 0 to 1",
        ),
    ];
    for (text, why) in cases {
        let err = shipped.with_user(text).expect_err(text);
        assert!(
            err.to_string().contains(why),
            "{text:?} gave {err}, wanting {why:?}"
        );
    }
}

#[test]
fn a_bad_user_file_is_ignored_not_fatal_and_a_good_one_is_read() {
    let dir = temp_dir();
    let shipped = Dictionary::shipped().lines().len();
    let path = dir.join("start-lines.toml");
    assert_eq!(load_dictionary(&path).lines().len(), shipped, "no file");
    std::fs::write(&path, "[[line]\n").expect("written");
    assert_eq!(load_dictionary(&path).lines().len(), shipped, "bad TOML");
    std::fs::write(&path, [0xff, 0xfe, 0x00]).expect("written");
    assert_eq!(load_dictionary(&path).lines().len(), shipped, "not text");
    std::fs::write(&path, "[[line]]\ngroup = \"scope\"\nid = \"x\"\ntext = \"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\"\n").expect("written");
    assert_eq!(
        load_dictionary(&path).lines().len(),
        shipped,
        "a line too wide drops the file"
    );
    std::fs::write(
        &path,
        "[[line]]\ngroup = \"scope\"\nid = \"x\"\ntext = \"Hello.\"\n",
    )
    .expect("written");
    assert_eq!(load_dictionary(&path).lines().len(), shipped + 1);
    std::fs::remove_dir_all(dir).ok();
}

// ---------------------------------------------------------------------- state

#[test]
fn a_missing_or_corrupt_state_is_a_fresh_one() {
    let dir = temp_dir();
    let path = dir.join("start-lines.toml");
    assert_eq!(load_state(&path), State::default());
    assert!(load_state(&path).is_fresh());
    for bad in [
        "starts = \"many\"\n",
        "[[[",
        "\u{0}\u{1}",
        "starts = -5\nbags = 3\n",
        "shown = []\n",
    ] {
        std::fs::write(&path, bad).expect("written");
        assert_eq!(load_state(&path), State::default(), "{bad:?}");
    }
    std::fs::write(&path, [0xff, 0xfe]).expect("written");
    assert_eq!(load_state(&path), State::default(), "not text");
    // A folder where the file should be is not an error either.
    std::fs::remove_file(&path).expect("removed");
    std::fs::create_dir(&path).expect("a folder");
    assert_eq!(load_state(&path), State::default());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn a_state_with_only_some_fields_is_read() {
    let dir = temp_dir();
    let path = dir.join("start-lines.toml");
    std::fs::write(
        &path,
        "starts = 41\nlast_day = \"2026-10-12\"\nfuture_field = 1\n",
    )
    .expect("written");
    let state = load_state(&path);
    assert_eq!(
        (state.starts, state.last_day.as_deref()),
        (41, Some("2026-10-12"))
    );
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn the_state_survives_a_save_and_a_load() {
    let dir = temp_dir();
    let path = dir.join("deeper").join("radar").join("start-lines.toml");
    let dict = Dictionary::shipped();
    let ctx = now();
    let mut state = State::default();
    let mut luck = SplitMix64::new(11);
    let mut facts = fresh_facts(&ctx);
    facts
        .set("pipelines.failing", 0)
        .set("reviews.count", 3)
        .set("reviews.prev", 5);
    for _ in 0..25 {
        let _ = pick(
            &dict,
            &UserConfig::default(),
            &mut state,
            &facts,
            &ctx,
            &mut luck,
        );
    }
    state.last_event = Some("2026-09-01".into());
    state.last_rare = Some("2026-10-01".into());
    save_state(&path, &state).expect("saved, folders made");
    assert_eq!(load_state(&path), state);
    let left: Vec<_> = std::fs::read_dir(path.parent().expect("a folder"))
        .expect("lists")
        .flatten()
        .collect();
    assert_eq!(left.len(), 1, "the file beside it is moved over it");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn saving_into_a_folder_that_cannot_be_made_is_an_error_for_the_caller() {
    let dir = temp_dir();
    let blocker = dir.join("file");
    std::fs::write(&blocker, "x").expect("written");
    assert!(save_state(&blocker.join("below").join("state.toml"), &State::default()).is_err());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn the_sandbox_keeps_the_state_out_of_the_real_folder() {
    // Every cargo test is sandboxed; with no XDG_STATE_HOME there is no path.
    if std::env::var_os("XDG_STATE_HOME").is_none() {
        assert_eq!(state_path(), None);
    }
}

#[test]
fn picking_counts_the_start() {
    let dict = tiny(&line("alpha", "a", "A", ""));
    let ctx = now();
    let mut state = State::default();
    assert!(state.is_fresh());
    let _ = go(
        &dict,
        &mut state,
        &fresh_facts(&ctx),
        &ctx,
        &mut Rolls::always(0.0),
    );
    assert_eq!((state.starts, state.starts_today), (1, 1));
    assert_eq!(state.last_day.as_deref(), Some("2026-10-13"));
    assert_eq!(state.first_day.as_deref(), Some("2026-10-13"));
    let _ = go(
        &dict,
        &mut state,
        &fresh_facts(&ctx),
        &ctx,
        &mut Rolls::always(0.0),
    );
    assert_eq!((state.starts, state.starts_today), (2, 2));
    let tomorrow = at(Date::from_days(TUE.days() + 1), 9, 0);
    let _ = go(
        &dict,
        &mut state,
        &fresh_facts(&tomorrow),
        &tomorrow,
        &mut Rolls::always(0.0),
    );
    assert_eq!((state.starts, state.starts_today), (3, 1));
    assert_eq!(state.first_day.as_deref(), Some("2026-10-13"));
}

#[test]
fn what_the_state_knows_becomes_facts_a_line_can_ask_for() {
    let dict = tiny(
        &(line(
            "alpha",
            "log",
            "Day {days}.",
            "when = [\"install.days >= 2\"]",
        ) + &line(
            "beta",
            "busy",
            "Start {today} today.",
            "when = [\"start.today >= 3\"]",
        ) + &line(
            "facts",
            "many",
            "Start {starts}.",
            "when = [\"start.count >= 1000\"]",
        )),
    );
    let ctx = at(Date::from_days(TUE.days() + 3), 9, 0);
    let state = State {
        starts: 1233,
        first_day: Some(TUE.iso()),
        last_day: Some(ctx.date.iso()),
        starts_today: 2,
        ..State::default()
    };
    let cfg = UserConfig::default();
    let mut texts = BTreeSet::new();
    for i in 0..40 {
        let mut s = state.clone();
        if let Some(p) = pick(
            &dict,
            &cfg,
            &mut s,
            &fresh_facts(&ctx),
            &ctx,
            &mut SplitMix64::new(i),
        ) {
            texts.insert(p.text());
        }
    }
    assert_eq!(
        texts,
        BTreeSet::from([
            "Day 3.".to_string(),
            "Start 3 today.".to_string(),
            "Start 1,234.".to_string()
        ])
    );
}

#[test]
fn an_anniversary_counts_its_years() {
    let dict = Dictionary::shipped();
    let ctx = at(Date::new(2028, 3, 2), 9, 0);
    let mut state = State {
        starts: 300,
        first_day: Some("2026-03-02".into()),
        last_day: Some("2028-03-01".into()),
        ..State::default()
    };
    let got = go(
        &dict,
        &mut state,
        &fresh_facts(&ctx),
        &ctx,
        &mut Rolls::always(0.0),
    )
    .expect("a line");
    assert_eq!(got.id, "anniversary");
    assert_eq!(
        got.text(),
        "2 years of radar today. Thanks for keeping watch."
    );
}

// ------------------------------------------------------------------ placeholders

fn say(line_text: &str, facts: impl FnOnce(Facts) -> Facts, ctx: &Context) -> Option<Picked> {
    let extra = r#"when = ["reviews.count >= 0"]"#;
    let dict = tiny(&line("alpha", "p", line_text, extra));
    let facts = facts(fresh_facts(ctx).with("reviews.count", 7));
    go(
        &dict,
        &mut state_today(ctx),
        &facts,
        ctx,
        &mut Rolls::always(0.0),
    )
}

#[test]
fn a_count_and_its_noun_agree() {
    let ctx = now();
    let count = |n: i64, text: &str| {
        let dict = tiny(&line("alpha", "p", text, "when = [\"reviews.count >= 0\"]"));
        let facts = fresh_facts(&ctx).with("reviews.count", n);
        go(
            &dict,
            &mut state_today(&ctx),
            &facts,
            &ctx,
            &mut Rolls::always(0.0),
        )
        .map(|p| p.text())
    };
    assert_eq!(
        count(1, "{n:branch|branches} here").as_deref(),
        Some("1 branch here")
    );
    assert_eq!(
        count(0, "{n:branch|branches} here").as_deref(),
        Some("0 branches here")
    );
    assert_eq!(
        count(3, "{n:branch|branches} here").as_deref(),
        Some("3 branches here")
    );
    assert_eq!(
        count(1, "{n:review} here").as_deref(),
        Some("1 review here")
    );
    assert_eq!(
        count(2, "{n:review} here").as_deref(),
        Some("2 reviews here")
    );
    assert_eq!(count(2, "{n} here").as_deref(), Some("2 here"));
    assert_eq!(
        count(1000, "{n:review} here").as_deref(),
        Some("999 reviews here"),
        "capped at 999"
    );
    assert_eq!(count(999, "{n}").as_deref(), Some("999"));
    assert_eq!(count(1, "{n:merge|merges}").as_deref(), Some("1 merge"));
}

#[test]
fn a_count_is_a_value_span_with_its_noun() {
    let got = say("You have {n:review} to do.", |f| f, &now()).expect("a line");
    assert_eq!(
        got.spans,
        vec![
            Span {
                text: "You have ".into(),
                kind: SpanKind::Text
            },
            Span {
                text: "7 reviews".into(),
                kind: SpanKind::Value
            },
            Span {
                text: " to do.".into(),
                kind: SpanKind::Text
            },
        ]
    );
}

#[test]
fn a_name_is_cut_to_16_cells_with_an_ellipsis() {
    let ctx = now();
    let name = |n: &str| say("{who}!", |f| f.with("who", n), &ctx).map(|p| p.text());
    assert_eq!(name("Sam Vimes").as_deref(), Some("Sam Vimes!"));
    assert_eq!(
        name("0123456789abcdef").as_deref(),
        Some("0123456789abcdef!"),
        "16 stays whole"
    );
    assert_eq!(
        name("0123456789abcdefg").as_deref(),
        Some("0123456789abcde…!"),
        "17 is cut"
    );
    // Wide characters count 2: ten of them are 20 cells; seven fit in 15.
    let wide = name("名名名名名名名名名名").expect("a line");
    assert_eq!(wide, "名名名名名名名…!");
    assert!(width(&wide) <= NAME_CELLS + 1);
}

#[test]
fn a_key_may_be_a_punctuation_mark_or_a_name() {
    let ctx = now();
    let keys = |text: &str| {
        say(text, |f| f, &ctx).map(|p| {
            p.spans
                .iter()
                .filter(|s| s.kind == SpanKind::Key)
                .map(|s| s.text.clone())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(keys("{key::} jumps.").expect("a line"), [":"]);
    assert_eq!(
        keys("{key:`} shows. {key:ctrl-t} starts.").expect("a line"),
        ["`", "ctrl-t"]
    );
    assert_eq!(
        keys("{key:/} filters. {key:esc} clears.").expect("a line"),
        ["/", "esc"]
    );
    assert_eq!(keys("{key:,} is Settings.").expect("a line"), [","]);
}

#[test]
fn the_clock_fills_time_unless_a_fact_has_it() {
    let ctx = at(TUE, 9, 5);
    assert_eq!(
        say("It's {time}.", |f| f, &ctx)
            .map(|p| p.text())
            .as_deref(),
        Some("It's 09:05.")
    );
    let held = say("Held at {time}.", |f| f.with("time", "03:00"), &ctx);
    assert_eq!(held.map(|p| p.text()).as_deref(), Some("Held at 03:00."));
}

#[test]
fn a_line_that_comes_out_too_wide_does_not_show() {
    let ctx = now();
    let long = "x".repeat(40);
    // The dictionary says `short` is one cell wide, so the line fits on paper.
    let text = "{short} {short} {short} {short}";
    assert!(say(text, |f| f.with("short", long.as_str()), &ctx).is_none());
    assert!(say(text, |f| f.with("short", "Vimes"), &ctx).is_some());
}

#[test]
fn the_counts_read_the_facts_the_line_names() {
    let binds = |when: &[&str]| {
        let conds: Vec<Cond> = when
            .iter()
            .map(|c| Cond::parse(c).expect("parses"))
            .collect();
        Binds::of(&conds)
    };
    let b = binds(&["start.gap_days >= 3", "changes.since_last >= 1"]);
    assert_eq!(
        (b.n.as_deref(), b.days.as_deref()),
        (Some("changes.since_last"), Some("start.gap_days"))
    );
    let b = binds(&["date == 10-31", "git.stalest_branch_days >= 90"]);
    assert_eq!(
        (b.n.as_deref(), b.days.as_deref()),
        (
            Some("git.stalest_branch_days"),
            Some("git.stalest_branch_days")
        )
    );
    let b = binds(&["git.unpushed >= 1", "git.unpushed_days >= 2"]);
    assert_eq!(
        (b.n.as_deref(), b.days.as_deref()),
        (Some("git.unpushed"), Some("git.unpushed_days"))
    );
    let b = binds(&["weekday == fri", "hour >= 13", "mine.open >= 3"]);
    assert_eq!(b.n.as_deref(), Some("mine.open"));
    let b = binds(&["reviews.count < reviews.prev"]);
    assert_eq!(
        (b.n.as_deref(), b.m.as_deref()),
        (Some("reviews.count"), Some("reviews.prev"))
    );
    let b = binds(&["install.anniversary"]);
    assert_eq!(b.n.as_deref(), Some("install.anniversary"));
    assert_eq!(binds(&["weekday == mon"]), Binds::default());
}

#[test]
fn queue_down_names_both_numbers() {
    let dict = Dictionary::shipped();
    let ctx = now();
    let facts = fresh_facts(&ctx)
        .with("reviews.count", 2)
        .with("reviews.prev", 9);
    let mut seen = false;
    for i in 0..400 {
        let mut state = state_today(&ctx);
        if let Some(p) = pick(
            &dict,
            &UserConfig::default(),
            &mut state,
            &facts,
            &ctx,
            &mut SplitMix64::new(i),
        ) && p.id == "queue-down"
        {
            assert_eq!(p.text(), "Review queue: 2, down from 9. Look at you.");
            seen = true;
        }
    }
    assert!(seen);
}

// ------------------------------------------------------------------ conditions

fn held(facts: &[(&str, Value)], ctx: &Context) -> Held {
    let mut f = fresh_facts(ctx);
    for (name, v) in facts {
        f.set(name, v.clone());
    }
    Held::new(
        &State::default(),
        Tick {
            first_today: true,
            gap_days: 0,
        },
        &f,
        ctx,
    )
}

#[test]
fn the_condition_forms_hold_or_do_not() {
    let ctx = now();
    let h = held(
        &[
            ("flag", true.into()),
            ("off", false.into()),
            ("count", 5.into()),
            ("zero", 0.into()),
            ("a.b", 3.into()),
            ("a.c", 8.into()),
            ("weekday", "fri".into()),
            ("date", "12-25".into()),
            ("hour", 22.into()),
        ],
        &ctx,
    );
    let cases = [
        ("flag", true),
        ("off", false),
        ("zero", false),
        ("count", true),
        ("missing", false),
        ("count == 5", true),
        ("count == 6", false),
        ("count >= 5", true),
        ("count >= 6", false),
        ("count <= 5", true),
        ("count < 5", false),
        ("count > 4", true),
        ("count>4", true),
        ("count > 5", false),
        ("missing >= 0", false),
        ("missing == 0", false),
        ("a.b < a.c", true),
        ("a.c < a.b", false),
        ("a.b < a.missing", false),
        ("weekday == fri", true),
        ("weekday == FRI", true),
        ("weekday == sat", false),
        ("weekday in sat,sun", false),
        ("weekday in fri, sat", true),
        ("date == 12-25", true),
        ("date == 12-24", false),
        ("date in 12-24..12-26", true),
        ("date in 12-26..12-28", false),
        ("date in 12-01..12-25", true),
        ("hour in 22..23", true),
        ("hour in 1..4", false),
        ("hour in 22..22", true),
        ("hour >= 22", true),
        ("flag == true", true),
        ("off == true", false),
        ("no_emoji", false),
        ("quarter_days_left <= 7", false),
        ("start.first_today", true),
        ("start.count == 1", false),
    ];
    for (text, want) in cases {
        let cond = Cond::parse(text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(cond.holds(&h, &ctx), want, "{text}");
    }
    let mut plain = ctx.clone();
    plain.emoji_ok = false;
    assert!(Cond::NoEmoji.holds(&h, &plain));
}

#[test]
fn conditions_that_make_no_sense_are_errors() {
    for bad in [
        "",
        "a b",
        "a >=",
        ">= 3",
        "a ==",
        "a in",
        "1 in 2..3",
        "a in ..3",
        "a >> 3",
        "$x",
    ] {
        assert!(Cond::parse(bad).is_err(), "{bad:?} parsed");
    }
}

#[test]
fn the_clock_facts_are_those_of_the_local_date() {
    let ctx = at(Date::new(2026, 12, 25), 22, 40);
    let h = held(&[], &ctx);
    let get = |name: &str| h.get(name).cloned();
    assert_eq!(get("hour"), Some(Value::Num(22)));
    assert_eq!(get("minute"), Some(Value::Num(40)));
    assert_eq!(get("weekday"), Some(Value::Str("fri".into())));
    assert_eq!(get("date"), Some(Value::Str("12-25".into())));
    assert_eq!(get("month_day"), Some(Value::Num(25)));
    assert_eq!(get("day_of_year"), Some(Value::Num(359)));
    assert_eq!(get("quarter_days_left"), Some(Value::Num(6)));
    assert_eq!(get("splash.echo_rare"), Some(Value::Bool(false)));
}

#[test]
fn a_callers_fact_replaces_one_worked_out_here() {
    let ctx = now();
    let h = held(&[("hour", 3.into())], &ctx);
    assert_eq!(h.get("hour"), Some(&Value::Num(3)));
    // Its value is the caller's own, so a line that read it would wait on it
    // unless it is a clock fact, which it is.
    assert_eq!(h.signature("hour"), None);
    let h = held(&[("start.count", 77.into())], &ctx);
    assert_eq!(
        h.signature("start.count"),
        Some(("start.count".into(), "77".into()))
    );
}

// ------------------------------------------------------------------- the pieces

#[test]
fn dates_round_trip_and_know_their_weekday() {
    for days in [
        -1, 0, 1, 59, 60, 365, 10_957, 11_016, 20_000, 20_746, 25_000,
    ] {
        assert_eq!(Date::from_days(days).days(), days);
    }
    assert_eq!(Date::new(1970, 1, 1).days(), 0);
    assert_eq!(Date::new(2000, 2, 29).days(), 11_016);
    assert_eq!(Date::new(2026, 10, 31).weekday(), "sat");
    assert_eq!(Date::new(2026, 10, 15).weekday(), "thu");
    assert_eq!(Date::new(1970, 1, 1).weekday(), "thu");
    assert_eq!(Date::new(2026, 9, 13).day_of_year(), 256);
    assert_eq!(Date::new(2028, 12, 31).day_of_year(), 366);
    assert_eq!(Date::new(2026, 10, 1).quarter_days_left(), 91);
    assert_eq!(Date::new(2026, 12, 31).quarter_days_left(), 0);
    assert_eq!(Date::new(2026, 3, 25).quarter_days_left(), 6);
    assert_eq!(Date::parse("2026-10-05"), Some(Date::new(2026, 10, 5)));
    assert_eq!(Date::new(2026, 10, 5).iso(), "2026-10-05");
    for bad in [
        "",
        "2026",
        "2026-13-01",
        "2026-00-10",
        "x-y-z",
        "2026-10-40",
    ] {
        assert_eq!(Date::parse(bad), None, "{bad:?}");
    }
}

#[test]
fn chance_is_in_range_and_repeatable() {
    let mut a = SplitMix64::new(42);
    let mut b = SplitMix64::new(42);
    let run: Vec<u64> = (0..5).map(|_| a.next_u64()).collect();
    assert_eq!(run, (0..5).map(|_| b.next_u64()).collect::<Vec<_>>());
    assert_ne!(run[0], SplitMix64::new(43).next_u64());
    let mut r = SplitMix64::new(1);
    for n in [0, 1, 2, 7, 169] {
        for _ in 0..200 {
            assert!(r.below(n) < n.max(1));
        }
    }
    for _ in 0..200 {
        let u = r.unit();
        assert!((0.0..1.0).contains(&u));
    }
    // 0 and 1 choices use no chance.
    let mut r = SplitMix64::new(1);
    let _ = (r.below(0), r.below(1));
    assert_eq!(r.next_u64(), SplitMix64::new(1).next_u64());
    // Entropy gives different generators.
    assert_ne!(
        SplitMix64::from_entropy().next_u64(),
        SplitMix64::from_entropy().next_u64()
    );
}

#[test]
fn thousands_get_commas() {
    assert_eq!(
        [0, 7, 999, 1000, 12_345, 99_999].map(thousands),
        ["0", "7", "999", "1,000", "12,345", "99,999"]
    );
}

#[test]
fn a_template_splits_into_text_keys_and_values() {
    let pieces = parse_template("A {key:x} B {n:one|many} C {who}.").expect("parses");
    assert_eq!(
        pieces,
        vec![
            Piece::Text("A ".into()),
            Piece::Key("x".into()),
            Piece::Text(" B ".into()),
            Piece::Value {
                name: "n".into(),
                noun: Some(("one".into(), "many".into()))
            },
            Piece::Text(" C ".into()),
            Piece::Value {
                name: "who".into(),
                noun: None
            },
            Piece::Text(".".into()),
        ]
    );
    for bad in ["{", "{n", "{key:}", "{n:}", "{ n }", "{n:|x}"] {
        assert!(parse_template(bad).is_err(), "{bad:?}");
    }
    assert_eq!(parse_template("").expect("parses"), vec![]);
}

// ------------------------------------------------------- many starts in a row

#[test]
fn a_year_of_starts_holds_every_rule_at_once() {
    let dict = Dictionary::shipped();
    let cfg = UserConfig {
        mine: vec!["Mind the gap.".into()],
        ..UserConfig::default()
    };
    let mut state = State::default();
    let mut luck = SplitMix64::new(2026);
    let mut groups: Vec<String> = Vec::new();
    let mut per_group: BTreeMap<String, usize> = BTreeMap::new();
    let mut ids_by_day: BTreeMap<(i64, String), usize> = BTreeMap::new();
    let mut events = 0;
    let mut start = 0;
    for day in 0..365 {
        let date = Date::from_days(TUE.days() + day);
        for hour in [9, 14] {
            start += 1;
            let mut ctx = at(date, hour, 10);
            ctx.first_run = start == 1;
            let mut facts = fresh_facts(&ctx);
            facts
                .set("reviews.count", (start % 5) as i64)
                .set("reviews.prev", ((start + 1) % 5) as i64)
                .set("mine.open", (start % 7) as i64)
                .set("pipelines.failing", 0)
                .set("pipelines.followed", 4)
                .set("agents.setup", 2)
                .set("agent", "Claude")
                .set("agent2", "Codex");
            let Some(got) = pick(&dict, &cfg, &mut state, &facts, &ctx, &mut luck) else {
                continue;
            };
            assert!(got.cells() <= MAX_CELLS, "{}: {}", got.id, got.text());
            assert!(
                !got.text().contains('{') && !got.text().contains('}'),
                "{}",
                got.text()
            );
            if got.anim.is_some() {
                events += 1;
                assert_eq!(got.anim.as_deref(), Some("fire"));
            }
            if got.group == "time" {
                *ids_by_day.entry((date.days(), got.id.clone())).or_default() += 1;
            }
            *per_group.entry(got.group.clone()).or_default() += 1;
            groups.push(got.group);
        }
    }
    assert!(
        ids_by_day.values().all(|n| *n == 1),
        "a time line twice in a day"
    );
    assert_eq!(groups.first().map(String::as_str), Some("first_run"));
    // No three draws in a row from one group (steps are not draws).
    for w in groups.windows(3) {
        if !STEPS.contains(&w[0].as_str()) {
            assert!(!(w[0] == w[1] && w[1] == w[2]), "{w:?}");
        }
    }
    assert!(events <= 365 / 30 + 1, "{events} events in a year");
    assert!(per_group.get("rare").copied().unwrap_or(0) <= 365 / 7 + 1);
    for g in [
        "scope", "work", "facts", "tips", "agents", "time", "glyph", "mine", "day",
    ] {
        assert!(
            per_group.get(g).copied().unwrap_or(0) > 0,
            "no {g} line in a year: {per_group:?}"
        );
    }
    // The state a year makes is small.
    let text = toml::to_string(&state).expect("serializes");
    assert!(text.len() < 20_000, "{} bytes", text.len());
}
