import type { ChromeView } from "../../src/chrome/types.js";

/** A chrome like `radar shot` shows on the Books screen: three tabs, sources, hints, a mode. */
export function booksChrome(over: Partial<ChromeView> = {}): ChromeView {
  return {
    version: 1,
    width: 80,
    height: 24,
    top_bar: {
      brand: "radar",
      tabs: [
        { label: "Books", mnemonic: "B", badge: { text: "3", ink: "accent" }, active: true, target: "chrome:tab:0" },
        { label: "Work", mnemonic: "W", target: "chrome:tab:1" },
        { label: "history", mnemonic: "h", target: "chrome:history" },
        { label: "settings", mnemonic: "s", target: "chrome:settings" },
      ],
      words: { text: "synced 12s", ink: "muted" },
      mode: { text: "read-only", ink: "warn", target: "chrome:mode" },
    },
    sources: [
      { label: "GitLab", active: true, target: "chrome:source:0" },
      { label: "GitHub", target: "chrome:source:1" },
    ],
    status: {
      scope: "BOOKS",
      hints: [
        { keys: "PgUp/PgDn", label: "scroll", keys_of: [] },
        { keys: "q", label: "quit", keys_of: [{ label: "q", chord: { key: { char: "q" } }, target: "chrome:key:1:0" }] },
        { keys: "x", label: "menu", keys_of: [{ label: "x", chord: { key: { char: "x" } }, action: "chrome.menu", target: "chrome:key:2:0" }] },
        { keys: "←→↑↓", label: "navigate", keys_of: [] },
      ],
      least_needed: ["navigate", "scroll"],
      counts: [
        { text: "11 open", ink: "muted" },
        { text: "3 need you", ink: "warn" },
      ],
    },
    keys_at: "body",
    ...over,
  };
}

export const actionsMenu = {
  id: "actions",
  title: "!263 fix the thing",
  question: false,
  items: [
    { value: "head", label: "!263", hint: "", heading: true },
    { value: "gitlab.mr.open", label: "open", hint: "in the browser", key: "o", target: "chrome:menu:1" },
    { value: "gitlab.mr.merge", label: "merge", hint: "when green", key: "m", target: "chrome:menu:2" },
  ],
  selected: 1,
};

export const confirm = {
  id: "confirm",
  title: "post comment?",
  question: true,
  change: "add" as const,
  items: [
    { value: "", label: "This posts your comment to !263.", hint: "", note: true },
    { value: "yes", label: "post it", hint: "", key: "y", target: "chrome:menu:0" },
    { value: "no", label: "drop it", hint: "", key: "n", target: "chrome:menu:1" },
  ],
  selected: 1,
};

export const settingsNav = {
  groups: [
    {
      id: "gitlab",
      name: "GitLab",
      open: true,
      target: "chrome:group:gitlab",
      sections: [
        { id: "connection", name: "connection", target: "chrome:section:0" },
        { id: "projects", name: "projects", target: "chrome:section:1" },
      ],
    },
    { id: "slack", name: "Slack", open: false, target: "chrome:group:slack", sections: [] },
  ],
  selected: "chrome:section:1",
  section_title: "projects",
  detail_title: "detail",
};
