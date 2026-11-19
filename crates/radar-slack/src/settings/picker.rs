//! Picking the watched channels from the ones you are in: after a good
//! check, the channels Slack lists for the token (public and private) as
//! rows to tick, under the registry in setup's connection card and in
//! Settings, both the connection and the channels section. A ticked channel is
//! watched; ticking one that is not in the registry adds it as a team
//! channel, unticking one that is keeps it there, unwatched. Pasting a link
//! or an id (`add a channel`) stays for the channels you are not in.

use radar_core::plugin::{Effect, Menu, MenuItem, Msg};
use radar_core::ui::icon::Icon;
use radar_core::view as v;

use super::{FIELD_LABEL_W, WsRow, ChRow, ws_rows, ch_rows};
use crate::client::ChannelInfo;
use crate::{SlackMsg, SlackPlugin};

/// Rows drawn at most: a workspace can put you in thousands of channels, and
/// the filter is how to reach the rest.
pub(crate) const MAX_SHOWN: usize = 200;

/// Where the list of your channels stands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum State {
    /// Not asked yet: nothing is drawn.
    #[default]
    NotAsked,
    Loading,
    Ready,
    Failed(String),
}

/// What Slack listed for the token, and the words that narrow it.
#[derive(Debug, Default)]
pub(crate) struct Joined {
    pub(crate) state: State,
    pub(crate) channels: Vec<ChannelInfo>,
    /// Private channels were left out: the token has no `groups:read`.
    pub(crate) private_skipped: bool,
    /// radar stopped at its page limit.
    pub(crate) truncated: bool,
    pub(crate) filter: String,
}

/// A row of the picker, in either section's flat list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PickRow {
    /// `YOUR CHANNELS`: not selectable.
    Heading,
    /// How the list stands; ⏎ asks Slack again.
    Status,
    /// The words that narrow the list.
    Filter,
    /// A channel you are in, by its place in [`Joined::channels`].
    Channel(usize),
}

impl PickRow {
    pub(crate) const fn selectable(self) -> bool {
        !matches!(self, Self::Heading)
    }
}

/// The rows the picker adds: none until the list was asked for.
pub(crate) fn rows(p: &SlackPlugin) -> Vec<PickRow> {
    if !p.authenticated() || p.joined.state == State::NotAsked {
        return vec![];
    }
    let mut rows = vec![PickRow::Heading, PickRow::Status];
    if p.joined.state == State::Ready && !p.joined.channels.is_empty() {
        rows.push(PickRow::Filter);
        rows.extend(visible(p).into_iter().take(MAX_SHOWN).map(PickRow::Channel));
    }
    rows
}

/// The channels the filter lets through (all of them with none), as places
/// in [`Joined::channels`]. Case does not matter; the name and the purpose
/// are looked in.
pub(crate) fn visible(p: &SlackPlugin) -> Vec<usize> {
    let words: Vec<String> = p.joined.filter.split_whitespace().map(str::to_lowercase).collect();
    p.joined
        .channels
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            let hay = format!("{} {} {}", c.name, c.purpose, c.topic).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .map(|(i, _)| i)
        .collect()
}

fn registered(p: &SlackPlugin, id: &str) -> Option<usize> {
    p.cfg.channels.iter().position(|c| c.id == id)
}

/// Whether a channel is ticked: in the registry and watched.
fn ticked(p: &SlackPlugin, id: &str) -> bool {
    registered(p, id).is_some_and(|i| p.cfg.channels[i].watched)
}

fn watched_count(p: &SlackPlugin) -> usize {
    p.joined.channels.iter().filter(|c| ticked(p, &c.id)).count()
}

fn channel_at(p: &SlackPlugin, k: usize) -> Option<&ChannelInfo> {
    p.joined.channels.get(k)
}

/// The words the filter row shows for its value.
fn filter_words(p: &SlackPlugin) -> v::Text {
    if p.joined.filter.trim().is_empty() {
        v::Text::of("— type part of a name", v::Ink::Muted)
    } else {
        v::Text::of(p.joined.filter.clone(), v::Ink::Primary)
    }
}

fn plural(n: usize) -> String {
    if n == 1 { "1 channel".into() } else { format!("{n} channels") }
}

