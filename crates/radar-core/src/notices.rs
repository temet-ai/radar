//! Bounded session-only notice occurrences. No I/O, wall clock or persistence.
use std::collections::VecDeque;

use crate::plugin::Message;
use crate::view::chrome::{NoticeHistoryView, NoticeStateView, NoticeTone, NoticeView};

pub const HISTORY_LIMIT: usize = 256;
pub const TEXT_LIMIT: usize = 2048;
pub const SUMMARY_LIMIT: usize = 120;
pub const INFO_MS: u64 = 5_000;
pub const ERROR_MS: u64 = 10_000;

#[derive(Debug, Default)]
pub(crate) struct HistorySelection {
    pub selected: usize,
    pub scroll: usize,
    pub detail_scroll: usize,
}

#[derive(Debug, Default)]
pub struct Notices {
    now_ms: u64,
    next_id: u64,
    revision: u64,
    active: VecDeque<NoticeView>,
    history: VecDeque<NoticeView>,
    omitted: u64,
}

impl Notices {
    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn through_id(&self) -> u64 {
        self.next_id
    }
    pub fn len(&self) -> usize {
        self.history.len()
    }
    pub fn is_empty(&self) -> bool {
        self.history.is_empty()
    }
    pub fn has_active(&self) -> bool {
        !self.active.is_empty()
    }
    pub fn history_id(&self, index: usize) -> Option<u64> {
        self.history.iter().rev().nth(index).map(|notice| notice.id)
    }
    pub fn history_index(&self, id: u64) -> Option<usize> {
        self.history.iter().rev().position(|notice| notice.id == id)
    }
    pub fn active(&self) -> impl Iterator<Item = &NoticeView> {
        self.active.iter()
    }
    pub fn since(&self, id: u64) -> impl Iterator<Item = &NoticeView> {
        self.history.iter().filter(move |notice| notice.id > id)
    }
    pub fn enqueue(&mut self, message: Message) -> NoticeView {
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("notice occurrence IDs exhausted");
        let notice = NoticeView {
            id: self.next_id,
            tone: message.tone,
            icon: message.icon,
            text: bounded_text(&message.text, TEXT_LIMIT),
            created_ms: self.now_ms,
            lifetime_ms: match message.tone {
                NoticeTone::Info => INFO_MS,
                NoticeTone::Error => ERROR_MS,
            },
        };
        if self.history.len() == HISTORY_LIMIT {
            self.history.pop_front();
            self.omitted = self.omitted.saturating_add(1);
        }
        if self.active.len() == HISTORY_LIMIT {
            self.active.pop_front();
        }
        self.active.push_back(notice.clone());
        self.history.push_back(notice.clone());
        self.revision = self.revision.saturating_add(1);
        notice
    }
    /// Backwards injected samples cannot revive notices.
    pub fn advance_clock(&mut self, now_ms: u64) -> bool {
        self.now_ms = self.now_ms.max(now_ms);
        let before = self.active.len();
        self.active
            .retain(|n| self.now_ms.saturating_sub(n.created_ms) < n.lifetime_ms);
        self.changed(before)
    }
    pub fn input(&mut self, escape: bool) -> bool {
        self.input_through(escape, self.next_id)
    }
    pub fn input_through(&mut self, escape: bool, through_id: u64) -> bool {
        let before = self.active.len();
        self.active
            .retain(|n| n.id > through_id || (!escape && n.tone != NoticeTone::Error));
        self.changed(before)
    }
    pub fn dismiss(&mut self, id: u64) -> bool {
        let before = self.active.len();
        self.active.retain(|n| n.id != id);
        self.changed(before)
    }
    fn changed(&mut self, before: usize) -> bool {
        let changed = self.active.len() != before;
        if changed {
            self.revision = self.revision.saturating_add(1);
        }
        changed
    }
    pub fn view(&self) -> NoticeStateView {
        NoticeStateView {
            now_ms: if self.has_active() { self.now_ms } else { 0 },
            revision: self.revision,
            through_id: self.next_id,
            active: self
                .active
                .iter()
                .skip(self.active.len().saturating_sub(4))
                .cloned()
                .collect(),
            hidden: self.active.len().saturating_sub(4) as u64,
        }
    }
    pub fn history_view(
        &self,
        selected: usize,
        scroll: usize,
        detail_scroll: usize,
    ) -> NoticeHistoryView {
        let selected = selected.min(self.history.len().saturating_sub(1));
        NoticeHistoryView {
            entries: self
                .history
                .iter()
                .map(|notice| {
                    let mut summary = notice.clone();
                    summary.text = bounded_text(&summary.text, SUMMARY_LIMIT);
                    summary
                })
                .collect(),
            detail: self.history.iter().rev().nth(selected).cloned(),
            omitted: self.omitted,
            selected,
            scroll,
            detail_scroll,
        }
    }
}

