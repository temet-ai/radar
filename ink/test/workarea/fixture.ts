import type { ScreenLike } from "../../src/workarea/model.js";

const seg = (text: string) => [{ text }];
const ext = (kind: string, data: object, lines: string[]) => ({
  node: "extension",
  capability: `work.${kind}`,
  version: 1,
  data,
  fallback: { node: "document", lines: lines.map(seg) },
});

/** A hand-written Work screen carrying every work-area section; a pending question lives in the agent area. */
export function workScreen(opts: { pending?: boolean } = {}): ScreenLike {
  return {
    scope: "work",
    panes: [
      { pane: "map", title: "Map", body: ext("map", {}, ["GG-12 ← MR !34", "GG-12 → branch ink-ui"]) },
      { pane: "timeline", title: "Timeline", body: ext("timeline", { status: "recording" }, ["10:02 plan agreed", "10:20 agent turn"]) },
      { pane: "tabs", title: "Overview", body: ext("tabs", {}, ["Item GG-12: Ink work area"]) },
      {
        pane: "agent",
        title: "Agent",
        body: ext("agent", { pending: opts.pending === true, status: "running" }, [
          opts.pending ? "Approve push to origin?" : "editing ink/src",
        ]),
      },
      { pane: "rail", title: "Next", body: ext("rail", {}, ["Review changes (reads)"]) },
    ],
  };
}
