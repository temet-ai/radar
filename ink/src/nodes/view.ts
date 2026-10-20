import type { Mnemonic } from "../text/mnemonic.js";

// The view data the Ink nodes draw: the TypeScript shape of crates/radar-core/src/view.rs (`Node` and what it holds),
// as serde writes it. Written by hand until T1's view types are exported with ts-rs; only what a node draws is
// typed, and every optional field is optional because serde skips defaults.

export type InkName =
  | "plain" | "primary" | "secondary" | "muted" | "emphasis" | "accent" | "accent_muted"
  | "accent_secondary" | "accent_tertiary" | "good" | "warn" | "bad" | "info" | "number" | "added" | "removed";

export type Target = string;

export interface Seg {
  text?: string;
  ink?: InkName;
  strong?: boolean;
  marked?: boolean;
  icon?: string | null;
  icon_lead?: boolean;
  link?: string | null;
  target?: Target | null;
  look?: string | null;
  chip?: boolean;
  /** Exact original shortcut appended by plain readers; visual styling already carries the mnemonic. */
  key_hint?: string | null;
}

/** One line of rich text: its runs in order (serde-transparent). */
export type Text = Seg[];

export interface ListRow {
  target: Target;
  text: Text;
  right?: Text | null;
  link?: string | null;
  detail?: Text | null;
  heading?: boolean;
}

export interface Editing {
  row: number;
  at: number;
  text: string;
  caret: number;
}

export interface ColumnSpec {
  heading: string;
  width: number;
  keep: number;
  icon?: string | null;
  flex?: boolean;
  extra?: boolean;
}

export interface SubjectRef {
  kind: string;
  id: string;
  label: string;
  url?: string | null;
  fields?: Record<string, string>;
}

export interface CellView {
  heading: string;
  text: Text;
  show?: SubjectRef | null;
  target?: Target | null;
}

export interface TreeRow {
  id: string;
  name: Text;
  heading?: boolean;
  cells?: CellView[];
  children?: TreeRow[];
  stripe?: string | null;
  chips?: { gives: number; text: Text } | null;
  glow?: string | null;
  detail?: Text | null;
}

export type State = "empty" | "loading" | "failed";

export interface Note {
  state: State;
  text: string;
}

export interface Tree {
  header?: Text;
  columns: ColumnSpec[];
  ordered_columns?: boolean;
  rows: TreeRow[];
  selected?: string[];
  open?: string[][];
  name_width?: number;
  look?: { lines: string; band: string; heading: string; band_level: number } | null;
  header_target?: Target | null;
  note?: Note | null;
  cut_names?: boolean;
  editing?: { path: string[]; at: number; text: string; caret: number } | null;
  pointer_action?: boolean;
}

export type Tone = "plain" | "muted" | "accent" | "info" | "good" | "warn" | "bad";

export interface Marks {
  bold?: boolean;
  italic?: boolean;
  strike?: boolean;
  underline?: boolean;
  code?: boolean;
}

export type InlineKind =
  | "text" // the serde default, omitted on the wire
  | { link: { url: string } }
  | { mention: { me?: boolean } }
  | "emoji"
  | { badge: { tone: Tone } }
  | { image: { src: string } }
  | "break";

export interface Inline {
  text: string;
  kind?: InlineKind;
  marks?: Marks;
}

export type Marker = "bullet" | "todo" | "done" | "decision" | { number: number };

export interface RichMedia {
  image?: boolean;
  name?: string;
  url?: string;
  src?: string;
  size?: [number, number] | null;
}

export type RichBlock =
  | { t: "para"; text: Inline[] }
  | { t: "heading"; level: number; text: Inline[] }
  | { t: "list"; items: { marker: Marker; blocks: RichBlock[] }[] }
  | { t: "quote"; blocks: RichBlock[] }
  | { t: "callout"; tone: Tone; symbol?: string; blocks: RichBlock[] }
  | { t: "code"; lang?: string; text: string }
  | { t: "table"; aligns?: ("left" | "center" | "right")[]; head?: Inline[][]; rows: Inline[][][] }
  | { t: "rule" }
  | { t: "details"; summary: Inline[]; blocks: RichBlock[] }
  | { t: "media" } & RichMedia;

export interface MeterStage {
  name: string;
  mark: "done" | "failed" | "running" | "waiting" | "manual" | "stopped" | "skipped";
  look: string;
}

export type Block =
  | ({ block: "line"; text: Text; target?: Target | null; key?: string | null } & Mnemonic)
  | { block: "section"; mark?: string | null; title: string; detail?: string | null; blocks: Block[] }
  | { block: "doc"; doc: RichBlock[]; bar?: Seg | null; most?: number | null; urls?: boolean | null }
  | { block: "meter"; stages: MeterStage[] }
  | { block: "wrapped"; text: string; prefix: Seg; ink: InkName };

export interface TableRow {
  target: Target;
  cells: CellView[];
}

export interface EntryView {
  target: Target;
  head: Text;
  before?: Text[];
  body?: Text[];
}

export interface LogPaintLine {
  text: Text;
  row: number;
  selected: boolean;
}