/// A row as the tree draws it, and where the typed filter starts in it when
/// that is being typed.
pub(crate) fn list_row(p: &SlackPlugin, row: PickRow, id: String) -> (v::ListRow, Option<u16>) {
    match row {
        PickRow::Heading => (
            v::ListRow::heading(
                id,
                v::Text::plain(" ")
                    .push(v::Seg::new("YOUR CHANNELS", v::Ink::Muted).strong())
                    .push(v::Seg::new("  tick the ones to watch", v::Ink::Muted)),
            ),
            None,
        ),
        PickRow::Status => (v::ListRow::new(id, status_text(p)), None),
        PickRow::Filter => {
            let shown = visible(p).len();
            let mut list = v::ListRow::new(id, v::kit::labelled("filter", FIELD_LABEL_W, filter_words(p)));
            let total = p.joined.channels.len();
            list.right = Some(v::Text::of(
                if shown > MAX_SHOWN { format!("first {MAX_SHOWN} of {shown}") } else { format!("{shown} of {total}") },
                v::Ink::Muted,
            ));
            let typing = matches!(p.edit, Some((crate::Target::JoinedFilter, _)));
            (list, typing.then_some(FIELD_LABEL_W as u16))
        }
        PickRow::Channel(k) => {
            let Some(info) = channel_at(p, k) else {
                return (v::ListRow::new(id, v::Text::new()), None);
            };
            let mut text = v::Text::new();
            text.add(if ticked(p, &info.id) {
                v::Seg::icon_or(Icon::Done, "[x] ", v::Ink::Good)
            } else {
                v::Seg::icon_or(Icon::Todo, "[ ] ", v::Ink::Muted)
            });
            text.add(v::Seg::icon(if info.is_private { Icon::PrivateChannel } else { Icon::Channel }));
            text.add(v::Seg::styled(format!("#{}", info.name), p.cfg.style_for("channel")));
            let about = if info.purpose.trim().is_empty() { &info.topic } else { &info.purpose };
            if !about.trim().is_empty() {
                text.add(v::Seg::new(
                    format!("   {}", radar_core::ui::fit(about.lines().next().unwrap_or(""), 48).trim_end()),
                    v::Ink::Muted,
                ));
            }
            (v::ListRow::new(id, text), None)
        }
    }
}

fn status_text(p: &SlackPlugin) -> v::Text {
    let j = &p.joined;
    match &j.state {
        State::NotAsked => v::Text::new(),
        State::Loading => v::Text::of("asking Slack for the channels you are in…", v::Ink::Info),
        State::Failed(why) => v::Text::new()
            .push(v::Seg::icon(Icon::Error))
            .push(v::Seg::new(why.clone(), v::Ink::Bad)),
        State::Ready => {
            let mut text = v::Text::of(
                if j.channels.is_empty() {
                    "Slack lists no channel you are in".to_string()
                } else {
                    format!("{} you are in · {} watched", plural(j.channels.len()), watched_count(p))
                },
                v::Ink::Secondary,
            );
            if j.private_skipped {
                text.add(v::Seg::new("   ", v::Ink::Muted));
                text.add(v::Seg::icon_or_lead(Icon::Warning, "", v::Ink::Warn));
                text.add(v::Seg::new("public ones only: no groups:read", v::Ink::Warn));
            }
            if j.truncated {
                text.add(v::Seg::new("   ", v::Ink::Muted));
                text.add(v::Seg::icon_or_lead(Icon::Warning, "", v::Ink::Warn));
                text.add(v::Seg::new("list cut short", v::Ink::Warn));
            }
            if !j.channels.is_empty() && visible(p).is_empty() {
                text.add(v::Seg::new("   none match the filter", v::Ink::Muted));
            }
            text
        }
    }
}

