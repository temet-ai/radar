// A compile-time check, no runtime: the node types the components draw (view.ts, hand-typed) must accept what the
// host's generated types (src/protocol/generated) say can arrive. A Rust-side change that adds or reshapes a node field
// the components do not know fails `npm run typecheck` here.
import type { Node as Generated } from "../protocol/generated/Node.js";
import type { ScreenView } from "../protocol/generated/ScreenView.js";
import type { Node } from "./view.js";

export const acceptsGenerated = (node: Generated): Node => node;
export const screenPanes = (view: ScreenView): Node[] => view.panes.map((p) => p.body);
