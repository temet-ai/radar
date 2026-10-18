import { createContext } from "react";
import type { MotionScene } from "./interaction.js";

/** Semantic identity comes from the host; geometry comes from the components that actually draw. */
export const MotionSceneContext = createContext<MotionScene | undefined>(undefined);
export const MotionPaneContext = createContext<string | undefined>(undefined);
