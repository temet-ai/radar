// GENERATED from crates/radar-core (ui/icon.rs `icons!`, design/theme/tokens.rs, and the parts' TENANT_SLOTS / SITE_SLOTS /
// CHANNEL_SLOTS) by a one-off script; the data is Rust's, never chosen in Ink. These are the defaults (truecolor,
// unicode pack) used until the host sends its resolved look (look.ts `applyLook`).
export const SLOTS: Readonly<Record<string, string>> = {
  "bg.base": "#0a0812",
  "bg.surface": "#12101a",
  "bg.overlay": "#1a162a",
  "bg.selection": "#44475a",
  "bg.hover": "#2a263e",
  "text.primary": "#f8f8f2",
  "text.secondary": "#e9d5ff",
  "text.muted": "#6272a4",
  "text.emphasis": "#ffffff",
  "accent.primary": "#e135ff",
  "accent.muted": "#9580ff",
  "accent.secondary": "#80ffea",
  "accent.tertiary": "#ff6ac1",
  "status.success": "#50fa7b",
  "status.warning": "#f1fa8c",
  "status.error": "#ff6363",
  "status.info": "#82aaff",
  "data.number": "#ffb86c",
  "diff.added": "#50fa7b",
  "diff.removed": "#ff6363",
  "glow.purple": "#e135ff",
  "glow.pink": "#ff00ff",
  "glow.cyan": "#80ffea",
  "tenant.cyan": "#80ffea",
  "tenant.green": "#50fa7b",
  "tenant.magenta": "#ff6ac1",
  "tenant.purple": "#e135ff",
  "tenant.blue": "#82aaff",
  "tenant.yellow": "#f1fa8c",
  "tenant.orange": "#ffb86c",
  "jira.blue": "#82aaff",
  "jira.cyan": "#80ffea",
  "jira.green": "#50fa7b",
  "jira.magenta": "#ff6ac1",
  "jira.purple": "#e135ff",
  "jira.yellow": "#f1fa8c",
  "jira.orange": "#ffb86c",
  "slack.blue": "#82aaff",
  "slack.cyan": "#80ffea",
  "slack.green": "#50fa7b",
  "slack.magenta": "#ff6ac1",
  "slack.purple": "#e135ff",
  "slack.yellow": "#f1fa8c",
  "slack.orange": "#ffb86c"
};

