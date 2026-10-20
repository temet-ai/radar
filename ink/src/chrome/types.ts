// The host's chrome as view data: the generated types (crates/radar-core/src/view/chrome.rs through
// ts-rs, ink/src/protocol/generated). Nothing here is hand-typed except the renderer's own Hit.

import type { SubjectRef } from "../protocol/generated/SubjectRef.js";

export type { Ink } from "../protocol/generated/Ink.js";
export type { Seg } from "../protocol/generated/Seg.js";
export type { Text } from "../protocol/generated/Text.js";
export type { Target } from "../protocol/generated/Target.js";
export type { Key } from "../protocol/generated/Key.js";
export type { Chord } from "../protocol/generated/Chord.js";
export type { Intent } from "../protocol/generated/Intent.js";
export type { KeysAt } from "../protocol/generated/KeysAt.js";
export type { Badge } from "../protocol/generated/Badge.js";
export type { BarTab } from "../protocol/generated/BarTab.js";
export type { ModeChip } from "../protocol/generated/ModeChip.js";
export type { TopBar } from "../protocol/generated/TopBar.js";
export type { SourceTab } from "../protocol/generated/SourceTab.js";
export type { HintKey } from "../protocol/generated/HintKey.js";
export type { Hint } from "../protocol/generated/Hint.js";
export type { Message } from "../protocol/generated/Message.js";
export type { StatusLine } from "../protocol/generated/StatusLine.js";
export type { HealthChip } from "../protocol/generated/HealthChip.js";
export type { Choice } from "../protocol/generated/Choice.js";
export type { MenuView } from "../protocol/generated/MenuView.js";
export type { NavSection } from "../protocol/generated/NavSection.js";
export type { NavGroup } from "../protocol/generated/NavGroup.js";
export type { SettingsNav } from "../protocol/generated/SettingsNav.js";
export type { ChromeView } from "../protocol/generated/ChromeView.js";

/** A rectangle of terminal cells (0-based), the target a pointer there acts on. */
// A contributed cell that shows a subject carries it: a press sends `show` on it, not `select`.
export type Hit = { x: number; y: number; w: number; h: number; target: string; show?: SubjectRef };
