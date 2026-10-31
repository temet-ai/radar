import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { freezeClock, mount, tick, until } from "../app/harness.js";
import { parseAnsi } from "../nodes/ansi.js";
import { STATES } from "./states.js";
import type { DockView } from "../../src/dock/model.js";

// Keys and the pointer in the dock, through the whole App: every interaction is an Intent::Dock, nothing is kept in Ink
// but the text being written.
let unfreeze = () => {};
beforeAll(() => {
  unfreeze = freezeClock();
});
afterAll(() => unfreeze());
let live: Array<() => void> = [];
afterEach(() => {
  live.forEach((f) => f());
  live = [];
});

const dir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../fixtures");
const load = (f: string) => JSON.parse(fs.readFileSync(path.join(dir, f), "utf8"));
const plain = (frame: string) => parseAnsi(frame).map((row) => row.map((c) => c.ch).join("").trimEnd());

async function open(dock: DockView, cols = 160, rows = 40) {
  const m = mount(cols, rows);
  live.push(m.stop);
  m.welcome();
  m.snapshot({ chrome: load(`host_chrome_${cols}x${rows}.json`), screen: { ...load(`gitlab_repos_${cols}x${rows}.json`), dock } }, 1);
  await tick(40);
  const docks = () => m.intents().map((i) => i.intent).filter((i) => i.intent === "dock");
  return { m, docks };
}

