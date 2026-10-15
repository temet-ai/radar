import "./setup";
// radar's own Output (the alias `ink-output` points at Ink's output.js, as in package.json's bundle script): every
// story's frame is composed the way radar composes it.
import "../src/app/fast-output";
// The paint over every finished frame (the scrim and shadow behind a menu, the hover), as the renderer has it.
import "../src/app/hover";
import "../stories/support/terminal.css";
import type { Preview } from "@storybook/react-vite";
import { tiers } from "../stories/support/look";
import { yogaReady } from "./shims/yoga";

const preview: Preview = {
  globalTypes: {
    tier: {
      description: "Colour tier: the Look the host sends for this terminal",
      toolbar: {
        title: "Tier",
        icon: "paintbrush",
        items: tiers().map((t) => ({ value: t, title: t === "truecolor" ? "truecolour" : t === "mono" ? "mono" : `${t} colours` })),
        dynamicTitle: true,
      },
    },
    pack: {
      description: "Icon pack (the Ratatui references are drawn with icons off)",
      toolbar: {
        title: "Icons",
        icon: "star",
        items: [
          { value: "off", title: "icons off (as the references)" },
          { value: "unicode", title: "unicode" },
          { value: "nerd", title: "nerd" },
          { value: "emoji", title: "emoji" },
        ],
        dynamicTitle: true,
      },
    },
  },
  initialGlobals: { tier: "truecolor", pack: "off" },
  parameters: {
    layout: "padded",
    controls: { expanded: false },
    a11y: { test: "todo" },
    options: { storySort: { order: ["Introduction", "Rules", "Nodes", "Text", "Chrome", "Page", "Editor", "Media"] } },
  },
  tags: ["autodocs"],
  // Yoga's wasm is loaded once before the first story renders (shims/yoga.ts).
  loaders: [
    async () => {
      await yogaReady;
      return {};
    },
  ],
};

export default preview;
