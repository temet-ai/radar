import type { Chord } from "../chrome/types.js";
import type { Mnemonic } from "../text/mnemonic.js";

/** `ActionSpec` (view.rs): one thing that can be done where the view is focused. */
export type ActionSpec = Mnemonic & {
  id: string;
  label: string;
  key?: Chord | null;
  heading?: boolean;
  icon?: string | null;
  change?: "add" | "edit" | null;
};
