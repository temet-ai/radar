use radar_core::design::theme::{Slot, Tier};
use radar_core::motion::*;
use radar_core::theme::{Theme, ThemeConfig};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use serde_json::{json, Value};

const AREA: Rect = Rect { x: 0, y: 0, width: 30, height: 8 };
const POPUP: Rect = Rect { x: 5, y: 2, width: 12, height: 4 };

fn col(c: Color) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Reset => "reset".into(),
        o => format!("{o:?}"),
    }
}
fn plain(theme: &Theme) -> Buffer {
    let mut b = Buffer::empty(AREA);
    b.set_style(AREA, theme.bg(Slot::BgSurface));
    for y in 0..AREA.height {
        b.set_string(0, y, format!("row {y} with some text"), theme.s(Slot::TextPrimary));
    }
    b
}
fn row_rect(y: u16) -> Rect { Rect { y, height: 1, ..AREA } }
fn marks() {
    region(Region::Body, AREA);
    region(Region::Popup, POPUP);
    region(Region::Message, Rect { x: 0, y: 7, width: 14, height: 1 });
    for y in 0..AREA.height { row(format!("row:{y}"), row_rect(y)); }
}
fn marks_loading() { marks(); loading(AREA); }
fn dump(b: &Buffer) -> Value {
    let mut v = vec![];
    for y in 0..AREA.height { for x in 0..AREA.width {
        let c = &b[(x, y)];
        v.push(json!([c.symbol(), col(c.fg), col(c.bg), format!("{:?}", c.modifier)]));
    } }
    Value::Array(v)
}
fn run(name: &str, level: Level, cue: Option<Cue>, theme: &Theme, times: &[u64], mk: fn()) -> Value {
    let clock = ManualClock::default();
    let mut e = Engine::new(level, Box::new(clock.clone()));
    let mut frames = vec![];
    if name == "message_gone" {
        begin(); mk();
        let mut b = plain(theme);
        e.draw(&mut b, theme);
    }
    if let Some(c) = cue { e.cue(c, theme); }
    let mut at = 0;
    for &t in times {
        clock.advance(t - at); at = t;
        begin(); mk();
        let mut b = plain(theme);
        if name == "message_gone" { b = Buffer::empty(AREA); b.set_style(AREA, theme.bg(Slot::BgSurface)); b.set_string(0, 7, "keys keys keys", theme.s(Slot::TextMuted)); }
        e.draw(&mut b, theme);
        frames.push(json!({"t": t, "running": e.running(), "cells": dump(&b)}));
    }
    json!({"name": name, "level": level.word(), "frames": frames})
}
fn main() {
    let theme = Theme::build(&ThemeConfig::default(), Tier::TrueColor, &[]);
    let c = |s| col(theme.bg(s).bg.unwrap_or(Color::Reset));
    let f = |s| col(theme.s(s).fg.unwrap_or(Color::Reset));
    let colours = json!({"base": c(Slot::BgBase), "surface": c(Slot::BgSurface), "overlay": c(Slot::BgOverlay), "glow": f(Slot::AccentPrimary), "text": f(Slot::TextPrimary), "muted": f(Slot::TextMuted)});
    let mut ch = Changes::default();
    ch.changed.insert("row:3".into());
    ch.added.insert("row:5".into());
    let ts: Vec<u64> = vec![0, 30, 60, 100, 140, 180, 200, 230, 260, 300, 340, 400, 450, 500];
    let mut sc = vec![];
    sc.push(run("plain", Level::Full, None, &theme, &[0], marks));
    sc.push(run("screen", Level::Subtle, Some(Cue::Screen), &theme, &ts, marks));
    sc.push(run("page", Level::Full, Some(Cue::Page), &theme, &ts, marks));
    sc.push(run("popup", Level::Subtle, Some(Cue::Popup), &theme, &ts, marks));
    sc.push(run("message", Level::Full, Some(Cue::Message), &theme, &ts, marks));
    sc.push(run("message_gone", Level::Full, Some(Cue::MessageGone), &theme, &ts, marks));
    sc.push(run("changed_subtle", Level::Subtle, Some(Cue::Changed(ch.clone())), &theme, &ts, marks));
    sc.push(run("changed_full", Level::Full, Some(Cue::Changed(ch)), &theme, &ts, marks));
    let ts2: Vec<u64> = (0..16).map(|i| i * 100).collect();
    sc.push(run("shimmer", Level::Full, None, &theme, &ts2, marks_loading));
    println!("{}", json!({"width": 30, "height": 8, "popup": [5,2,12,4], "message": [0,7,14,1], "colours": colours, "scenarios": sc}));
}
