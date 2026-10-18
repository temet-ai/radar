import type { NoticeView } from "../../src/protocol/generated/NoticeView.js";
export const notice = (id: number, over: Partial<NoticeView> = {}): NoticeView => ({ id, tone: "info", text: "Watch updated", created_ms: 0, lifetime_ms: 5000, ...over });
/** Host::status_hints while history owns input, using stable notice-specific targets. */
export const historyStatus = {
  scope: "WATCH", least_needed: ["navigate", "scroll", "quit", "expand/collapse"], counts: [], health: [],
  hints: [
    { keys: "j/k", label: "move", keys_of: [
      { label: "j", chord: { key: { char: "j" } }, target: "chrome:notices:older" },
      { label: "k", chord: { key: { char: "k" } }, target: "chrome:notices:newer" },
    ] },
    { keys: "PgUp/PgDn", label: "details", keys_of: [
      { label: "PgUp", chord: { key: "page_up" as const }, target: "chrome:notices:details_up" },
      { label: "PgDn", chord: { key: "page_down" as const }, target: "chrome:notices:details_down" },
    ] },
    { keys: "esc", label: "back", keys_of: [{ label: "esc", chord: { key: "esc" as const }, target: "chrome:notices:close" }] },
  ],
};
