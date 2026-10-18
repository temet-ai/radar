import type { Case } from "./cases-screen.js";
import { chromeOf } from "./support.js";
import type { NoticeView } from "../../src/protocol/generated/NoticeView.js";
const W = 160, H = 40;
const occurrence = (id = 1): NoticeView => ({ id, tone: "info", text: `City Watch report ${id}`, created_ms: 0, lifetime_ms: 5000 });
const screen = { id: "watch", scope: "watch", panes: [{ pane: "middle", title: "Watch", body: { node: "document", lines: [[{ text: "UNDERLYING" }]] } }], actions: [] };
const base = () => ({ now_ms: 2500, revision: 1, through_id: 1, active: [occurrence()], hidden: 0 });
const notices = (change: (state: any, on: boolean) => void): Case => (on) => { const state = base(); change(state, on); return { chrome: chromeOf(W, H, { notices: state, notices_visible: true }), screen }; };
const item = (change: (notice: any, on: boolean) => void): Case => notices((state, on) => change(state.active[0], on));
const historyBase = () => ({ entries: [occurrence(1), occurrence(2)], detail: occurrence(2), selected: 0, scroll: 0, detail_scroll: 0, omitted: 0 });
const history = (change: (view: any, on: boolean) => void): Case => (on) => { const view = historyBase(); change(view, on); return { chrome: chromeOf(W, H, { notice_history: view, notices_visible: false }), screen }; };
const immediate = (change: (message: any, on: boolean) => void): Case => (on) => { const message = { type: "notice", cursor: 1, occurrence: occurrence(), text: "Legacy Watch report", revision: 1, now_ms: 2500 }; change(message, on); return { chrome: chromeOf(W, H, { notices_visible: true }), screen, extra: [message] }; };
export const NOTICE_CASES: Record<string, Case> = {
  "ChromeView.notices": (on) => ({ chrome: chromeOf(W, H, { notices: on ? base() : undefined, notices_visible: true }), screen }),
  "ChromeView.notice_history": (on) => ({ chrome: chromeOf(W, H, { notice_history: on ? historyBase() : undefined }), screen }),
  "ChromeView.notices_visible": (on) => ({ chrome: chromeOf(W, H, { notices: base(), notices_visible: on }), screen }),
  "NoticeStateView.now_ms": notices((state, on) => { state.now_ms = on ? 2500 : 0; }),
  "NoticeStateView.active": notices((state, on) => { if (!on) state.active = []; }),
  "NoticeStateView.hidden": notices((state, on) => { state.hidden = on ? 7 : 0; }),
  "NoticeView.tone": item((notice, on) => { notice.tone = on ? "error" : "info"; }),
  "NoticeView.icon": item((notice, on) => { notice.icon = on ? "check" : "info"; }),
  "NoticeView.text": item((notice, on) => { notice.text = on ? "Updated City Watch report" : "Initial report"; }),
  "NoticeView.created_ms": item((notice, on) => { notice.created_ms = on ? 2000 : 0; }),
  "NoticeView.lifetime_ms": item((notice, on) => { notice.lifetime_ms = on ? 10000 : 5000; }),
  "NoticeHistoryView.entries": history((view, on) => { if (!on) view.entries = []; }),
  "NoticeHistoryView.detail": history((view, on) => { if (!on) view.detail = null; }),
  "NoticeHistoryView.omitted": history((view, on) => { view.omitted = on ? 42 : 0; }),
  "NoticeHistoryView.selected": history((view, on) => { view.selected = on ? 1 : 0; }),
  "NoticeHistoryView.scroll": history((view, on) => { view.entries = Array.from({ length: 60 }, (_, i) => occurrence(i + 1)); view.selected = 25; view.scroll = on ? 20 : 0; }),
  "NoticeHistoryView.detail_scroll": history((view, on) => { view.detail.text = Array.from({ length: 60 }, (_, i) => `Report line ${i}`).join("\n"); view.detail_scroll = on ? 20 : 0; }),
  "Notice.occurrence": immediate((message, on) => { if (!on) message.occurrence = undefined; }),
  "Notice.now_ms": immediate((message, on) => { message.now_ms = on ? 2500 : 0; }),
};
export const NOTICE_INVISIBLE: Record<string, string> = {
  "NoticeView.id": "stable occurrence identity for reconciliation and dismissal; input tests reject stale clicks",
  "NoticeStateView.revision": "authoritative reconciliation ordering; store tests reject stale and duplicate snapshots",
  "NoticeStateView.through_id": "occurrence watermark for immediate delivery and input; never a displayed count",
  "Notice.revision": "immediate delivery ordering; duplicate/stale reconciliation covered in notices/store",
};