describe("dock input", () => {
  it("types into the composer and sends it, one intent", async () => {
    const { m, docks } = await open(STATES.empty!);
    m.type("hi there");
    await until(() => plain(m.frame()).join("\n").includes("> hi there"));
    m.type("\r");
    await until(() => docks().length > 0);
    expect(docks()).toEqual([{ intent: "dock", dock: "send", text: "hi there" }]);
  });

  it("a slash opens the command menu with what each does; the arrows and tab choose, ⏎ sends the whole word", async () => {
    const commands = [
      { name: "compact", detail: "summarise the conversation" },
      { name: "context", detail: "show context usage" },
      { name: "model", detail: "<model>  choose the model" },
    ];
    const dock: DockView = { ...STATES.empty!, composer: { ...STATES.empty!.composer!, commands } };
    const { m, docks } = await open(dock);
    m.type("/");
    await until(() => plain(m.frame()).join("\n").includes("/compact  summarise the conversation"));
    const text = plain(m.frame()).join("\n");
    expect(text).toContain("/context  show context usage");
    expect(text).toContain("/model    <model>  choose the model");
    m.type("\x1b[B");
    await tick(40);
    m.type("\t");
    await until(() => plain(m.frame()).join("\n").includes("> /context "));
    m.type("\r");
    await until(() => docks().length > 0);
    expect(docks()).toEqual([{ intent: "dock", dock: "send", text: "/context" }]);
  });

  it("a click on a row of the slash menu runs that command, as ⏎ on it does", async () => {
    const commands = [
      { name: "compact", detail: "summarise the conversation" },
      { name: "context", detail: "show context usage" },
      { name: "model", detail: "<model>  choose the model" },
    ];
    for (const transcript of [[], STATES.streaming!.transcript ?? []]) {
      const dock: DockView = { ...STATES.empty!, transcript, composer: { ...STATES.empty!.composer!, commands } };
      const { m, docks } = await open(dock);
      m.type("/co");
      await until(() => plain(m.frame()).join("\n").includes("/context  show context usage"));
      const rows = plain(m.frame());
      const y = rows.findIndex((r) => r.includes("/context  show context usage"));
      const x = rows[y]!.indexOf("show context");
      // SGR press and release at the 0-based cell
      m.type(`\x1b[<0;${x + 1};${y + 1}M\x1b[<0;${x + 1};${y + 1}m`);
      await until(() => docks().some((d: any) => d.dock === "send"));
      expect(docks().filter((d: any) => d.dock === "send")).toEqual([{ intent: "dock", dock: "send", text: "/context" }]);
      await until(() => !plain(m.frame()).join("\n").includes("/context  show context usage"));
    }
  });

  it("takes keys written together in order: text, then enter", async () => {
    const { m, docks } = await open(STATES.empty!);
    m.type("ab\rcd\r");
    await until(() => docks().length >= 1);
    await tick(60);
    expect(docks().map((d: any) => d.text)).toContain("ab");
  });

  it("answers an approval with a and d, and stops with ctrl-c", async () => {
    const a = await open(STATES.approval!);
    a.m.type("a");
    await until(() => a.docks().length > 0);
    expect(a.docks()[0]).toEqual({ intent: "dock", dock: "approve", id: "p1" });
    const d = await open(STATES.approval!);
    d.m.type("d");
    await until(() => d.docks().length > 0);
    expect(d.docks()[0]).toEqual({ intent: "dock", dock: "deny", id: "p1" });
    const s = await open(STATES.streaming!);
    s.m.type("\x03");
    await until(() => s.docks().length > 0);
    expect(s.docks()[0]).toEqual({ intent: "dock", dock: "stop" });
  });

  it("esc gives the keys back to the host", async () => {
    const { m, docks } = await open(STATES.empty!);
    m.type("\x1b");
    await until(() => docks().length > 0, 1000);
    expect(docks()[0]).toEqual({ intent: "dock", dock: "blur" });
  });

  it("leaves keys to the pane while the dock does not have them", async () => {
    const { m, docks } = await open(STATES.unfocused!);
    m.type("hi");
    await tick(80);
    expect(docks()).toEqual([]);
  });

  it("a click on a tab switches to it and a drag of the top edge resizes", async () => {
    const { m, docks } = await open(STATES["two-tabs"]!);
    const rows = plain(m.frame());
    const y = rows.findIndex((r) => r.includes("agents")) + 1;
    const x = rows[y]!.indexOf("mr-1234") + 1;
    m.type(`\x1b[<0;${x + 1};${y + 1}M\x1b[<0;${x + 1};${y + 1}m`);
    await until(() => docks().length > 0);
    expect(docks()[0]).toEqual({ intent: "dock", dock: "switch_tab", id: "s2" });
    // press on the top edge, move up five rows: the dock is drawn taller, and the host hears the height once, on release
    const edge = y;
    const before = plain(m.frame()).findIndex((r) => r.includes("agents"));
    m.type(`\x1b[<0;40;${edge}M`);
    m.type(`\x1b[<32;40;${edge - 3}M`);
    m.type(`\x1b[<32;40;${edge - 5}M`);
    await until(() => plain(m.frame()).findIndex((r) => r.includes("agents")) < before);
    expect(docks().some((d: any) => d.dock === "resize")).toBe(false);
    m.type(`\x1b[<0;40;${edge - 5}m`);
    await until(() => docks().some((d: any) => d.dock === "resize"));
    const resizes = docks().filter((d: any) => d.dock === "resize") as any[];
    expect(resizes.length).toBe(1);
    expect(resizes[0].rows).toBeGreaterThan(7);
  });

  it("the top edge dragged past the top stops where the pane above keeps its room, and the host keeps that height", async () => {
    const { m, docks } = await open(STATES["two-tabs"]!);
    const edge = plain(m.frame()).findIndex((r) => r.includes("agents")) + 1;
    m.type(`\x1b[<0;40;${edge}M`);
    m.type(`\x1b[<32;40;1M`);
    await until(() => plain(m.frame()).findIndex((r) => r.includes("agents")) < edge - 1);
    const rows = plain(m.frame());
    const top = rows.findIndex((r) => r.includes("agents"));
    // a move with no button down outside the window: the release happened where radar did not hear it
    m.type(`\x1b[<35;40;1M`);
    await until(() => docks().some((d: any) => d.dock === "resize"));
    // the dock runs from its top line to the row above the status line: the height drawn is the height kept
    expect(docks().filter((d: any) => d.dock === "resize")).toEqual([{ intent: "dock", dock: "resize", rows: rows.length - 1 - top }]);
  });

  describe("closing a tab", () => {
    const tabRow = (m: Awaited<ReturnType<typeof open>>["m"]) => {
      const rows = plain(m.frame());
      const y = rows.findIndex((r) => r.includes("agents")) + 1;
      return { row: rows[y]!, y };
    };

    it("shows a close mark on every tab, and a click on one closes that tab only", async () => {
      const { m, docks } = await open(STATES["two-tabs"]!);
      const { row, y } = tabRow(m);
      expect(row.match(/×/g)).toHaveLength(2);
      // the second tab's mark (not the active one's)
      const x = row.lastIndexOf("×");
      m.type(`\x1b[<0;${x + 1};${y + 1}M\x1b[<0;${x + 1};${y + 1}m`);
      await until(() => docks().some((d: any) => d.dock === "close_tab"));
      expect(docks().filter((d: any) => d.dock !== "focus")).toEqual([{ intent: "dock", dock: "close_tab", id: "s2" }]);
    });

    it("a middle click anywhere on a tab closes it", async () => {
      const { m, docks } = await open(STATES["two-tabs"]!);
      const { row, y } = tabRow(m);
      const x = row.indexOf("mr-1234");
      m.type(`\x1b[<1;${x + 1};${y + 1}M`);
      await until(() => docks().some((d: any) => d.dock === "close_tab"));
      expect(docks().filter((d: any) => d.dock !== "focus")).toEqual([{ intent: "dock", dock: "close_tab", id: "s2" }]);
    });

    it("alt-w closes the active tab and the status line says so", async () => {
      const { m, docks } = await open(STATES.empty!);
      expect(plain(m.frame()).join("\n")).toContain("alt-w close");
      m.type("\x1bw");
      await until(() => docks().length > 0);
      expect(docks()).toEqual([{ intent: "dock", dock: "close_tab", id: "s1" }]);
    });

    it.each([
      ["kitty CSI-u", "\x1b[119;3u"],
      ["modifyOtherKeys", "\x1b[27;3;119~"],
    ])("alt-w written as %s closes the active tab the same way", async (_n, bytes) => {
      const { m, docks } = await open(STATES.empty!);
      m.type(bytes);
      await until(() => docks().length > 0);
      expect(docks()).toEqual([{ intent: "dock", dock: "close_tab", id: "s1" }]);
    });

    it("alt-w closes the tab while the dock is shown but the page has the keys", async () => {
      const { m, docks } = await open(STATES.unfocused!);
      m.type("\x1bw");
      await until(() => docks().length > 0);
      expect(docks()).toEqual([{ intent: "dock", dock: "close_tab", id: "s1" }]);
    });

    it("alt-w on a dock with no session hides it, so close leaves no empty window", async () => {
      const { m, docks } = await open(STATES["no-sessions"]!);
      m.type("\x1b[119;3u");
      await until(() => docks().length > 0);
      expect(docks()).toEqual([{ intent: "dock", dock: "hide" }]);
    });

    it("a close mark clicked on an unfocused dock closes and gives it the keys", async () => {
      const { m, docks } = await open(STATES.unfocused!);
      const { row, y } = tabRow(m);
      m.type(`\x1b[<0;${row.indexOf("×") + 1};${y + 1}M`);
      await until(() => docks().length > 1);
      expect(docks()).toEqual([
        { intent: "dock", dock: "close_tab", id: "s1" },
        { intent: "dock", dock: "focus" },
      ]);
    });
  });

  it("a click on approve once, approve for session, always allow or deny answers the request, as a, s, l and d do", async () => {
    // As the host sends a shell request now: always allow offered beside the rest.
    const approval: DockView = {
      ...STATES.approval!,
      transcript: STATES.approval!.transcript.map((i) =>
        i.item === "permission" ? { ...i, actions: ["approve_once", "approve_for_session", "always_allow", "deny", "view_diff"] } : i,
      ),
    };
    for (const [word, dock] of [["approve once", "approve"], ["approve for session", "approve_for_session"], ["always allow", "always_allow"], ["deny", "deny"]] as const) {
      const { m, docks } = await open(approval);
      const rows = plain(m.frame());
      const y = rows.findIndex((r) => r.includes("approve once") && r.includes("deny"));
      const x = rows[y]!.indexOf(word);
      m.type(`\x1b[<0;${x + 2};${y + 1}M\x1b[<0;${x + 2};${y + 1}m`);
      await until(() => docks().length > 0);
      expect(docks()[0]).toEqual({ intent: "dock", dock, id: "p1" });
    }
  });

  it("a click on a question's choice answers it, and on decline declines it", async () => {
    const a = await open(STATES.question!);
    let rows = plain(a.m.frame());
    let y = rows.findIndex((r) => r.includes("2 patch"));
    a.m.type(`\x1b[<0;${rows[y]!.indexOf("patch") + 1};${y + 1}M\x1b[<0;${rows[y]!.indexOf("patch") + 1};${y + 1}m`);
    await until(() => a.docks().length > 0);
    expect(a.docks()[0]).toEqual({ intent: "dock", dock: "answer", id: "q1", question: "0", choices: ["patch"] });
    const d = await open(STATES.question!);
    rows = plain(d.m.frame());
    y = rows.findIndex((r) => r.includes("esc decline") && r.includes("choose"));
    d.m.type(`\x1b[<0;${rows[y]!.indexOf("decline") + 1};${y + 1}M\x1b[<0;${rows[y]!.indexOf("decline") + 1};${y + 1}m`);
    await until(() => d.docks().length > 0);
    expect(d.docks()[0]).toEqual({ intent: "dock", dock: "decline", id: "q1" });
  });

  it("a number answers a waiting question from the keys", async () => {
    const { m, docks } = await open(STATES.question!);
    m.type("1");
    await until(() => docks().length > 0);
    expect(docks()[0]).toEqual({ intent: "dock", dock: "answer", id: "q1", question: "0", choices: ["revert"] });
  });

  it("a click on a folded tool call opens it", async () => {
    const { m, docks } = await open(STATES["tool-folded"]!);
    const rows = plain(m.frame());
    const y = rows.findIndex((r) => r.includes("git log -3 --oneline"));
    const x = rows[y]!.indexOf("git log");
    m.type(`\x1b[<0;${x + 1};${y + 1}M\x1b[<0;${x + 1};${y + 1}m`);
    await until(() => docks().length > 0);
    expect(docks()[0]).toEqual({ intent: "dock", dock: "fold", item: "t1", open: true });
  });

  it("the wheel over the dock scrolls its transcript, not the pane", async () => {
    const many: DockView = { ...STATES.empty!, transcript: Array.from({ length: 40 }, (_, i) => ({ item: "notice" as const, id: `n${i}`, text: `line ${i}` })) };
    const { m, docks } = await open(many);
    const rows = plain(m.frame());
    const y = rows.findIndex((r) => r.includes("agents")) + 3;
    expect(plain(m.frame()).join("\n")).toContain("line 39");
    m.type(`\x1b[<64;40;${y + 1}M`);
    await until(() => !plain(m.frame()).join("\n").includes("line 39"));
    expect(plain(m.frame()).join("\n")).toContain("line 36");
    expect(m.intents().filter((i: any) => i.intent.intent === "key")).toEqual([]);
    expect(docks()).toEqual([]);
  });
});

