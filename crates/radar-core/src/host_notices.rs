//! Session-only notices and their inert history surface.
use super::*;

impl Host {
    pub fn notify(&mut self, message: crate::plugin::Message) {
        let selected_id = self
            .notice_history
            .as_ref()
            .and_then(|s| self.notices.history_id(s.selected));
        let notice = self.notices.enqueue(message);
        if let Some(selection) = &mut self.notice_history
            && let Some(id) = selected_id
        {
            match self.notices.history_index(id) {
                Some(index) => selection.selected = index,
                None => {
                    selection.selected = self.notices.len().saturating_sub(1);
                    selection.detail_scroll = 0;
                }
            }
        }
        self.message = Some(crate::plugin::Message {
            tone: notice.tone,
            icon: notice.icon,
            text: notice.text,
        });
    }
    pub fn notify_error(&mut self, text: impl Into<String>) {
        self.notify(crate::plugin::Message::error(None, text));
    }
    pub fn set_ui_clock_ms(&mut self, now_ms: u64) {
        if self.notices.advance_clock(now_ms) {
            self.sync_message_compat();
        }
    }
    pub fn notice_input(&mut self, escape: bool) {
        if self.notices.input(escape) {
            self.sync_message_compat();
            if !self.notices.has_active() {
                self.cue(Cue::MessageGone);
            }
        }
    }
    pub(crate) fn clear_message_compat(&mut self) {
        self.message = None;
    }
    pub(super) fn sync_message_compat(&mut self) {
        self.message = self
            .notices
            .active()
            .last()
            .map(|n| crate::plugin::Message {
                tone: n.tone,
                icon: n.icon,
                text: n.text.clone(),
            });
    }
    pub fn notices_visible(&self) -> bool {
        self.menu.is_none()
            && !self.capturing()
            && self.history.is_none()
            && self.notice_history.is_none()
            && self.setup.is_none()
            && self.tour.is_none()
            && !self.notice_terminal_owns_input()
            && self.screen().is_none_or(|(owner, screen)| {
                self.screen_view(owner, screen.id)
                    .is_none_or(|view| view.overlay.is_none())
            })
    }
    fn notice_terminal_owns_input(&self) -> bool {
        self.dock.visible
            && self.dock.focused
            && self.dock_has_place()
            && self
                .plugins
                .iter()
                .filter_map(|p| p.dock_tabs())
                .any(|(tabs, active)| {
                    tabs.iter().any(|tab| {
                        Some(tab.id.as_str()) == active.as_deref() && tab.terminal.is_some()
                    })
                })
    }
    pub(super) fn notice_history_can_open(&self) -> bool {
        self.menu.is_none()
            && !self.capturing()
            && !self.dock.focused
            && self.setup.is_none()
            && self.tour.is_none()
            && self.screen().is_none_or(|(owner, screen)| {
                self.screen_view(owner, screen.id)
                    .is_none_or(|view| view.overlay.is_none())
            })
    }
    pub(super) fn guard_notice_history_owner(&mut self) {
        if self.notice_history.is_some() && !self.notice_history_can_open() {
            self.notice_history = None;
            self.history = None;
            self.clear_notice_hits();
        }
    }
    fn clear_notice_hits(&mut self) {
        self.hits.borrow_mut().clear();
        *self.hovered.borrow_mut() = None;
    }
    pub(crate) fn reset_notice_input_support(&mut self) {
        self.notice_input_supported = false;
    }
    pub fn set_notice_motion_enabled(&mut self, enabled: bool) {
        self.notice_motion = enabled;
    }
    pub fn notice_motion_enabled(&self) -> bool {
        self.notice_motion
    }
    pub fn notice_frame_needed(&self, area: ratatui::layout::Rect) -> bool {
        let has_sources = matches!(self.view, View::Screen(i) if self.group_screens(i).len() > 1);
        self.notice_motion
            && self.notices.has_active()
            && self.notices_visible()
            && !crate::shell::notice_content(area, has_sources).is_empty()
    }
    pub(super) fn open_notice_history(&mut self) {
        self.notice_history = Some(Default::default());
        self.clear_notice_hits();
    }
    pub(super) fn select_notice_history(&mut self, index: usize) {
        if let Some(selection) = &mut self.notice_history {
            let next = index.min(self.notices.len().saturating_sub(1));
            if next != selection.selected {
                selection.selected = next;
                selection.detail_scroll = 0;
            }
        }
    }
    pub(super) fn notice_history_key(&mut self, key: Chord) -> Vec<HostEffect> {
        if key.ctrl && key.key == Key::Char('c') {
            self.quit = true;
            return vec![];
        }
        match key.key {
            Key::Esc | Key::Char('~') => {
                self.notice_history = None;
                self.clear_notice_hits();
            }
            Key::Char('H') => return self.open_recent_history(),
            Key::Down | Key::Char('j') => {
                if let Some(s) = &self.notice_history {
                    self.select_notice_history(s.selected.saturating_add(1));
                }
            }
            Key::Up | Key::Char('k') => {
                if let Some(s) = &self.notice_history {
                    self.select_notice_history(s.selected.saturating_sub(1));
                }
            }
            Key::Home | Key::Char('g') => self.select_notice_history(0),
            Key::End | Key::Char('G') => {
                self.select_notice_history(self.notices.len().saturating_sub(1))
            }
            Key::PageDown => {
                if let Some(s) = &mut self.notice_history {
                    s.detail_scroll = s
                        .detail_scroll
                        .saturating_add(5)
                        .min(crate::notices::TEXT_LIMIT);
                }
            }
            Key::PageUp => {
                if let Some(s) = &mut self.notice_history {
                    s.detail_scroll = s.detail_scroll.saturating_sub(5);
                }
            }
            _ => {}
        }
        vec![]
    }
    pub(super) fn notice_history_target(
        &mut self,
        target: &crate::view::Target,
        activate: bool,
    ) -> Vec<HostEffect> {
        match self.chrome_hit(target) {
            Some(Hit::NoticeHistoryKey(key)) if activate => {
                // Ink already sent its captured watermark. Never clear unseen newer errors here.
                if key.key == Key::Esc && !self.notice_input_supported {
                    self.notice_input(true);
                }
                return self.notice_history_key(key);
            }
            Some(Hit::Tab(_) | Hit::HistoryTab) if activate => {
                self.notice_history = None;
                self.history = None;
                self.clear_notice_hits();
                return self.chrome_intent(target, true);
            }
            _ => {}
        }
        let Some(rest) = target.as_str().strip_prefix("chrome:notices:") else {
            return vec![];
        };
        if let Some(id) = rest.strip_prefix("id:").and_then(|s| s.parse().ok())
            && let Some(index) = self.notices.history_index(id)
        {
            self.select_notice_history(index);
        }
        vec![]
    }
    pub(crate) fn notice_history_key_target(key: Chord) -> Option<crate::view::Target> {
        let name = match key.key {
            Key::Esc => "close",
            Key::Down | Key::Char('j') => "older",
            Key::Up | Key::Char('k') => "newer",
            Key::PageUp => "details_up",
            Key::PageDown => "details_down",
            _ => return None,
        };
        Some(crate::view::Target::new(format!("chrome:notices:{name}")))
    }
    pub(super) fn notice_history_target_key(name: &str) -> Option<Chord> {
        let code = match name {
            "close" => Key::Esc,
            "older" => Key::Down,
            "newer" => Key::Up,
            "details_up" => Key::PageUp,
            "details_down" => Key::PageDown,
            _ => return None,
        };
        Some(Chord::new(code))
    }
    /// Presentation-only input bypasses the action journal. Physical keys keep their ordinary route.
    pub(super) fn update_notices(&mut self, event: &Event) -> Option<Vec<HostEffect>> {
        use crate::view::Intent;
        let mut input = event;
        while let Event::Tracked { event, .. } = input {
            input = event;
        }
        match input {
            Event::Intent(Intent::NoticeInput { escape, through_id }) => {
                self.notices.input_through(*escape, *through_id);
                self.notice_input_supported = true;
                self.sync_message_compat();
                return Some(vec![]);
            }
            Event::Intent(Intent::DismissNotice { id }) => {
                if self.notices_visible() {
                    self.notices.dismiss(*id);
                }
                self.sync_message_compat();
                return Some(vec![]);
            }
            Event::Intent(Intent::Key { chord }) if !self.notice_input_supported => {
                self.notice_input(chord.key == crate::view::Key::Esc)
            }
            _ => {}
        }
        let opens = match input {
            Event::Intent(Intent::Key { chord }) => {
                chord.key == crate::view::Key::Char('~') && !chord.ctrl && !chord.alt
            }
            _ => false,
        };
        if opens && self.notice_history.is_none() && self.notice_history_can_open() {
            self.open_notice_history();
            return Some(vec![]);
        }
        self.notice_history.as_ref()?;
        match input {
            Event::Intent(Intent::Key { chord }) => {
                Some(self.notice_history_key(*chord))
            }
            Event::Intent(intent @ (Intent::Activate { target } | Intent::Select { target })) => {
                Some(self.notice_history_target(target, matches!(intent, Intent::Activate { .. })))
            }
            Event::Mouse {
                kind, column, row, ..
            } => {
                if *kind == MouseKind::Press {
                    match self.hit_at(*column, *row) {
                        Some(Hit::NoticeHistory(id)) => {
                            if let Some(index) = self.notices.history_index(id) {
                                self.select_notice_history(index);
                            }
                        }
                        Some(Hit::NoticeHistoryKey(key)) => {
                            if key.key == Key::Esc {
                                self.notice_input(true);
                            }
                            return Some(self.notice_history_key(key));
                        }
                        Some(hit @ (Hit::Tab(_) | Hit::HistoryTab)) => {
                            if let Some(target) = self.chrome_target_of(&hit) {
                                return Some(self.notice_history_target(&target, true));
                            }
                        }
                        _ => {}
                    }
                } else if matches!(kind, MouseKind::ScrollUp | MouseKind::ScrollDown) {
                    return Some(self.notice_history_key(Chord::new(if *kind == MouseKind::ScrollUp {
                            Key::Up
                        } else {
                            Key::Down
                        })));
                }
                Some(vec![])
            }
            Event::Intent(Intent::Copy { .. }) => None,
            Event::Intent(_) | Event::Paste(_) => Some(vec![]),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{Chord, Intent, Key};

    fn probe() -> (
        Host,
        std::sync::Arc<std::sync::Mutex<crate::host::tests::Log>>,
    ) {
        let (plugin, log) = crate::host::tests::probe("watch", 'W', "watch");
        (Host::new(vec![plugin], 0).unwrap(), log)
    }
    fn press(host: &mut Host, code: Key) -> Vec<HostEffect> {
        host.update(crate::host::chord_event(Chord::new(code)))
    }
    #[test]
    fn every_effect_occurrence_is_retained_without_recording_raw_text() {
        let (mut host, _) = probe();
        host.enable_journal();
        let secret = "private notice payload";
        let effects = host.route(
            0,
            vec![
                Effect::Notify(secret.into()),
                Effect::Notify(secret.into()),
                Effect::error(None, secret),
            ],
            0,
        );
        assert_eq!(host.notices.len(), 3);
        assert_eq!(
            host.notices.since(0).map(|n| n.id).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(
            host.notices.view().active[2].tone,
            crate::view::chrome::NoticeTone::Error
        );
        assert!(!format!("{effects:?}").contains(secret));
        assert!(!host.config_table().to_string().contains(secret));
    }
    #[test]
    fn notice_history_is_session_only_and_h_keeps_durable_history() {
        let (mut host, _) = probe();
        host.notify("first".into());
        host.notify("second".into());
        assert!(press(&mut host, Key::Char('~')).is_empty());
        let chrome = host.chrome_view(80, 24);
        assert_eq!(chrome.notice_history.unwrap().entries.len(), 2);
        assert!(chrome.history.is_none());
        assert!(!chrome.notices_visible);
        press(&mut host, Key::Esc);
        assert!(host.notice_history.is_none());
        assert!(!host.notices.has_active());
        press(&mut host, Key::Char('~'));
        assert_eq!(host.notices.len(), 2);
        assert!(
            press(&mut host, Key::Char('H'))
                .iter()
                .any(|e| matches!(e, HostEffect::History(_)))
        );
        assert!(host.notice_history.is_none());
        assert!(host.history.is_some());
    }
    #[test]
    fn history_status_clicks_and_topbar_navigation_match_their_keys() {
        let (mut host, log) = probe();
        host.notify("notice details".repeat(20).into());
        press(&mut host, Key::Char('~'));
        let page = host
            .chrome_view(80, 24)
            .status
            .hints
            .into_iter()
            .flat_map(|h| h.keys_of)
            .find(|k| k.chord.key == Key::PageDown)
            .unwrap()
            .target;
        host.update(Event::Intent(Intent::Activate { target: page }));
        assert_eq!(host.notice_history.as_ref().unwrap().detail_scroll, 5);
        host.hits.borrow_mut().push((
            ratatui::layout::Rect::new(1, 1, 5, 1),
            Hit::NoticeHistoryKey(Chord::new(Key::Esc)),
        ));
        host.update(Event::Mouse {
            kind: MouseKind::Press,
            column: 1,
            row: 1,
            at_ms: 0,
        });
        assert!(host.notice_history.is_none());
        assert!(!host.notices.has_active());
        press(&mut host, Key::Char('~'));
        host.update(Event::Intent(Intent::Activate {
            target: crate::view::chrome::targets::settings_tab(),
        }));
        assert!(host.notice_history.is_none());
        assert_eq!(host.view, View::Settings);
        assert!(log.lock().unwrap().keys.is_empty());
    }
    #[test]
    fn history_escape_preserves_newer_unseen_errors_for_keys_and_clicks() {
        for click in [false, true] {
            let (mut host, _) = probe();
            host.notify_error("seen error");
            press(&mut host, Key::Char('~'));
            let through_id = host.notices.through_id();
            host.notify_error("newer unseen error");
            let unseen_id = host.notices.through_id();
            host.update(Event::Intent(Intent::NoticeInput {
                escape: true,
                through_id,
            }));
            let intent = if click {
                Intent::Activate {
                    target: crate::view::Target::new("chrome:notices:close"),
                }
            } else {
                Intent::Key {
                    chord: Chord::new(Key::Esc),
                }
            };
            host.update(Event::Intent(intent));
            assert!(host.notice_history.is_none());
            assert_eq!(
                host.notices
                    .view()
                    .active
                    .iter()
                    .map(|n| n.id)
                    .collect::<Vec<_>>(),
                [unseen_id]
            );
        }
    }
    #[test]
    fn history_selection_stays_on_its_occurrence_until_evicted() {
        let (mut host, _) = probe();
        host.notify("reading this notice".into());
        press(&mut host, Key::Char('~'));
        host.notice_history.as_mut().unwrap().detail_scroll = 5;
        host.notify("new arrival".into());
        let selected = host.notice_history.as_ref().unwrap();
        assert_eq!((selected.selected, selected.detail_scroll), (1, 5));
        assert_eq!(host.notices.history_id(selected.selected), Some(1));
        for _ in 0..crate::notices::HISTORY_LIMIT {
            host.notify("later arrival".into());
        }
        let selected = host.notice_history.as_ref().unwrap();
        assert_eq!(
            (selected.selected, selected.detail_scroll),
            (crate::notices::HISTORY_LIMIT - 1, 0)
        );
        assert_eq!(host.notices.history_id(selected.selected), Some(3));
    }
    #[test]
    fn notice_route_respects_editor_menu_and_dock_ownership() {
        let (mut host, log) = probe();
        log.lock().unwrap().typing = true;
        press(&mut host, Key::Char('~'));
        assert!(host.notice_history.is_none());
        log.lock().unwrap().typing = false;
        log.lock().unwrap().overlay = true;
        press(&mut host, Key::Char('~'));
        assert!(host.notice_history.is_none());
        log.lock().unwrap().overlay = false;
        host.menu = Some(OpenMenu::open(
            Owner::Host,
            Menu::confirm("confirm", "proceed?", "yes", "no"),
        ));
        press(&mut host, Key::Char('~'));
        assert!(host.notice_history.is_none());
        host.menu = None;
        host.dock.focused = true;
        press(&mut host, Key::Char('~'));
        assert!(host.notice_history.is_none());
    }
    #[test]
    fn stale_history_hint_targets_never_choose_a_replacement_confirmation() {
        let (mut host, log) = probe();
        host.notify("notice".into());
        press(&mut host, Key::Char('~'));
        let targets: Vec<_> = host
            .chrome_view(80, 24)
            .status
            .hints
            .into_iter()
            .flat_map(|h| h.keys_of)
            .map(|k| k.target)
            .collect();
        assert!(
            targets
                .iter()
                .all(|t| t.as_str().starts_with("chrome:notices:"))
        );
        assert!(
            targets
                .iter()
                .any(|t| t.as_str() == "chrome:notices:details_up")
        );
        host.update(Event::Msg {
            plugin: 0,
            msg: Box::new("question"),
        });
        assert!(host.notice_history.is_none());
        for target in targets {
            host.update(Event::Intent(Intent::Activate { target }));
            assert!(
                host.menu
                    .as_ref()
                    .is_some_and(|open| open.menu.id == "background-question")
            );
            assert!(log.lock().unwrap().bus.is_empty());
        }
    }
    #[test]
    fn background_questions_and_editors_replace_inert_notice_history() {
        let (mut host, log) = probe();
        host.notify("notice".into());
        press(&mut host, Key::Char('~'));
        host.update(Event::Msg {
            plugin: 0,
            msg: Box::new("question"),
        });
        assert!(host.notice_history.is_none());
        assert!(host.menu.as_ref().is_some_and(|open| open.menu.question));
        press(&mut host, Key::Esc);
        assert!(host.menu.is_none());
        press(&mut host, Key::Char('~'));
        log.lock().unwrap().typing = true;
        host.update(Event::Tick(1));
        assert!(host.notice_history.is_none());
        assert!(host.capturing());
    }
    #[test]
    fn key_dismissal_preserves_history_and_dispatches_the_key_once() {
        let (mut host, log) = probe();
        host.notify_error("problem");
        host.notify("information".into());
        let through_id = host.notices.through_id();
        host.update(Event::Intent(Intent::NoticeInput {
            escape: false,
            through_id,
        }));
        host.update(Event::Intent(Intent::Key {
            chord: Chord::new(Key::Char('a')),
        }));
        assert_eq!(log.lock().unwrap().keys, ['a']);
        assert_eq!(host.notices.view().active.len(), 1);
        assert_eq!(host.notices.len(), 2);
    }
    #[test]
    fn covered_notice_click_dismisses_only_its_id_and_never_the_underlying_row() {
        let (mut host, log) = probe();
        host.notify("information".into());
        let id = host.notices.through_id();
        let area = ratatui::layout::Rect::new(2, 2, 20, 1);
        host.hits.borrow_mut().push((
            area,
            Hit::Key(Chord::new(Key::Char('a')), 0),
        ));
        host.hits.borrow_mut().push((area, Hit::Notice(id)));
        host.update(Event::Mouse {
            kind: MouseKind::Press,
            column: 3,
            row: 2,
            at_ms: 0,
        });
        assert!(!host.notices.has_active());
        assert!(log.lock().unwrap().keys.is_empty());
        host.notify("information".into());
        host.update(Event::Intent(Intent::DismissNotice { id }));
        assert!(host.notices.has_active());
        assert_eq!(host.notices.len(), 2);
    }
    #[test]
    fn suppressed_notices_expire_and_no_notice_frame_timer_survives() {
        let (mut host, _) = probe();
        host.set_notice_motion_enabled(true);
        host.notify("information".into());
        let area = ratatui::layout::Rect::new(0, 0, 80, 24);
        assert!(host.notice_frame_needed(area));
        for (width, height) in [(0, 0), (2, 4), (80, 4), (2, 24)] {
            assert!(!host.notice_frame_needed(ratatui::layout::Rect::new(0, 0, width, height)));
        }
        assert!(host.notice_frame_needed(ratatui::layout::Rect::new(0, 0, 3, 5)));
        host.set_notice_motion_enabled(false);
        assert!(!host.notice_frame_needed(area));
        host.set_notice_motion_enabled(true);
        host.menu = Some(OpenMenu::open(
            Owner::Host,
            Menu::confirm("question", "proceed?", "yes", "no"),
        ));
        assert!(!host.notice_frame_needed(area));
        assert!(!host.notices_visible());
        host.set_ui_clock_ms(crate::notices::INFO_MS);
        host.menu = None;
        assert!(!host.notices.has_active());
        assert!(!host.notice_frame_needed(area));
        assert_eq!(host.notices.len(), 1);
    }
}
