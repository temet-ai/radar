import type { Key as InkKey } from "ink";
import type { Chord, Hit, Intent, MenuView } from "../chrome/types.js";
import { hitAt } from "../chrome/layout.js";
import { chordOf } from "./keys.js";
import { selectIntent, type SelectMove } from "../editor/selection.js";
import { parseMouse, type MouseEvent } from "./mouse.js";
import type { UiState } from "../state/ui-state.js";

/** What the controller needs to know about the frame showing. */
export interface InputView {
  /** The menu or question open, if one is. */
  menu(): MenuView | null;
  /** The host is capturing text (an input request or a menu's field): every key goes to it. */
  capturing(): boolean;
  /** What a key is on the body showing (view_tui::key / section_key): movement selects, ⏎ activates, the rest is Intent::Key. */
  route(chord: Chord): Intent;
  /** The key on a pane the frontend scrolls itself: "done" when it scrolled (nothing is sent), an Intent to send, or none. */
  scroll?(chord: Chord): Intent | "done" | undefined;
  /** The open capturing editor's data, when one is (screen.overlay.body.data of the editor capability). */
  editor?(): { line: string; caret: number; selection?: [number, number] | null } | null;
  /** The capitals the host takes as screen or source keys (ChromeView.screen_keys). */
  screenKeys?(): string[] | undefined;
  /** An intent other than a Select or Fold was sent: the host's state is unknown until its next snapshot. */
  desync?(): void;
  /** Every pointer target on screen now, topmost last. */
  hits(): Hit[];
  /** The agent dock has the keys (DockView.focused): true when it took the chord (src/dock/composer.ts). */
  dock?(chord: Chord): boolean;
  /** A pointer event on the dock, or one that continues a drag of its edge: true when the dock took it. */
  dockMouse?(ev: MouseEvent, hit: Hit | undefined): boolean;
  /** The pane drawn at the cell (HitRegistry.paneAt), if any. */
  paneAt?(x: number, y: number): string | undefined;
  /** The wheel over pane `pane` (none: over no pane) at the cell: that pane scrolls (and has the focus, host.rs
   * on_mouse), nothing else does. False, or without it: the wheel is k/j to whatever has the keys. */
  wheel?(pane: string | undefined, up: boolean, x: number, y: number): boolean;
}