/// What the detail pane says about a picker row.
pub(crate) fn detail(p: &SlackPlugin, row: PickRow) -> Vec<v::Text> {
    let j = &p.joined;
    match row {
        PickRow::Heading | PickRow::Status => {
            let mut detail = vec![
                v::Text::plain("The channels you are in, public and private, as Slack lists them for the token."),
                v::Text::new(),
                v::Text::of(
                    "Tick the ones to watch: a ticked channel is added to the registry as a team channel and read on every refresh. Unticking one keeps it in the registry, not read.",
                    v::Ink::Muted,
                ),
                v::Text::new(),
                v::Text::of("A channel you are not in is added with its link or id, under add a channel.", v::Ink::Muted),
            ];
            if let State::Failed(why) = &j.state {
                detail.splice(0..0, [v::kit::heading("could not list them"), v::Text::of(why.clone(), v::Ink::Bad), v::Text::new()]);
            }
            if j.private_skipped {
                detail.extend([
                    v::Text::new(),
                    v::kit::heading("private channels left out"),
                    v::Text::of(
                        "The token has no groups:read, so Slack lists the public channels only. Add groups:read to the Slack app (radar's manifest asks for it), install it again and check the connection.",
                        v::Ink::Warn,
                    ),
                ]);
            }
            if j.truncated {
                detail.extend([
                    v::Text::new(),
                    v::kit::heading("list cut short"),
                    v::Text::of(
                        "radar stopped at its page limit. Filter to narrow the list, or add the channel with its link.",
                        v::Ink::Warn,
                    ),
                ]);
            }
            detail
        }
        PickRow::Filter => {
            let mut detail = Vec::new();
            if let Some((target @ crate::Target::JoinedFilter, input)) = &p.edit {
                let verdict = super::validate(p, target, &input.value());
                detail.extend([
                    v::kit::heading("as you type"),
                    v::kit::as_you_type(verdict.as_deref().map_err(String::as_str)),
                    v::Text::new(),
                ]);
            }
            detail.extend([
                v::Text::plain("Narrows the list to the channels whose name or purpose has every word."),
                v::Text::new(),
                v::kit::labelled("now", 6, filter_words(p)),
                v::kit::labelled("shown", 6, v::Text::plain(format!("{} of {}", visible(p).len().min(MAX_SHOWN), j.channels.len()))),
            ]);
            detail
        }
        PickRow::Channel(k) => {
            let Some(info) = channel_at(p, k) else { return vec![] };
            let mut detail = vec![v::kit::heading("watched"), match registered(p, &info.id) {
                Some(i) if p.cfg.channels[i].watched => {
                    v::Text::of(format!("yes, as #{}", p.cfg.channels[i].alias), v::Ink::Good)
                }
                Some(i) => v::Text::of(format!("no: in the registry as #{}, not read", p.cfg.channels[i].alias), v::Ink::Muted),
                None => v::Text::of("no: not in the registry", v::Ink::Muted),
            }, v::Text::new()];
            detail.extend([
                v::kit::labelled("in Slack", 10, v::Text::plain(format!("#{}", info.name))),
                v::kit::labelled("kind", 10, v::Text::plain(if info.is_private { "private" } else { "public" })),
                v::kit::labelled("id", 10, v::Text::plain(info.id.clone())),
            ]);
            for (label, value) in [("topic", &info.topic), ("purpose", &info.purpose)] {
                if !value.trim().is_empty() {
                    detail.push(v::kit::labelled(label, 10, v::Text::of(value.clone(), v::Ink::Secondary)));
                }
            }
            detail
        }
    }
}

/// The row's options: ⏎ and space tick a channel, ⏎ on the list's status
/// asks Slack again, ⏎ on the filter types it.
pub(crate) fn menu(p: &SlackPlugin, row: PickRow) -> Option<Menu> {
    Some(match row {
        PickRow::Heading => return None,
        PickRow::Channel(k) => {
            let info = channel_at(p, k)?;
            let on = ticked(p, &info.id);
            Menu::new(
                format!("joined:{}", info.id),
                format!("#{}", info.name),
                vec![
                    MenuItem::new(
                        ' ',
                        if on { "leave unwatched" } else { "receive updates" },
                        if on { "kept in the registry, not read" } else { "read on every refresh" },
                        "toggle_watch",
                    ),
                    MenuItem::new('r', "refresh the list", "ask Slack for the channels you are in again", "list"),
                ],
            )
        }
        PickRow::Status => Menu::new(
            "joined:",
            "your channels",
            vec![MenuItem::new('r', "refresh the list", "ask Slack for the channels you are in again", "list")],
        ),
        PickRow::Filter => {
            let mut items = vec![MenuItem::new('e', "change it", "part of a name or purpose", "filter")];
            if !p.joined.filter.trim().is_empty() {
                items.push(MenuItem::new('c', "clear it", "show every channel", "clear_filter"));
            }
            Menu::new("joined:", "filter", items)
        }
    })
}

/// A choice from [`menu`], or from the section's bulk menu.
pub(crate) fn on_menu(p: &mut SlackPlugin, id: &str, value: &str) -> Vec<Effect> {
    match value {
        "toggle_watch" => toggle(p, id),
        "list" => load(p),
        "filter" => {
            p.edit = Some((crate::Target::JoinedFilter, radar_core::input::TextInput::new(&p.joined.filter)));
            vec![]
        }
        "clear_filter" => {
            p.joined.filter.clear();
            select_filter(p);
            vec![]
        }
        _ => vec![],
    }
}

/// The typed filter was kept.
pub(crate) fn set_filter(p: &mut SlackPlugin, words: String) -> Vec<Effect> {
    p.joined.filter = words;
    select_filter(p);
    vec![]
}

