//! The dock's pipe to the renderer under a long transcript: what one
//! streamed word costs the host and the line it sends, with 500 items in
//! the transcript against 20. The budget is a streamed word under 50 ms
//! to the frame; the bytes are exact, the time is a ceiling far over what a
//! word costs, so a busy machine does not fail it but O(transcript) does.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::*;
use crate::dock::{DockItem, DockView};
use crate::host::{Event, Host};
use crate::plugin::{Plugin, ScreenSpec};
use crate::view::{ListRow, Node, PaneId, PaneView, ScreenView};

/// A part whose dock is whatever the test put in `view`.
struct Streamer {
    view: Arc<Mutex<DockView>>,
}

impl Plugin for Streamer {
    fn id(&self) -> &'static str {
        "streamer"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec { id: "main", name: "Main", key: 'M' }]
    }
    fn view(&self, scope: &str, _: &crate::view::ViewCx<'_>) -> Option<ScreenView> {
        let rows = (0..30).map(|i| ListRow::new(format!("row{i}"), format!("row number {i}"))).collect();
        Some(ScreenView::new(scope, vec![PaneView::new(PaneId::Middle, "rows", Node::List { rows, selected: Some(0), editing: None })]))
    }
    fn dock(&self) -> Option<DockView> {
        Some(self.view.lock().unwrap().clone())
    }
}

const WORDS: &str = "the change moves the transcript out of the hot path so a streamed word costs the same whatever the \
    session holds and the renderer keeps what it already wrapped and draws only the rows that show";

/// A transcript of `n` items: users, answers a paragraph long, tool calls.
fn transcript(n: usize) -> Vec<DockItem> {
    (0..n)
        .map(|i| match i % 3 {
            0 => DockItem::User { id: format!("user-{i}"), text: format!("question {i}: {WORDS}"), queued: None },
            1 => DockItem::assistant(format!("m{i}"), format!("answer {i}: {WORDS} {WORDS}"), true),
            _ => DockItem::Tool {
                id: format!("call{i}"),
                title: format!("ls -la src/{i}"),
                kind: "shell".into(),
                state: crate::dock::ToolState::Done,
                open: false,
                result: vec!["a".into(), "b".into()],
                diff: Vec::new(),
            },
        })
        .collect()
}

struct Run {
    /// The longest snapshot line sent for one streamed word.
    most_bytes: usize,
    /// The host's time for one word: the event, the snapshot, its line.
    per_word: Duration,
}

/// Stream `words` words into an answer after `items` items, one snapshot
/// each, as the runner does: an event, then what goes out, encoded, then
/// the renderer's ack.
fn stream(items: usize, words: usize) -> Run {
    let mut view = DockView { visible: true, active: Some("s1".into()), transcript: transcript(items), ..DockView::default() };
    view.tabs.push(crate::dock::DockTab {
        id: "s1".into(),
        provider: "fake".into(),
        icon: None,
        title: "one".into(),
        state: crate::dock::SessionState::Streaming,
        unread: false,
        running: Some(0),
        terminal: None,
    });
    view.transcript.push(DockItem::assistant("live", String::new(), false));
    let shared = Arc::new(Mutex::new(view));
    let host = Host::new(vec![Box::new(Streamer { view: shared.clone() })], 0).unwrap();
    let mut bridge = Bridge::new(host, 160, 40);
    bridge.receive(ToHost::Hello {
        protocol: PROTOCOL,
        view: crate::view::VERSION,
        capabilities: Capabilities { columns: 160, rows: 40, mouse: true, true_colour: true, image: ImageProtocol::None, plain_text: false },
    });
    let pump = |bridge: &mut Bridge| -> usize {
        let mut most = 0;
        for m in bridge.outgoing() {
            let line = encode(&m);
            if let ToRenderer::Snapshot(s) = &m {
                most = most.max(line.len());
                bridge.receive(ToHost::Ack { cursor: s.cursor });
            }
        }
        most
    };
    pump(&mut bridge);
    let mut most_bytes = 0;
    let started = Instant::now();
    for w in 0..words {
        if let Some(DockItem::Assistant { text, .. }) = shared.lock().unwrap().transcript.last_mut() {
            text.push_str(WORDS.split(' ').nth(w % 30).unwrap_or("word"));
            text.push(' ');
        }
        bridge.update(Event::Tick(0));
        most_bytes = most_bytes.max(pump(&mut bridge));
    }
    Run { most_bytes, per_word: started.elapsed() / words as u32 }
}