/** The pane a work area's section target names (`pane:<id>` or `pane:<id>#n`, workarea/model.ts). */
const paneOfTarget = (target: string | undefined): string | undefined => /^pane:(nav|middle|detail)(#\d+)?$/.exec(target ?? "")?.[1];

export const DOUBLE_CLICK_MS = 400;

/**
 * The menu showing now. The host asks a menu once (ToRenderer::Question) and moving through it, typing in it or
 * scrolling it show only in the chrome's copy of the same menu (bridge.rs ask_and_tell); so while the asked menu is
 * open the host's current copy is the one drawn and keyed, and the asked one only until the first snapshot has it.
 */
export function menuShowing(asked: MenuView | undefined, chrome: MenuView | null | undefined): MenuView | null {
  if (asked && chrome && chrome.id === asked.id) return chrome;
  return asked ?? chrome ?? null;
}

/** Keys the host's `on_key` takes before the body's movement: an ASCII capital switches screen when a screen has it. */
function hostFirst(chord: Chord, claimed: string[] | undefined): boolean {
  if (typeof chord.key !== "object" || !/^[A-Z]$/.test(chord.key.char) || chord.ctrl || chord.alt) return false;
  // The host lists the capitals it takes (ChromeView.screen_keys); an unclaimed one is the body's (G: last row, or
  // the end of a document). An empty list is a build where no part claims a capital: G is the body's then too (the host
  // fills the list always now). Only a frontend without the field (no list at all) sends every capital to the host.
  return !claimed || claimed.includes(chord.key.char);
}

/** Turns keys and mouse reports into the Intents the Ratatui adapter sends for the same view. */
export class InputController {
  private lastClick: { target: string; at: number } | undefined;
  /** Where the arrows already sent have left a menu's selection, until the host's next snapshot replaces the menu. */
  private menuAt: { menu: MenuView; selected: number } | undefined;

  constructor(
    private readonly view: InputView,
    private readonly sendRaw: (intent: Intent) => void,
    private readonly ui: UiState,
    private readonly now: () => number = Date.now,
    /** What the pointer is over, cut to its row (host.rs hover_at), to be lit; null when nothing is. Called on every
     * motion report and after every draw: the callee does nothing when the area did not change. */
    private readonly hover: (hit: Hit | null) => void = () => {},
    /** The cell a right-click pressed (0-based), told to the host just before its Context so the menu opens there. */
    private readonly pointer: (column: number, row: number) => void = () => {},
  ) {}

  private send(intent: Intent) {
    this.sendRaw(intent);
    if (intent.intent !== "select" && intent.intent !== "fold") this.view.desync?.();
  }

  /** A keypress from Ink's useInput. A paste (several characters) is sent as one key per character. */
  key(input: string, key: InkKey) {
    const chord = chordOf(input, key);
    if (!chord) {
      if (!key.ctrl && !key.meta) {
        if (this.view.capturing()) this.paste(input);
        else for (const c of [...input]) this.keyChord({ key: { char: c } });
      }
      return;
    }
    this.keyChord(chord);
  }

  /**
   * A paste (TextInput::paste, input.rs): while the host is capturing text it is the text with each run of line
   * breaks one space, leading and trailing breaks dropped, one character key each; otherwise nothing takes it.
   */
  paste(text: string) {
    if (!this.view.capturing()) return;
    const cleaned = text.split(/[\r\n]+/).filter((p) => p !== "").join(" ");
    for (const c of [...cleaned]) this.keyChord({ key: { char: c } });
  }

  keyChord(chord: Chord, shift = false) {
    // A key between two clicks makes them two clicks, not a double one.
    this.lastClick = undefined;
    const menu = this.view.menu();
    if (this.view.capturing()) {
      // Shift with a movement key in the capturing editor selects (Intent::EditSelect): the host owns the range.
      const editor = shift && !menu ? this.view.editor?.() : null;
      const move = typeof chord.key === "string" && ["left", "right", "home", "end"].includes(chord.key) && !chord.ctrl && !chord.alt ? (chord.key as SelectMove) : null;
      if (editor && move) {
        this.send(selectIntent(editor, move));
        return;
      }
      this.send({ intent: "key", chord });
      return;
    }
    if (menu) {
      this.menuKey(menu, chord);
      return;
    }
    if (this.view.dock?.(chord)) return;
    // An uppercase letter is the host's first (on_key: a screen or source key such as G for GitHub switches
    // there); only when no screen has it does it reach the part, where `G` is "last row". The host makes that
    // choice, so the letter goes to it as a key, not as the Select the body's own movement would be.
    if (hostFirst(chord, this.view.screenKeys?.())) {
      this.send({ intent: "key", chord });
      return;
    }
    // A Document or Rich pane scrolls in the frontend (view_tui::key); the host hears of it only at a Rich pane's end.
    const scrolled = this.view.scroll?.(chord);
    if (scrolled === "done") return;
    if (scrolled) {
      this.send(scrolled);
      return;
    }
    this.send(this.view.route(chord));
  }

  private menuKey(menu: MenuView, chord: Chord) {
    const items = menu.items;
    const pickable = (i: number) => {
      const c = items[i];
      return !!c && !(c.shown_only || c.heading || c.note);
    };
    if (this.menuAt?.menu !== menu) this.menuAt = menu.selected == null ? undefined : { menu, selected: menu.selected };
    const sel = this.menuAt?.selected;
    // j and k move too (menu_key), unless a choice has that key.
    const ch = typeof chord.key === "object" ? chord.key.char : undefined;
    const taken = (c: string) => items.some((it) => pickable(items.indexOf(it)) && (it.key ?? it.keys) === c);
    const dir = chord.key === "down" || (ch === "j" && !taken("j")) ? 1 : chord.key === "up" || (ch === "k" && !taken("k")) ? -1 : 0;
    if (!chord.ctrl && !chord.alt && sel !== undefined && dir !== 0) {
      const step = dir;
      for (let i = sel + step; i >= 0 && i < items.length; i += step) {
        if (pickable(i)) {
          this.menuAt = { menu, selected: i };
          break;
        }
      }
    }
    if (typeof chord.key === "object" && !chord.ctrl && !chord.alt) {
      const ch = chord.key.char;
      const hit = items.find((c) => !(c.shown_only || c.heading || c.note) && (c.key ?? c.keys) === ch);
      if (hit) {
        this.sendRaw({ intent: "invoke", action: hit.value });
        return;
      }
    }
    if (chord.key === "enter" && sel !== undefined) {
      const c = items[sel];
      if (c && pickable(sel)) {
        this.sendRaw({ intent: "invoke", action: c.value });
        return;
      }
    }
    // Movement, esc and anything else go to the host, which owns the selection.
    this.sendRaw({ intent: "key", chord });
  }

  /** The terminal lost focus: nothing is lit under a pointer that left the window. */
  blur() {
    this.pointerAt = undefined;
    this.hover(null);
  }

  /** Where the pointer last was: shell.rs hover lights what a redraw puts under a still pointer, so the frame asks again
   * after each draw (a list that scrolls under the mouse, a screen switch). */
  private pointerAt: { x: number; y: number } | undefined;
  refreshHover() {
    const at = this.pointerAt;
    if (!at) return;
    this.hover(this.view.menu() ? null : this.litAt(at.x, at.y));
  }

  /** host.rs hover_at: the hit under the pointer, cut to the pointer's row; while text is typed only the status line's
   * keys are lit, and a pane's own area never is. */
  private litAt(x: number, y: number): Hit | null {
    const over = hitAt(this.view.hits(), x, y);
    if (!over) return null;
    if (this.view.capturing() && !over.target.startsWith("chrome:key:")) return null;
    if (over.target === "pane" || over.target.startsWith("pane:")) return null;
    return { ...over, y, h: 1 };
  }

  /** Raw stdin data that holds SGR mouse reports. */
  mouse(data: string) {
    for (const ev of parseMouse(data)) this.mouseEvent(ev);
  }

  mouseEvent(ev: MouseEvent) {
    const menu = this.view.menu();
    const typing = this.view.capturing();
    if (!menu && !typing && this.view.dockMouse?.(ev, hitAt(this.view.hits(), ev.x, ev.y))) return;
    if (typing && (ev.kind === "wheel_up" || ev.kind === "wheel_down")) return;
    if (ev.kind === "wheel_up" || ev.kind === "wheel_down") {
      // The host presses ↑/↓ in a menu and k/j on the body (the wheel moves the selection).
      const up = ev.kind === "wheel_up";
      if (menu) {
        this.keyChord({ key: up ? "up" : "down" });
        return;
      }
      // What is under the pointer scrolls, and nothing else: not the pane that has the keys, not the dock's composer.
      const pane = this.view.paneAt?.(ev.x, ev.y) ?? paneOfTarget(hitAt(this.view.hits(), ev.x, ev.y)?.target);
      if (!this.view.wheel?.(pane, up, ev.x, ev.y)) this.keyChord({ key: { char: up ? "k" : "j" } });
      return;
    }
    if (ev.kind === "motion" || ev.kind === "press") this.pointerAt = { x: ev.x, y: ev.y };
    if (ev.kind === "motion") {
      // What a click would act on is lit under the pointer (ui::hover::paint, shell.rs hover): any hit but a menu item.
      if (menu && !typing) {
        // Over a menu the menu's highlight moves to the item under the pointer (host.rs hover); nothing is lit.
        const at = hitAt(this.view.hits(), ev.x, ev.y);
        const idx = at ? menu.items.findIndex((c) => c.target === at.target) : -1;
        const item = menu.items[idx];
        if (at && item && !(item.shown_only || item.heading || item.note)) {
          if (this.menuAt?.menu !== menu) this.menuAt = menu.selected == null ? undefined : { menu, selected: menu.selected };
          if (this.menuAt?.selected !== idx) {
            this.menuAt = { menu, selected: idx };
            this.sendRaw({ intent: "select", target: at.target });
          }
        }
        this.hover(null);
        return;
      }
      this.hover(menu ? null : this.litAt(ev.x, ev.y));
      return;
    }
    if (ev.kind !== "press") return;
    const hit = hitAt(this.view.hits(), ev.x, ev.y);
    if (!hit) {
      // A press away from an open menu closes it (the host's own rule).
      if (menu && !this.view.capturing()) this.sendRaw({ intent: "key", chord: { key: "esc" } });
      // A left press on a pane's empty space or border only focuses it (host.rs on_mouse, Hit::Pane).
      else if (!menu && !typing && ev.button === "left") {
        const pane = this.view.paneAt?.(ev.x, ev.y);
        if (pane) this.sendRaw({ intent: "select", target: `pane:${pane}` });
      }
      return;
    }
    const target = hit.target;
    // Typing goes on until ⏎ or esc: a click elsewhere does not drop it. The status line's keys and a menu's own
    // rows are still pressed (host.rs on_mouse).
    if (typing && !target.startsWith("chrome:key:") && !target.startsWith("chrome:menu:")) return;
    if (ev.button === "right") {
      this.ui.select(target);
      this.pointer(ev.x, ev.y);
      this.sendRaw({ intent: "context", target });
      return;
    }
    if (ev.button !== "left") return;
    const t = this.now();
    const double = this.lastClick?.target === target && t - this.lastClick.at <= DOUBLE_CLICK_MS;
    this.lastClick = double ? undefined : { target, at: t };
    this.ui.select(target);
    // The chrome (tabs, mode chip, status keys, menu rows) acts on one click; rows select, and
    // two clicks within 400 ms are ⏎ (docs/GUIDE.md 2.5, host.rs on_mouse).
    if (target.startsWith("chrome:")) this.sendRaw({ intent: "activate", target });
    else {
      // The host's click is Select; the second click of a double adds ⏎ on the selection.
      this.sendRaw({ intent: "select", target });
      if (double) this.sendRaw({ intent: "activate", target });
    }
  }
}
