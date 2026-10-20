//! A message as the screen, the detail pane and the pages draw it: mrkdwn
//! turned into the kit's rich text (links, mentions named from the cache,
//! emoji as glyphs where known, code, quotes, files) and laid out by the
//! kit, with the look words choosing the slots.

use radar_core::ratatui::style::Style;
use radar_core::ratatui::text::Span;
use radar_core::ui::icon::{self, Icon};
use radar_core::ui::rich::{self, Block, Doc, Inline, Marks};

use crate::client::Message;
use crate::config::Settings;
use crate::model::{Data, Namer, ts_secs};
use crate::mrkdwn::{self, DLine, Kind};

/// The text of a message as rich text: people, groups and channels named
/// from the cache (you and your groups marked as you), emoji as glyphs
/// where one is known, a run of `>` lines a quote, a ``` block code, and
/// paragraphs where a blank line parts them.
pub fn doc(data: &Data, cfg: &Settings, text: &str) -> Doc {
    let names = Namer { data, cfg };
    let me = data.me.as_ref().map(|m| m.user_id.as_str());
    let mine: Vec<&str> = data.my_groups().iter().map(|g| g.id.as_str()).collect();
    let is_me = |k: &Kind| match k {
        Kind::User { id } => Some(id.as_str()) == me,
        Kind::Group { id } => mine.contains(&id.as_str()),
        _ => false,
    };
    from_lines(mrkdwn::parse(text, &names), &is_me)
}

/// Lines of the same kind, together.
fn from_lines(lines: Vec<DLine>, is_me: &dyn Fn(&Kind) -> bool) -> Doc {
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let (quote, pre) = (lines[i].quote, lines[i].pre);
        let start = i;
        while i < lines.len() && lines[i].quote == quote && lines[i].pre == pre {
            i += 1;
        }
        let group = &lines[start..i];
        if pre {
            let text = group
                .iter()
                .map(|l| l.segs.iter().map(|s| s.text.as_str()).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n");
            blocks.push(Block::Code {
                lang: String::new(),
                text,
            });
            continue;
        }
        let paras = paragraphs(group, is_me);
        if quote {
            blocks.push(Block::Quote { blocks: paras });
        } else {
            blocks.extend(paras);
        }
    }
    Doc::new(blocks)
}

/// Lines as paragraphs: a blank line parts them, other lines break.
fn paragraphs(lines: &[DLine], is_me: &dyn Fn(&Kind) -> bool) -> Vec<Block> {
    let mut out = Vec::new();
    let mut runs: Vec<Inline> = Vec::new();
    for l in lines {
        if l.segs.iter().all(|s| s.text.trim().is_empty()) {
            if !runs.is_empty() {
                out.push(Block::para(std::mem::take(&mut runs)));
            }
            continue;
        }
        if !runs.is_empty() {
            runs.push(Inline::line_break());
        }
        runs.extend(l.segs.iter().map(|s| inline(s, is_me)));
    }
    if !runs.is_empty() {
        out.push(Block::para(runs));
    }
    out
}

fn inline(s: &mrkdwn::Seg, is_me: &dyn Fn(&Kind) -> bool) -> Inline {
    let marks = Marks {
        bold: s.bold,
        italic: s.italic,
        strike: s.strike,
        code: s.code,
        underline: false,
    };
    let run = match &s.kind {
        Kind::Text => Inline::text(s.text.clone()),
        Kind::Link { url } => Inline::link(s.text.clone(), url.clone()),
        Kind::User { .. } | Kind::Group { .. } => Inline::mention(s.text.clone(), is_me(&s.kind)),
        Kind::Channel { .. } | Kind::Broadcast => Inline::mention(s.text.clone(), false),
        Kind::Emoji { name } => Inline::emoji(rich::emoji(name).unwrap_or_else(|| s.text.clone())),
    };
    run.with(marks)
}