describe("dock composer after a send", () => {
  it("a send that creates the tab leaves the composer empty: the next message is exactly what was typed", async () => {
    const { m, docks } = await open({ ...STATES.empty!, tabs: [], active: null, composer: { can_send: true, can_stop: false } });
    m.type("say hi");
    await until(() => plain(m.frame()).join("\n").includes("> say hi"));
    m.type("\r");
    await until(() => docks().length === 1);
    // the host creates the tab; its snapshot may carry stale or empty composer data
    m.snapshot({ chrome: load("host_chrome_160x40.json"), screen: { ...load("gitlab_repos_160x40.json"), dock: STATES.empty! } }, 2);
    await tick(60);
    m.type("x");
    await until(() => plain(m.frame()).join("\n").includes("> x"));
    m.type("\r");
    await until(() => docks().length === 2);
    expect(docks().map((d: any) => d.text)).toEqual(["say hi", "x"]);
  });

  it("the same when the host's snapshot arrives between the keys of the send", async () => {
    const { m, docks } = await open(STATES.empty!);
    m.type("one");
    await tick(30);
    m.type("\r");
    for (let v = 2; v < 6; v++) m.snapshot({ chrome: load("host_chrome_160x40.json"), screen: { ...load("gitlab_repos_160x40.json"), dock: STATES.empty! } }, v);
    m.type("two\r");
    await until(() => docks().length === 2, 1500);
    expect(docks().map((d: any) => d.text)).toEqual(["one", "two"]);
  });

  it("after the dock loses the keys and gets them back, the next message is only what is typed then", async () => {
    const { m, docks } = await open(STATES.empty!);
    const snap = (dock: DockView, v: number) => m.snapshot({ chrome: load("host_chrome_160x40.json"), screen: { ...load("gitlab_repos_160x40.json"), dock } }, v);
    m.type("one\r");
    await until(() => docks().length === 1);
    snap({ ...STATES.empty!, focused: false }, 2);
    await tick(40);
    snap({ ...STATES.empty!, focused: true }, 3);
    await tick(40);
    m.type("two\r");
    await until(() => docks().length >= 2, 1500);
    expect(docks().map((d: any) => d.text ?? d.dock)).toEqual(["one", "two"]);
  });

  it("keys written after the dock key hides the dock go to the host, not into the hidden composer", async () => {
    const { m, docks } = await open(STATES.empty!);
    m.type("`ab");
    await until(() => m.intents().length >= 6, 1500);
    expect(docks()).toEqual([{ intent: "dock", dock: "hide" }]);
    const noticeInput = { intent: "notice_input", escape: false, through_id: load("host_chrome_160x40.json").notices.through_id };
    expect(m.intents().map((i) => i.intent)).toEqual([
      noticeInput,
      { intent: "dock", dock: "hide" },
      noticeInput,
      { intent: "key", chord: { key: { char: "a" } } },
      noticeInput,
      { intent: "key", chord: { key: { char: "b" } } },
    ]);
  });

  it("snapshots interleaved with every key of two messages do not bring the first back", async () => {
    const { m, docks } = await open(STATES.empty!);
    let v = 2;
    const snap = () => m.snapshot({ chrome: load("host_chrome_160x40.json"), screen: { ...load("gitlab_repos_160x40.json"), dock: STATES.empty! } }, v++);
    for (const k of ["a", "b", "\r", "c", "d", "\r"]) {
      m.type(k);
      snap();
      snap();
    }
    await until(() => docks().length === 2, 1500);
    expect(docks().map((d: any) => d.text)).toEqual(["ab", "cd"]);
  });
});