#[test]
fn a_streamed_word_costs_the_same_with_500_items_as_with_20() {
    let short = stream(20, 60);
    let long = stream(500, 60);
    eprintln!(
        "per streamed word: 20 items {:?}, {} bytes; 500 items {:?}, {} bytes",
        short.per_word, short.most_bytes, long.per_word, long.most_bytes
    );
    // The line carries what changed, not the transcript: 480 more items add
    // nothing to it.
    assert!(
        long.most_bytes <= short.most_bytes + 512,
        "a word with 500 items sent {} bytes, with 20 {}",
        long.most_bytes,
        short.most_bytes
    );
    // A debug build's ceiling, many times what a word costs: the 16 ms of a
    // frame.
    assert!(long.per_word < Duration::from_millis(16), "a word with 500 items took {:?}", long.per_word);
}

/// What Ink does with a dock it is sent (ink/src/dock/transcript.ts): the
/// items kept from the transcript it holds, then the tail.
#[derive(Default)]
struct Renderer {
    held: Option<(u64, Vec<DockItem>)>,
}

impl Renderer {
    /// The whole transcript the snapshot shows, or `Err` when it builds on
    /// a revision this renderer does not hold (it asks for a resync).
    fn take(&mut self, dock: &DockView) -> Result<Vec<DockItem>, ()> {
        let items = match dock.kept {
            None => dock.transcript.clone(),
            Some(k) => match &self.held {
                Some((rev, items)) if *rev == k.revision && k.items as usize <= items.len() => {
                    let mut all = items[..k.items as usize].to_vec();
                    all.extend(dock.transcript.iter().cloned());
                    all
                }
                _ => {
                    self.held = None;
                    return Err(());
                }
            },
        };
        self.held = Some((dock.revision, items.clone()));
        Ok(items)
    }
}

fn hello() -> ToHost {
    ToHost::Hello {
        protocol: PROTOCOL,
        view: crate::view::VERSION,
        capabilities: Capabilities { columns: 80, rows: 24, mouse: true, true_colour: true, image: ImageProtocol::None, plain_text: false },
    }
}

struct Rig {
    shared: Arc<Mutex<DockView>>,
    bridge: Bridge,
    ink: Renderer,
}

impl Rig {
    fn new(items: usize) -> Self {
        let view = DockView { visible: true, active: Some("s1".into()), transcript: transcript(items), ..DockView::default() };
        let shared = Arc::new(Mutex::new(view));
        let host = Host::new(vec![Box::new(Streamer { view: shared.clone() })], 0).unwrap();
        let mut bridge = Bridge::new(host, 80, 24);
        bridge.receive(hello());
        Self { shared, bridge, ink: Renderer::default() }
    }

    /// Change the part's dock, then send what goes out, as Ink takes it in:
    /// the transcript Ink draws, if a snapshot went.
    fn step(&mut self, change: impl FnOnce(&mut DockView)) -> Option<Vec<DockItem>> {
        change(&mut self.shared.lock().unwrap());
        self.bridge.update(Event::Tick(0));
        self.pump()
    }

    fn pump(&mut self) -> Option<Vec<DockItem>> {
        let mut drawn = None;
        for m in self.bridge.outgoing() {
            // Over the wire and back, as Ink reads it.
            let m: ToRenderer = serde_json::from_slice(&encode(&m)).unwrap();
            if let ToRenderer::Snapshot(s) = m {
                self.bridge.receive(ToHost::Ack { cursor: s.cursor });
                let dock = s.screen.as_ref().and_then(|v| v.dock.as_deref()).expect("a dock");
                match self.ink.take(dock) {
                    Ok(items) => drawn = Some(items),
                    Err(()) => {
                        self.bridge.receive(ToHost::Resync);
                    }
                }
            }
        }
        drawn
    }

    fn truth(&self) -> Vec<DockItem> {
        self.shared.lock().unwrap().transcript.clone()
    }
}

fn last_text(d: &mut DockView) -> &mut String {
    match d.transcript.last_mut() {
        Some(DockItem::Assistant { text, .. } | DockItem::User { text, .. }) => text,
        _ => panic!("the foot is text"),
    }
}

