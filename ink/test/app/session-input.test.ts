import { describe, expect, it } from "vitest";
import { Session, capabilitiesFrom } from "../../src/app/session.js";
import { FakeConnection } from "../../src/transport/fake.js";

const make = () => {
  const conn = new FakeConnection();
  const session = new Session(conn, capabilitiesFrom({}, { columns: 80, rows: 24 }));
  return { conn, session };
};
const key = { intent: "key", chord: { key: "down" } } as const;
const snap = (cursor: number) => ({ type: "snapshot", cursor, chrome: { version: 1 }, screen: null, section: null, focus: "middle" });

describe("session input causality hooks", () => {
  it("reports local input synchronously without sending, consuming a sequence or publishing state", () => {
    const { conn, session } = make();
    const order: string[] = [];
    let stateChanges = 0;
    const state = session.state;
    session.subscribe(() => stateChanges++);
    session.onInput(() => order.push("input"));
    session.input();
    order.push("local scroll");
    session.input();
    order.push("local composer edit");
    expect(order).toEqual(["input", "local scroll", "input", "local composer edit"]);
    expect(session.state).toBe(state);
    expect(stateChanges).toBe(0);
    expect(conn.sent).toEqual([]);
    session.sendIntent(key);
    expect(conn.sent).toEqual([{ type: "intent", seq: 1, cursor: 0, intent: key }]);
  });

  it("reports each exact sequence and base cursor once before the connection can reply", () => {
    const { conn, session } = make();
    conn.receive(snap(7));
    const order: unknown[] = [];
    session.onInput(() => order.push("input"));
    session.onIntent((seq, cursor) => order.push(["intent", seq, cursor]));
    const send = conn.send.bind(conn);
    conn.send = (message: any) => {
      if (message.type === "intent") {
        order.push(["send", message.seq, message.cursor]);
        conn.receive(snap(message.cursor + 1));
      }
      send(message);
    };
    session.input();
    session.sendIntent(key);
    session.sendIntent(key);
    expect(order).toEqual(["input", ["intent", 1, 7], ["send", 1, 7], ["intent", 2, 8], ["send", 2, 8]]);
    expect(session.cursor).toBe(9);
  });

  it("never replays input or intents to new listeners, including on snapshots and unrelated sends", () => {
    const { conn, session } = make();
    session.input();
    session.sendIntent(key);
    const inputs: string[] = [];
    const intents: number[][] = [];
    const stopInput = session.onInput(() => inputs.push("input"));
    const stopIntent = session.onIntent((seq, cursor) => intents.push([seq, cursor]));
    conn.receive(snap(3));
    session.resize(100, 30);
    session.sendPointer(2, 4);
    session.log("debug", "Watch");
    expect(inputs).toEqual([]);
    expect(intents).toEqual([]);
    session.input();
    session.sendIntent(key);
    expect(inputs).toEqual(["input"]);
    expect(intents).toEqual([[2, 3]]);
    stopInput();
    stopIntent();
    stopInput();
    stopIntent();
    session.input();
    session.sendIntent(key);
    expect(inputs).toEqual(["input"]);
    expect(intents).toEqual([[2, 3]]);
  });

  it("keeps the observed wire identity stable when a listener receives a newer snapshot", () => {
    const { conn, session } = make();
    conn.receive(snap(4));
    session.onIntent((seq, cursor) => {
      expect([seq, cursor]).toEqual([1, 4]);
      conn.receive(snap(5));
    });
    session.sendIntent(key);
    expect(conn.sent.at(-1)).toEqual({ type: "intent", seq: 1, cursor: 4, intent: key });
  });
});