export type DiffKind = "Hunk" | "Context" | "Added" | "Removed" | "Note"; // serde writes the variant names as is

export interface DiffLine {
  kind: DiffKind;
  old: number | null;
  new: number | null;
  text: string;
}

export interface DiffUnder {
  line: number;
  lines: Text[];
  input?: { text: string; caret: number; selection?: [number, number] | null } | null;
}

export type Node =
  | { node: "list"; rows: ListRow[]; selected?: number | null; editing?: Editing | null }
  | ({ node: "tree" } & Tree)
  | { node: "document"; lines: Text[]; cut?: boolean }
  | { node: "rich"; blocks: Block[]; pictures?: string | null }
  | { node: "table"; columns: ColumnSpec[]; rows: TableRow[]; selected?: number | null; empty?: string | null }
  | { node: "entries"; rows: EntryView[]; selected?: number | null; empty: string }
  | { node: "log"; lines: LogPaintLine[]; top: number; total: number; empty: string }
  | {
      node: "diff";
      lines: DiffLine[];
      under: DiffUnder[];
      selected: number;
      top: number;
      split: boolean;
      language?: string | null;
      empty: string;
      /** The host's syntax runs per line (Node::Diff.painted); absent when the language is unknown. */
      painted?: { text: string; token: string }[][];
      /** The words that changed in each line, in characters (Node::Diff.words); absent when none did. */
      words?: [number, number][][];
    }
  | { node: "state"; state: State; text: string }
  | { node: "extension"; capability: string; version: number; data: unknown; fallback: Node };

/** The `editor` capability's data (view::Editor). */
export interface EditorData {
  title: string;
  lines: string[];
  line: string;
  caret: number;
  /** The selected text of `line` as [anchor, extent] counted as `caret` is (the caret is at the extent); absent when none. */
  selection?: [number, number] | null;
}

export type ChordName = {
  key: { char: string } | "enter" | "esc" | "tab" | "back_tab" | "backspace" | "delete" | "up" | "down" | "left" | "right" | "home" | "end" | "page_up" | "page_down";
  ctrl?: boolean;
  alt?: boolean;
};

/** One row of a view's actions (view::ActionSpec). */
export interface ActionSpec extends Mnemonic {
  id: string;
  label: string;
  key?: ChordName | null;
  heading?: boolean;
  icon?: string | null;
  change?: "add" | "edit" | null;
}

/** The version 1 `detail` extension and footer use Rust's exported wire contract directly. */
export type { DetailHeader } from "../protocol/generated/DetailHeader.js";
export type { DetailFact } from "../protocol/generated/DetailFact.js";
export type { DetailContent } from "../protocol/generated/DetailContent.js";
export type { FootAction } from "../protocol/generated/FootAction.js";
export type { FootActions } from "../protocol/generated/FootActions.js";

/** What the node components need from the shell, passed to every `renderNode`. */
export interface NodeContext {
  /** Columns the node may use. */
  width: number;
  /** Rows the node may use; unlimited when absent. */
  height?: number;
  /** Whether the pane holds the keys (selection is drawn strongly only then). */
  focused: boolean;
  /** Plain-text mode: no colour, state said in words. */
  plainText?: boolean;
  /** A Document scrolls in its pane (the settings detail: ui::scroll::paragraph); elsewhere it is cut (widgets::paragraph). */
  scrolls?: boolean;
  scrollIdentity?: string;
  /** The screen pane the node is the body of (PaneView.pane): its scroller reports its rows for it (ToHost::Rows). */
  pane?: string;
  /** PaneView.scroll: where the part moved the pane's scroll; the scroller goes there when it changes. */
  scrollTo?: number;
  bodyTrack?: boolean;
  /** The sheet of settings or setup (shell.rs section_detail_lines): a section's heading is laid one cell wider than the
   * text it is wrapped in, so its rule drops to a row of its own. */
  sheet?: boolean;
  /** Capabilities named by Extension nodes this shell draws itself (`editor`). */
  capabilities?: readonly string[];
  /** The actions of the view the node sits in (ScreenView.actions): a node drawn as a fallback lists them. */
  actions?: readonly ActionSpec[];
  /** Where the node registers the rectangles a pointer press acts on (T10's App clears it each frame). */
  registry?: {
    add(hit: { x: number; y: number; w: number; h: number; target: string; show?: SubjectRef }): void;
    replace?(owner: string, hits: Array<{ x: number; y: number; w: number; h: number; target: string }>): void;
    remove?(owner: string): void;
    /** Where a pane is drawn (HitRegistry.addPane): the wheel and a press on its empty space act on it. */
    addPane?(area: { x: number; y: number; w: number; h: number; pane: string }): void;
    /** Where text can be selected with the mouse (HitRegistry.addSelect): a pane's inside, the dock's transcript. */
    addSelect?(area: import("../input/selection.js").SelectArea): void;
  };
  /** Sends an intent for a target the user chose (T10 wires it to the transport). */
  onIntent?: (intent: { intent: string; [k: string]: unknown }) => void;
}
