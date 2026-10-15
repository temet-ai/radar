// What the session tells the app, and how often (T7, speed). Every notification is a React commit and an Ink frame
// (about 20-60 ms at 160x40 on a busy machine), so a burst of messages is one notification. (An identical snapshot still
// notifies: a snapshot is also the host's answer to what the renderer predicted, see app.test.tsx.)
import { describe, expect, it } from "vitest";
import { QUESTION_WAIT_MS, Session, capabilitiesFrom } from "../../src/app/session.js";
import { FakeConnection } from "../../src/transport/fake.js";

const make = () => {
  const conn = new FakeConnection();
  const session = new Session(conn, capabilitiesFrom({}, { columns: 160, rows: 40 }), () => {}, true);
  let notified = 0;
  session.subscribe(() => notified++);
  return { conn, session, count: () => notified };
};
const snap = (cursor: number, selected = 0) => ({ type: "snapshot", cursor, chrome: { version: 1 }, screen: { panes: [{ body: { selected } }] }, section: null, focus: "middle" });
const flush = () => new Promise<void>((r) => setTimeout(r, 0));

describe("session notifications", () => {
  it("a burst of messages in one read notifies once", async () => {
    const { conn, count } = make();
    conn.receive({ type: "notice", text: "saved" });
    conn.receive(snap(1));
    conn.receive(snap(2, 1));
    await flush();
    expect(count()).toBe(1);
  });

  it("a snapshot that differs notifies", async () => {
    const { conn, session, count } = make();
    conn.receive(snap(1));
    await flush();
    const before = count();
    conn.receive(snap(2, 1));
    await flush();
    expect(count()).toBe(before + 1);
    expect(session.state.snapshot?.screen.panes[0].body.selected).toBe(1);
  });

  // A right-click's menu: the host asks the question, then sends the snapshot that carries it a few ms later (in another
  // read). Drawing each was two full frames for one click (the first up to 25 ms at 280x75).
  const question = (id: number, cursor: number) => ({ type: "question", id, cursor, menu: { id: "keys", title: "row", question: true, items: [] } });
  it("a question and the snapshot after it in another read notify once, when the snapshot is there", async () => {
    const { conn, session, count } = make();
    conn.receive(snap(1));
    await flush();
    const before = count();
    conn.receive(question(1, 2));
    expect(session.state.questions).toHaveLength(1); // the keys after it are the question's at once
    await new Promise((r) => setTimeout(r, 5));
    expect(count()).toBe(before);
    conn.receive(snap(3));
    await flush();
    expect(count()).toBe(before + 1);
    await new Promise((r) => setTimeout(r, QUESTION_WAIT_MS + 10));
    expect(count()).toBe(before + 1); // the wait was dropped with the snapshot: no second notification
  });

  it("a question with no snapshot after it is drawn after QUESTION_WAIT_MS", async () => {
    const { conn, count } = make();
    conn.receive(snap(1));
    await flush();
    const before = count();
    conn.receive(question(1, 2));
    await flush();
    expect(count()).toBe(before);
    await new Promise((r) => setTimeout(r, QUESTION_WAIT_MS + 10));
    expect(count()).toBe(before + 1);
  });

  it("anything else that arrives while a question waits draws at once, the question with it", async () => {
    const { conn, session, count } = make();
    conn.receive(snap(1));
    await flush();
    const before = count();
    conn.receive(question(1, 2));
    conn.receive({ type: "notice", text: "saved" });
    await flush();
    expect(count()).toBe(before + 1);
    expect(session.state.questions).toHaveLength(1);
    await new Promise((r) => setTimeout(r, QUESTION_WAIT_MS + 10));
    expect(count()).toBe(before + 1);
  });

  it("the state is current at once, before the notification", () => {
    const { conn, session } = make();
    conn.receive(snap(1));
    expect(session.cursor).toBe(1);
  });
});