export interface PackIcon {
  style: string;
  unicode: string;
  nerd: string;
  emoji: string;
}
export const PACK: Readonly<Record<string, PackIcon>> = {
 "group": {
  "style": "accent.primary",
  "unicode": "◆",
  "nerd": "",
  "emoji": "🏢"
 },
 "repo": {
  "style": "accent.secondary",
  "unicode": "▤",
  "nerd": "",
  "emoji": "📦"
 },
 "branch": {
  "style": "accent.secondary",
  "unicode": "⎇",
  "nerd": "",
  "emoji": "🌿"
 },
 "commit": {
  "style": "text.muted",
  "unicode": "◦",
  "nerd": "",
  "emoji": "🔘"
 },
 "change": {
  "style": "accent.primary",
  "unicode": "⇄",
  "nerd": "",
  "emoji": "🔀"
 },
 "draft": {
  "style": "text.muted",
  "unicode": "◌",
  "nerd": "",
  "emoji": "📝"
 },
 "merged": {
  "style": "glow.purple",
  "unicode": "◈",
  "nerd": "",
  "emoji": "🟣"
 },
 "closed": {
  "style": "status.error",
  "unicode": "⊘",
  "nerd": "",
  "emoji": "⛔"
 },
 "approved": {
  "style": "status.success",
  "unicode": "✓",
  "nerd": "",
  "emoji": "👍"
 },
 "needs_approval": {
  "style": "status.warning",
  "unicode": "◎",
  "nerd": "",
  "emoji": "👀"
 },
 "thread": {
  "style": "accent.secondary",
  "unicode": "❝",
  "nerd": "",
  "emoji": "💬"
 },
 "comment": {
  "style": "text.secondary",
  "unicode": "✎",
  "nerd": "",
  "emoji": "💭"
 },
 "resolved": {
  "style": "status.success",
  "unicode": "✓",
  "nerd": "",
  "emoji": "🆗"
 },
 "conflict": {
  "style": "status.error",
  "unicode": "≠",
  "nerd": "",
  "emoji": "💥"
 },
 "rebase": {
  "style": "status.warning",
  "unicode": "↻",
  "nerd": "",
  "emoji": "🔁"
 },
 "label": {
  "style": "accent.tertiary",
  "unicode": "⌗",
  "nerd": "",
  "emoji": "🔖"
 },
 "pipeline": {
  "style": "status.info",
  "unicode": "⋮",
  "nerd": "",
  "emoji": "🚦"
 },
 "run_passed": {
  "style": "status.success",
  "unicode": "✓",
  "nerd": "",
  "emoji": "✅"
 },
 "run_failed": {
  "style": "status.error",
  "unicode": "✗",
  "nerd": "",
  "emoji": "❌"
 },
 "run_running": {
  "style": "status.info",
  "unicode": "●",
  "nerd": "",
  "emoji": "🔄"
 },
 "run_pending": {
  "style": "text.muted",
  "unicode": "◷",
  "nerd": "",
  "emoji": "⏳"
 },
 "run_skipped": {
  "style": "text.muted",
  "unicode": "»",
  "nerd": "",
  "emoji": "⏩"
 },
 "run_manual": {
  "style": "accent.primary",
  "unicode": "▷",
  "nerd": "",
  "emoji": "👆"
 },
 "run_canceled": {
  "style": "text.muted",
  "unicode": "⊘",
  "nerd": "",
  "emoji": "🚫"
 },
 "job": {
  "style": "status.info",
  "unicode": "▸",
  "nerd": "",
  "emoji": "👷"
 },
 "log": {
  "style": "text.muted",
  "unicode": "≣",
  "nerd": "",
  "emoji": "📜"
 },
 "issue": {
  "style": "accent.secondary",
  "unicode": "◇",
  "nerd": "",
  "emoji": "📋"
 },
 "epic": {
  "style": "glow.purple",
  "unicode": "✦",
  "nerd": "",
  "emoji": "⚡"
 },
 "story": {
  "style": "status.success",
  "unicode": "▣",
  "nerd": "",
  "emoji": "📗"
 },
 "task": {
  "style": "status.info",
  "unicode": "☐",
  "nerd": "",
  "emoji": "📘"
 },
 "bug": {
  "style": "status.error",
  "unicode": "✱",
  "nerd": "",
  "emoji": "🐛"
 },
 "subtask": {
  "style": "status.info",
  "unicode": "↳",
  "nerd": "",
  "emoji": "🔹"
 },
 "priority_highest": {
  "style": "status.error",
  "unicode": "⇈",
  "nerd": "",
  "emoji": "⏫"
 },
 "priority_high": {
  "style": "status.error",
  "unicode": "↑",
  "nerd": "",
  "emoji": "🔼"
 },
 "priority_medium": {
  "style": "status.warning",
  "unicode": "=",
  "nerd": "",
  "emoji": "🟡"
 },
 "priority_low": {
  "style": "status.info",
  "unicode": "↓",
  "nerd": "",
  "emoji": "🔽"
 },
 "priority_lowest": {
  "style": "status.info",
  "unicode": "⇊",
  "nerd": "",
  "emoji": "⏬"
 },
 "todo": {
  "style": "text.muted",
  "unicode": "○",
  "nerd": "",
  "emoji": "⚪"
 },
 "in_progress": {
  "style": "status.info",
  "unicode": "◐",
  "nerd": "",
  "emoji": "🔵"
 },
 "done": {
  "style": "status.success",
  "unicode": "●",
  "nerd": "",
  "emoji": "🟢"
 },
 "site": {
  "style": "accent.secondary",
  "unicode": "⊕",
  "nerd": "",
  "emoji": "🌐"
 },
 "board": {
  "style": "accent.secondary",
  "unicode": "⊟",
  "nerd": "",
  "emoji": "📊"
 },
 "person": {
  "style": "text.secondary",
  "unicode": "☺",
  "nerd": "",
  "emoji": "👤"
 },
 "you": {
  "style": "accent.primary",
  "unicode": "☻",
  "nerd": "",
  "emoji": "🙋"
 },
 "bot": {
  "style": "text.muted",
  "unicode": "◙",
  "nerd": "󰚩",
  "emoji": "🤖"
 },
 "team": {
  "style": "text.secondary",
  "unicode": "⚇",
  "nerd": "",
  "emoji": "👥"
 },
 "chat": {
  "style": "accent.secondary",
  "unicode": "✉",
  "nerd": "",
  "emoji": "💬"
 },
 "channel": {
  "style": "text.secondary",
  "unicode": "#",
  "nerd": "",
  "emoji": "📢"
 },
 "private_channel": {
  "style": "status.warning",
  "unicode": "⊡",
  "nerd": "",
  "emoji": "🔒"
 },
 "direct": {
  "style": "accent.primary",
  "unicode": "✉",
  "nerd": "",
  "emoji": "📩"
 },
 "reply": {
  "style": "accent.secondary",
  "unicode": "↩",
  "nerd": "",
  "emoji": "🧵"
 },
 "mention": {
  "style": "accent.primary",
  "unicode": "@",
  "nerd": "",
  "emoji": "📣"
 },
 "reaction": {
  "style": "status.warning",
  "unicode": "♥",
  "nerd": "",
  "emoji": "😀"
 },
 "unread": {
  "style": "accent.primary",
  "unicode": "•",
  "nerd": "",
  "emoji": "🆕"
 },
 "alert": {
  "style": "status.warning",
  "unicode": "▲",
  "nerd": "",
  "emoji": "🔔"
 },
 "alert_firing": {
  "style": "status.error",
  "unicode": "▲",
  "nerd": "",
  "emoji": "🔥"
 },
 "alert_resolved": {
  "style": "status.success",
  "unicode": "△",
  "nerd": "",
  "emoji": "💚"
 },
 "settings": {
  "style": "text.secondary",
  "unicode": "⚙",
  "nerd": "",
  "emoji": "🔧"
 },
 "action": {
  "style": "accent.secondary",
  "unicode": "▷",
  "nerd": "",
  "emoji": "⚡"
 },
 "copy": {
  "style": "accent.secondary",
  "unicode": "⧉",
  "nerd": "",
  "emoji": "📋"
 },
 "link": {
  "style": "accent.secondary",
  "unicode": "↗",
  "nerd": "",
  "emoji": "🔗"
 },
 "search": {
  "style": "accent.secondary",
  "unicode": "⊙",
  "nerd": "",
  "emoji": "🔍"
 },
 "time": {
  "style": "text.muted",
  "unicode": "◷",
  "nerd": "",
  "emoji": "🕒"
 },
 "updated": {
  "style": "text.muted",
  "unicode": "↺",
  "nerd": "",
  "emoji": "🔃"
 },
 "calendar": {
  "style": "text.muted",
  "unicode": "▦",
  "nerd": "",
  "emoji": "📅"
 },
 "info": {
  "style": "status.info",
  "unicode": "ℹ",
  "nerd": "",
  "emoji": "💡"
 },
 "warning": {
  "style": "status.warning",
  "unicode": "⚠",
  "nerd": "",
  "emoji": "🚧"
 },
 "error": {
  "style": "status.error",
  "unicode": "✗",
  "nerd": "",
  "emoji": "🛑"
 },
 "empty": {
  "style": "text.muted",
  "unicode": "∅",
  "nerd": "",
  "emoji": "📭"
 },
 "menu": {
  "style": "text.muted",
  "unicode": "≡",
  "nerd": "",
  "emoji": "📋"
 },
 "keys": {
  "style": "text.muted",
  "unicode": "⌘",
  "nerd": "",
  "emoji": "🎹"
 },
 "close": {
  "style": "text.muted",
  "unicode": "×",
  "nerd": "\uf467",
  "emoji": "❌"
 },
 "quit": {
  "style": "text.muted",
  "unicode": "✕",
  "nerd": "",
  "emoji": "🚪"
 },
 "navigate": {
  "style": "text.muted",
  "unicode": "↔",
  "nerd": "",
  "emoji": "🧭"
 },
 "scroll": {
  "style": "text.muted",
  "unicode": "↕",
  "nerd": "",
  "emoji": "📜"
 },
 "bulk": {
  "style": "text.muted",
  "unicode": "◫",
  "nerd": "",
  "emoji": "📚"
 },
 "fold": {
  "style": "text.muted",
  "unicode": "±",
  "nerd": "",
  "emoji": "🔽"
 },
 "colours": {
  "style": "accent.primary",
  "unicode": "◑",
  "nerd": "",
  "emoji": "🎨"
 },
 "icons": {
  "style": "status.warning",
  "unicode": "☺",
  "nerd": "",
  "emoji": "😀"
 },
 "parts": {
  "style": "accent.secondary",
  "unicode": "⊞",
  "nerd": "",
  "emoji": "🧩"
 },
 "connection": {
  "style": "status.success",
  "unicode": "⇌",
  "nerd": "",
  "emoji": "🔌"
 },
 "views": {
  "style": "accent.secondary",
  "unicode": "▽",
  "nerd": "",
  "emoji": "👓"
 },
 "look": {
  "style": "accent.primary",
  "unicode": "◑",
  "nerd": "",
  "emoji": "💅"
 },
 "local": {
  "style": "text.secondary",
  "unicode": "⌂",
  "nerd": "",
  "emoji": "💻"
 },
 "editor": {
  "style": "accent.secondary",
  "unicode": "✎",
  "nerd": "",
  "emoji": "📝"
 },
 "terminal": {
  "style": "text.secondary",
  "unicode": "$",
  "nerd": "",
  "emoji": "🐚"
 },
 "mode_read": {
  "style": "status.info",
  "unicode": "◉",
  "nerd": "",
  "emoji": "🔒"
 },
 "mode_write": {
  "style": "accent.secondary",
  "unicode": "✎",
  "nerd": "",
  "emoji": "📝"
 },
 "mode_edit": {
  "style": "accent.primary",
  "unicode": "✎",
  "nerd": "",
  "emoji": "🔨"
 },
 "file": {
  "style": "text.secondary",
  "unicode": "▫",
  "nerd": "",
  "emoji": "📄"
 },
 "folder": {
  "style": "accent.secondary",
  "unicode": "▪",
  "nerd": "",
  "emoji": "📁"
 },
 "folder_open": {
  "style": "accent.secondary",
  "unicode": "▭",
  "nerd": "",
  "emoji": "📂"
 },
 "file_rust": {
  "style": "status.warning",
  "unicode": "▫",
  "nerd": "",
  "emoji": "🦀"
 },
 "file_java": {
  "style": "status.error",
  "unicode": "▫",
  "nerd": "",
  "emoji": "☕"
 },
 "file_kotlin": {
  "style": "glow.purple",
  "unicode": "▫",
  "nerd": "",
  "emoji": "🎯"
 },
 "file_typescript": {
  "style": "status.info",
  "unicode": "▫",
  "nerd": "",
  "emoji": "🔷"
 },
 "file_javascript": {
  "style": "status.warning",
  "unicode": "▫",
  "nerd": "",
  "emoji": "🟨"
 },
 "file_python": {
  "style": "status.info",
  "unicode": "▫",
  "nerd": "",
  "emoji": "🐍"
 },
 "file_go": {
  "style": "glow.cyan",
  "unicode": "▫",
  "nerd": "",
  "emoji": "🐹"
 },
 "file_markdown": {
  "style": "text.secondary",
  "unicode": "▫",
  "nerd": "",
  "emoji": "📝"
 },
 "file_json": {
  "style": "status.warning",
  "unicode": "▫",
  "nerd": "",
  "emoji": "🧾"
 },
 "file_yaml": {
  "style": "status.error",
  "unicode": "▫",
  "nerd": "",
  "emoji": "📒"
 },
 "file_toml": {
  "style": "text.secondary",
  "unicode": "▫",
  "nerd": "",
  "emoji": "📙"
 },
 "file_docker": {
  "style": "status.info",
  "unicode": "▫",
  "nerd": "",
  "emoji": "🐳"
 },
 "file_sql": {
  "style": "glow.pink",
  "unicode": "▫",
  "nerd": "",
  "emoji": "💾"
 },
 "file_html": {
  "style": "status.error",
  "unicode": "▫",
  "nerd": "",
  "emoji": "🌐"
 },
 "file_css": {
  "style": "status.info",
  "unicode": "▫",
  "nerd": "",
  "emoji": "🎨"
 },
 "file_shell": {
  "style": "status.success",
  "unicode": "▫",
  "nerd": "",
  "emoji": "🐚"
 },
 "file_c": {
  "style": "status.info",
  "unicode": "▫",
  "nerd": "",
  "emoji": "🔤"
 },
 "file_cpp": {
  "style": "status.info",
  "unicode": "▫",
  "nerd": "",
  "emoji": "➕"
 },
 "file_ruby": {
  "style": "status.error",
  "unicode": "▫",
  "nerd": "",
  "emoji": "💎"
 },
 "file_php": {
  "style": "glow.purple",
  "unicode": "▫",
  "nerd": "",
  "emoji": "🐘"
 },
 "file_swift": {
  "style": "status.warning",
  "unicode": "▫",
  "nerd": "",
  "emoji": "🐦"
 },
 "file_image": {
  "style": "glow.pink",
  "unicode": "▫",
  "nerd": "",
  "emoji": "📷"
 },
 "file_git": {
  "style": "status.warning",
  "unicode": "▫",
  "nerd": "",
  "emoji": "🌱"
 },
 "file_lock": {
  "style": "text.muted",
  "unicode": "▫",
  "nerd": "",
  "emoji": "🔒"
 },
 "file_xml": {
  "style": "status.warning",
  "unicode": "▫",
  "nerd": "",
  "emoji": "📰"
 },
 "file_text": {
  "style": "text.secondary",
  "unicode": "▫",
  "nerd": "",
  "emoji": "📃"
 }
};