/// A message's one line as spans: its first line of text with `:name:`
/// emoji as their glyphs, a custom one (no glyph) kept as `:name:` in
/// `emoji`; or what it carries, in `text`.
pub fn first_line_spans(
    data: &Data,
    cfg: &Settings,
    m: &Message,
    text: Style,
    emoji: Style,
) -> Vec<Span<'static>> {
    let names = Namer { data, cfg };
    let line = mrkdwn::parse(&m.text, &names)
        .into_iter()
        .find(|l| l.segs.iter().any(|s| !s.text.trim().is_empty()));
    let Some(line) = line else {
        return vec![Span::styled(first_line(data, cfg, m), text)];
    };
    let mut spans: Vec<Span<'static>> = Vec::new();
    for s in line.segs {
        let (words, style) = match &s.kind {
            Kind::Emoji { name } => match rich::emoji(name) {
                Some(glyph) => (glyph, text),
                None => (s.text.clone(), emoji),
            },
            _ => (s.text.clone(), text),
        };
        match spans.last_mut() {
            Some(last) if last.style == style => {
                last.content = format!("{}{words}", last.content).into()
            }
            _ => spans.push(Span::styled(words, style)),
        }
    }
    if let Some(first) = spans.first_mut() {
        first.content = first.content.trim_start().to_string().into();
    }
    if let Some(last) = spans.last_mut() {
        last.content = last.content.trim_end().to_string().into();
    }
    spans.retain(|s| !s.content.is_empty());
    spans
}

/// A message's one line: its first line of text (emoji as glyphs where
/// one is known), or what it carries.
pub fn first_line(data: &Data, cfg: &Settings, m: &Message) -> String {
    let line = mrkdwn::first_line(&m.text, &Namer { data, cfg });
    if !line.is_empty() {
        return first_line_spans(data, cfg, m, Style::default(), Style::default())
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
    }
    if let Some(s) = m.shared.first() {
        let said = mrkdwn::first_line(&s.text, &Namer { data, cfg });
        return match s.author.as_str() {
            "" => format!("shared a message: {said}"),
            who => format!("shared {who}'s message: {said}"),
        };
    }
    match (m.files.first(), m.subtype.as_deref()) {
        (Some(f), _) => format!("shared {}", f.name),
        (None, Some(s)) => s.replace('_', " "),
        _ => "(no text)".into(),
    }
}

/// `40s`, `3d`, `6w`: how long ago a ts was, the kit's short form.
pub fn age(ts: &str, now: i64) -> String {
    radar_core::time::ago(ts_secs(ts), now)
}

/// `Tue 6 Oct 13:27`, or `13:27` today: when a ts was, in local time.
pub fn when(ts: &str) -> String {
    when_at(ts, radar_core::time::now_secs())
}

/// [`when`] as it reads at `now`.
pub fn when_at(ts: &str, now: i64) -> String {
    when_secs(ts_secs(ts), now)
}

/// [`when`] for a time in seconds.
pub fn when_secs(secs: i64, now: i64) -> String {
    radar_core::time::when_in(secs, now, zone(secs), zone(now))
}

/// Seconds the local zone is ahead of UTC at `epoch`; tests read UTC so they
/// say the same wherever they run.
fn zone(epoch: i64) -> i64 {
    if cfg!(test) {
        0
    } else {
        radar_core::time::local_offset(epoch)
    }
}

/// `06:46`: the time of day of a ts, in local time, for a conversation's lines.
pub fn clock(ts: &str) -> String {
    let secs = ts_secs(ts);
    clock_in(secs, zone(secs))
}

/// [`clock`] in the zone `offset` seconds ahead of UTC.
pub fn clock_in(secs: i64, offset: i64) -> String {
    let rem = (secs + offset).rem_euclid(86_400);
    format!("{:02}:{:02}", rem / 3600, rem % 3600 / 60)
}