/// Every change the dock's transcript sees reaches the renderer whole:
/// words streamed into the last item, items added, an earlier item changed
/// (a fold opened), another tab, items gone, a renderer started again, and
/// a renderer that lost what it held.
#[test]
fn the_renderer_rebuilds_the_hosts_transcript_from_what_changed() {
    let mut rig = Rig::new(31);
    assert_eq!(rig.pump().as_deref(), Some(rig.truth().as_slice()), "the first is whole");

    for word in ["one ", "two ", "three "] {
        let drawn = rig.step(|d| last_text(d).push_str(word));
        assert_eq!(drawn.as_deref(), Some(rig.truth().as_slice()), "a word streamed");
    }
    assert_eq!(rig.step(|_| {}), None, "nothing changed, nothing sent");

    let drawn = rig.step(|d| d.transcript.push(DockItem::Notice { id: "n".into(), text: "resumed".into() }));
    assert_eq!(drawn.as_deref(), Some(rig.truth().as_slice()), "an item added");

    let drawn = rig.step(|d| {
        if let DockItem::Tool { open, .. } = &mut d.transcript[2] {
            *open = true;
        }
    });
    assert_eq!(drawn.as_deref(), Some(rig.truth().as_slice()), "an early item changed");

    let drawn = rig.step(|d| {
        d.active = Some("s2".into());
        d.transcript = transcript(5);
    });
    assert_eq!(drawn.as_deref(), Some(rig.truth().as_slice()), "another tab");

    let drawn = rig.step(|d| d.transcript.truncate(1));
    assert_eq!(drawn.as_deref(), Some(rig.truth().as_slice()), "items gone");

    // A restarted renderer holds nothing: it is told everything again.
    rig.bridge.reconnect();
    rig.ink = Renderer::default();
    rig.bridge.receive(hello());
    assert_eq!(rig.pump().as_deref(), Some(rig.truth().as_slice()), "whole after a restart");

    // A renderer that lost its transcript asks again and is sent it whole.
    rig.ink.held = Some((999, vec![]));
    assert_eq!(rig.step(|d| last_text(d).push_str(" more")), None, "it cannot build on what it lost");
    assert_eq!(rig.pump().as_deref(), Some(rig.truth().as_slice()), "whole after a resync");
}

/// RADAR_TRACE_LATENCY=1: the snapshot where a message sent from the composer
/// first shows in the transcript says so, with the send's intent, so Ink
/// times Enter to the message painted (the send itself may show nothing).
#[test]
fn a_traced_send_is_timed_to_the_snapshot_that_shows_the_message() {
    let mut rig = Rig::new(3);
    rig.bridge.set_trace(true);
    rig.pump();
    let send = crate::view::Intent::Dock(crate::dock::DockIntent::Send { text: "hi".into() });
    rig.bridge.receive(ToHost::Intent { seq: 5, cursor: 0, intent: send });
    rig.pump();
    rig.shared.lock().unwrap().transcript.push(DockItem::User { id: "user-3".into(), text: "hi".into(), queued: None });
    rig.bridge.update(Event::Tick(0));
    let out = rig.bridge.outgoing();
    let trace = out
        .iter()
        .find_map(|m| match m {
            ToRenderer::Snapshot(s) => s.trace.clone(),
            _ => None,
        })
        .expect("a traced snapshot");
    assert_eq!((trace.cause.as_str(), trace.seq), ("message shown (intent dock send)", Some(5)));
}

/// An item added after a streamed word sends that item and nothing before it.
#[test]
fn a_change_at_the_foot_sends_only_the_items_it_changed() {
    let mut rig = Rig::new(100);
    rig.pump();
    rig.shared.lock().unwrap().transcript.push(DockItem::Notice { id: "x".into(), text: "x".into() });
    rig.bridge.update(Event::Tick(0));
    let out = rig.bridge.outgoing();
    let dock = out
        .iter()
        .find_map(|m| match m {
            ToRenderer::Snapshot(s) => s.screen.as_ref().and_then(|v| v.dock.clone()),
            _ => None,
        })
        .expect("a snapshot");
    assert_eq!(dock.kept.map(|k| k.items), Some(100), "the 100 items before it are kept");
    assert_eq!(dock.transcript.len(), 1);
}
