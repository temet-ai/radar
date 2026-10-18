import type { ChromeView } from "../chrome/types.js";
/** A displayed history back click captures the same bounded dismissal as physical Escape. */
export function noticeHistoryEscapeTarget(chrome: ChromeView | undefined, target: string): boolean {
  if (!chrome?.notice_history) return false;
  if (target === "chrome:notices:close") return true;
  return (chrome.status.hints ?? []).some((hint) => hint.keys_of?.some((key) => key.target === target && key.chord.key === "esc" && !key.chord.ctrl && !key.chord.alt));
}