/// `2026-10-06`: a day for Slack's search (`after:`), in UTC.
pub fn date(secs: i64) -> String {
    let (y, m, d) = civil(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// The day a ts falls on, in words, relative to `now` (local time): `today`,
/// `yesterday`, `Mon 5 Oct`.
pub fn day(ts: &str, now: i64) -> String {
    let secs = ts_secs(ts);
    day_in(secs, now, zone(secs), zone(now))
}

/// [`day`] with the zone's offsets for the ts and for `now`.
pub fn day_in(secs: i64, now: i64, offset: i64, now_offset: i64) -> String {
    let d = (secs + offset).div_euclid(86_400);
    let today = (now + now_offset).div_euclid(86_400);
    match today - d {
        0 => "today".into(),
        1 => "yesterday".into(),
        _ => {
            const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
            const MONTHS: [&str; 12] = [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ];
            let (_, m, dd) = civil(d);
            let weekday = (d + 3).rem_euclid(7) as usize;
            format!("{} {dd} {}", DAYS[weekday], MONTHS[(m - 1) as usize])
        }
    }
}

/// Days since 1970-01-01 → (year, month, day) (Howard Hinnant's algorithm).
fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `👀 2  👍 1`: reactions as text, a glyph where one is known, else
/// `:name:`.
pub fn reactions(m: &Message) -> String {
    m.reactions
        .iter()
        .map(|r| {
            let base = r.name.split("::").next().unwrap_or(&r.name);
            let face = rich::emoji(base).unwrap_or_else(|| format!(":{}:", r.name));
            format!("{face} {}", r.count)
        })
        .collect::<Vec<_>>()
        .join("  ")
}

/// `1 reply`, `4 replies`.
pub fn replies(n: u32) -> String {
    match n {
        0 => "no replies".into(),
        1 => "1 reply".into(),
        n => format!("{n} replies"),
    }
}

/// A channel's icon, by its id: `G…` is an old private channel, `D…` a
/// direct message.
pub fn channel_icon(id: &str) -> Icon {
    match id.chars().next() {
        Some('G') => Icon::PrivateChannel,
        Some('D') => Icon::Direct,
        _ => Icon::Channel,
    }
}

/// Who wrote a message, for a row: `you` for the reader, else the short name.
pub fn who(data: &Data, m: &Message) -> String {
    let me = data.me.as_ref().map(|m| m.user_id.as_str());
    if author_icon(me, m) == Icon::You {
        "you".to_string()
    } else {
        radar_core::ui::short_name(&data.author(m))
    }
}

/// The icon of who wrote a message: you, a bot, a person.
pub fn author_icon(me: Option<&str>, m: &Message) -> Icon {
    match (&m.user, &m.bot_name) {
        (Some(u), _) if Some(u.as_str()) == me => Icon::You,
        (Some(_), _) => Icon::Person,
        (None, Some(_)) => Icon::Bot,
        _ => Icon::Person,
    }
}

/// The icon of something a thread can open, by the label `openable` gives
/// it: `file notes.csv`, `image a.png`, `shared: …`, else a link.
pub fn link_icon(label: &str) -> Icon {
    if let Some(name) = label.strip_prefix("file ") {
        Icon::for_file(name)
    } else if label.starts_with("image ") {
        Icon::FileImage
    } else if label.starts_with("shared: ") {
        Icon::Thread
    } else {
        Icon::Link
    }
}

/// A failure's notice: the error icon (the host draws it in its colour),
/// else `✗`, before the words.
pub fn failed(words: &str) -> radar_core::Effect {
    let (icon, mark) = icon::plain_or(Icon::Error, "✗");
    radar_core::Effect::error(icon, format!("{mark}{words}"))
}

/// A channel's name for a menu heading and the icon to name before it:
/// `#alias` with icons off, the channel's icon and `alias` with them on.
pub fn channel_heading(id: &str, alias: &str) -> (Option<Icon>, String) {
    match icon::plain_or(channel_icon(id), "#") {
        (Some(i), _) => (Some(i), alias.to_string()),
        (None, _) => (None, format!("#{alias}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod notice_severity_tests {
        use super::*;

        #[test]
        fn failed_notice_keeps_error_tone_with_icons_on_and_off() {
            use radar_core::ui::icon as packs;
            // Restore the thread-local pack even if this helper contract fails.
            struct Restore(packs::Icons);
            impl Drop for Restore {
                fn drop(&mut self) { packs::set_current(&self.0); }
            }
            let _restore = Restore(packs::current());
            for pack in ["off", "unicode"] {
                let (icons, warnings) = packs::resolve(pack, &[], &Default::default());
                assert!(warnings.is_empty());
                packs::set_current(&icons);
                let radar_core::Effect::Notice(message) = failed("request refused") else {
                    panic!("failure must carry semantic error severity");
                };
                assert_eq!(message.tone, radar_core::view::chrome::NoticeTone::Error);
                assert_eq!(message.icon, (pack != "off").then_some(Icon::Error));
                assert_eq!(message.text, if pack == "off" { "✗ request refused" } else { "request refused" });
            }
        }
    }

    #[test]
    fn a_channels_icon_follows_its_id() {
        assert_eq!(channel_icon("C0123"), Icon::Channel);
        assert_eq!(channel_icon("G0123"), Icon::PrivateChannel);
        assert_eq!(channel_icon("D0123"), Icon::Direct);
    }

    #[test]
    fn what_a_thread_opens_has_the_icon_of_what_it_is() {
        assert_eq!(link_icon("file main.rs"), Icon::FileRust);
        assert_eq!(link_icon("image a.png"), Icon::FileImage);
        assert_eq!(link_icon("shared: Bob's message"), Icon::Thread);
        assert_eq!(link_icon("the docs"), Icon::Link);
    }

    #[test]
    fn an_author_is_you_a_bot_or_a_person() {
        let mut m = Message {
            user: Some("U1".into()),
            ..Message::default()
        };
        assert_eq!(author_icon(Some("U1"), &m), Icon::You);
        assert_eq!(author_icon(Some("U2"), &m), Icon::Person);
        m.user = None;
        m.bot_name = Some("ci".into());
        assert_eq!(author_icon(None, &m), Icon::Bot);
    }

    fn as_doc(text: &str) -> Doc {
        let is_me = |k: &Kind| matches!(k, Kind::User { id } if id == "U1");
        from_lines(mrkdwn::parse(text, &mrkdwn::NoNames), &is_me)
    }

    #[test]
    fn a_message_becomes_paragraphs_quotes_and_code_with_mentions_and_glyphs() {
        let doc = as_doc(
            "hi <@U1|alice> and <@U2|bob> :tada: :our-parrot:\nsecond line\n\n&gt; quoted\n&gt; more\n```\nlet a = 1;\n```\nafter",
        );
        assert_eq!(
            rich::plain(&doc, 60),
            "hi @alice and @bob 🎉 :our-parrot:\nsecond line\n\n│ quoted\n│ more\n\n  let a = 1;\n\nafter"
        );
        let Block::Para { text } = &doc.blocks[0] else {
            panic!("{doc:?}")
        };
        assert_eq!(text[1], Inline::mention("@alice", true));
        assert_eq!(text[3], Inline::mention("@bob", false));
        assert!(matches!(&doc.blocks[1], Block::Quote { .. }));
        assert!(matches!(&doc.blocks[2], Block::Code { text, .. } if text == "let a = 1;"));
    }

    #[test]
    fn a_rows_line_has_glyphs_and_keeps_a_custom_emoji_as_its_name_in_the_emoji_style() {
        let m = Message {
            text: "  \n:rocket: shipped :our-parrot: today :tada:".into(),
            ..Message::default()
        };
        let (data, cfg) = (Data::default(), Settings::default());
        assert_eq!(
            first_line(&data, &cfg, &m),
            "🚀 shipped :our-parrot: today 🎉"
        );
        let (text, emoji) = (Style::new().bold(), Style::new().italic());
        let spans = first_line_spans(&data, &cfg, &m, text, emoji);
        let got: Vec<(&str, Style)> = spans
            .iter()
            .map(|s| (s.content.as_ref(), s.style))
            .collect();
        assert_eq!(
            got,
            [
                ("🚀 shipped ", text),
                (":our-parrot:", emoji),
                (" today 🎉", text)
            ]
        );
    }

    #[test]
    fn reactions_show_glyphs_where_known() {
        let mut m = Message::default();
        let r = |name: &str, count| crate::client::Reaction {
            name: name.into(),
            count,
            users: vec![],
        };
        m.reactions = vec![r("eyes", 2), r("+1::skin-tone-2", 1), r("our-parrot", 3)];
        assert_eq!(reactions(&m), "👀 2  👍 1  :our-parrot: 3");
    }

    #[test]
    fn days_and_times_read_in_words() {
        // 2026-10-06 00:00 UTC, a Tuesday.
        let tue = 1_791_244_800;
        assert_eq!(day_in(tue + 3600, tue + 7200, 0, 0), "today");
        assert_eq!(day_in(tue - 3600, tue + 7200, 0, 0), "yesterday");
        assert_eq!(day_in(tue - 2 * 86_400, tue, 0, 0), "Sun 4 Oct");
        assert_eq!(age(&format!("{}.0", tue), tue + 3 * 86_400 + 5), "3d");
        assert_eq!(age(&format!("{}.0", tue), tue + 30), "30s");
        assert_eq!(age(&format!("{}.0", tue), tue + 90 * 86_400), "12w");
    }

    #[test]
    fn local_time_moves_the_day_and_the_clock_with_the_zone() {
        let tue = 1_791_244_800;
        // 23:30 UTC on Monday is 00:30 on Tuesday two hours east of it.
        let late = tue - 1800;
        assert_eq!(clock_in(late, 0), "23:30");
        assert_eq!(clock_in(late, 7200), "01:30");
        assert_eq!(day_in(late, tue + 3600, 7200, 7200), "today");
        assert_eq!(day_in(late, tue + 3600, 0, 0), "yesterday");
        // An absolute time is the kit's: the clock alone today, never UTC-fixed.
        assert_eq!(
            when_at(&format!("{}.5", tue + 14 * 3600 + 120), tue + 15 * 3600).len(),
            5
        );
    }
}