fn bounded_text(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut bounded: String = chars
        .by_ref()
        .take(limit)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if chars.next().is_some() && limit > 0 {
        bounded.pop();
        bounded.push('…');
    }
    bounded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_text_bounds_show_truncation_and_neutralize_controls() {
        assert_eq!(bounded_text("abcd", 4), "abcd");
        assert_eq!(bounded_text("abcde", 4), "abc…");
        assert_eq!(bounded_text("🦉🦉🦉", 2), "🦉…");
        assert_eq!(bounded_text("a\n\t\u{1b}b", 8), "a   b");
    }

    #[test]
    fn bounded_summary_and_selected_detail_have_a_fixed_wire_byte_budget() {
        let mut notices = Notices::default();
        for text in ["🦉".repeat(TEXT_LIMIT + 1), "\\\"".repeat(TEXT_LIMIT)] {
            for _ in 0..HISTORY_LIMIT {
                notices.enqueue(text.clone().into());
            }
            assert!(
                serde_json::to_vec(&notices.history_view(0, 0, 0))
                    .unwrap()
                    .len()
                    < 170_000
            );
            assert!(serde_json::to_vec(&notices.view()).unwrap().len() < 34_000);
        }
    }

    #[test]
    fn a_key_watermark_cannot_dismiss_a_newer_unseen_error() {
        let mut notices = Notices::default();
        let seen = notices.enqueue(Message::error(None, "seen"));
        let unseen = notices.enqueue(Message::error(None, "unseen"));
        notices.input_through(true, seen.id);
        assert_eq!(notices.view().active, [unseen]);
    }

    #[test]
    fn occurrences_are_distinct_and_keep_order_even_for_identical_words() {
        let mut notices = Notices::default();
        let first = notices.enqueue("same".into());
        let second = notices.enqueue("same".into());
        assert_ne!(first.id, second.id);
        assert_eq!(notices.history_view(0, 0, 0).entries, [first, second]);
    }

    #[test]
    fn lifetimes_use_monotonic_injected_time_and_exact_boundaries() {
        let mut notices = Notices::default();
        notices.advance_clock(100);
        let info = notices.enqueue("info".into());
        let error = notices.enqueue(Message::error(None, "problem"));
        notices.advance_clock(5_099);
        assert_eq!(notices.view().active, [info, error.clone()]);
        notices.advance_clock(5_100);
        assert_eq!(notices.view().active, [error]);
        notices.advance_clock(0);
        assert_eq!(notices.now_ms(), 5_100);
        notices.advance_clock(10_100);
        assert!(!notices.has_active());
        assert_eq!(notices.len(), 2);
    }

    #[test]
    fn ordinary_keys_dismiss_errors_and_escape_preserves_history() {
        let mut notices = Notices::default();
        let info = notices.enqueue("info".into());
        notices.enqueue(Message::error(None, "problem"));
        assert!(notices.input(false));
        assert_eq!(notices.view().active, [info]);
        assert!(!notices.input(false));
        assert!(notices.input(true));
        assert_eq!(notices.len(), 2);
    }

    #[test]
    fn bounded_history_active_and_text_report_omissions() {
        let mut notices = Notices::default();
        for _ in 0..HISTORY_LIMIT + 7 {
            notices.enqueue("🦉".repeat(TEXT_LIMIT + 10).into());
        }
        let state = notices.history_view(0, 0, 0);
        assert_eq!(state.entries.len(), HISTORY_LIMIT);
        assert_eq!(notices.view().active.len(), 4);
        assert_eq!(notices.view().hidden, (HISTORY_LIMIT - 4) as u64);
        assert_eq!(state.omitted, 7);
        assert_eq!(state.entries[0].id, 8);
        assert_eq!(state.entries[0].text.chars().count(), SUMMARY_LIMIT);
        assert_eq!(state.detail.unwrap().text.chars().count(), TEXT_LIMIT);
    }

    #[test]
    fn stale_dismissal_cannot_affect_a_later_identical_occurrence() {
        let mut notices = Notices::default();
        let first = notices.enqueue("same".into());
        assert!(notices.dismiss(first.id));
        let second = notices.enqueue("same".into());
        assert!(!notices.dismiss(first.id));
        assert_eq!(notices.view().active, [second]);
    }
}