fn select_filter(p: &mut SlackPlugin) {
    if matches!(ws_rows(p).get(p.section_rows[0]), Some(WsRow::Pick(_)))
        && let Some(i) = ws_rows(p).iter().position(|r| *r == WsRow::Pick(PickRow::Filter))
    {
        p.section_rows[0] = i;
    }
    if matches!(ch_rows(p).get(p.section_rows[1]), Some(ChRow::Pick(_)))
        && let Some(i) = ch_rows(p).iter().position(|r| *r == ChRow::Pick(PickRow::Filter))
    {
        p.section_rows[1] = i;
    }
}

/// Tick or untick a channel (by its id) in the list.
pub(crate) fn toggle(p: &mut SlackPlugin, id: &str) -> Vec<Effect> {
    let Some(info) = p.joined.channels.iter().find(|c| c.id == id).cloned() else {
        return vec![];
    };
    // Where each section's cursor is on this channel, so it stays on it
    // when the registry above it grows.
    let in_connection = matches!(ws_rows(p).get(p.section_rows[0]), Some(WsRow::Pick(PickRow::Channel(k))) if channel_at(p, *k).is_some_and(|c| c.id == id));
    let in_channels = matches!(ch_rows(p).get(p.section_rows[1]), Some(ChRow::Pick(PickRow::Channel(k))) if channel_at(p, *k).is_some_and(|c| c.id == id));
    let fx = match registered(p, id) {
        Some(i) => {
            let channel = &mut p.cfg.channels[i];
            channel.watched = !channel.watched;
            if channel.watched {
                let mut fx = vec![Effect::Notify(format!("watching #{}", channel.alias))];
                fx.extend(p.fetch_now());
                fx
            } else {
                vec![Effect::Notify(format!("#{} is not read now; it stays in the registry", channel.alias))]
            }
        }
        None => {
            let (_, alias) = super::register(p, &info);
            let mut fx = vec![Effect::Notify(if alias == info.name {
                format!("watching #{}", info.name)
            } else {
                format!("watching #{} as {alias}", info.name)
            })];
            fx.extend(p.fetch_now());
            fx
        }
    };
    let k = p.joined.channels.iter().position(|c| c.id == id);
    if let Some(k) = k {
        if in_connection && let Some(i) = ws_rows(p).iter().position(|r| *r == WsRow::Pick(PickRow::Channel(k))) {
            p.section_rows[0] = i;
        }
        if in_channels && let Some(i) = ch_rows(p).iter().position(|r| *r == ChRow::Pick(PickRow::Channel(k))) {
            p.section_rows[1] = i;
        }
    }
    p.changed();
    fx
}

/// Ask Slack for the channels the token is in.
pub(crate) fn load(p: &mut SlackPlugin) -> Vec<Effect> {
    let stamp = p.begin_check("joined");
    let action = crate::settings_history::joined_action();
    let c = match p.client() {
        Ok(c) if p.authenticated() => c,
        other => {
            p.complete_check("joined", stamp);
            let why = other.err().unwrap_or_else(|| "check the Slack connection first, then try again".into());
            p.joined.state = State::Failed(why.clone());
            return vec![crate::settings_history::joined_unavailable(), crate::view::failed(&why)];
        }
    };
    // A token known to lack groups:read is not asked for private channels:
    // that is refused. One whose scopes are not known is asked, and falls back.
    let private = p.data.me.as_ref().is_none_or(|me| me.scopes.is_empty() || me.scopes.iter().any(|s| s == "groups:read"));
    p.joined.state = State::Loading;
    let captured = action.clone();
    vec![Effect::SpawnAction {
        action,
        task: Box::pin(async move {
            let result = c.joined_channels(private).await.map_err(|e| format!("{e:#}"));
            Box::new(SlackMsg::Joined { stamp, action: captured, result }) as Msg
        }),
    }]
}

/// Slack's list came back: held, and drawn.
pub(crate) fn on_joined(p: &mut SlackPlugin, result: Result<crate::client::JoinedRead, String>) -> Vec<Effect> {
    match result {
        Ok(got) => {
            p.joined.private_skipped = got.private_skipped;
            p.joined.truncated = got.truncated;
            p.joined.channels = got.channels;
            p.joined.state = State::Ready;
            vec![]
        }
        Err(why) => {
            p.joined.state = State::Failed(why.clone());
            vec![crate::view::failed(&why)]
        }
    }
}

#[cfg(test)]
mod tests;
