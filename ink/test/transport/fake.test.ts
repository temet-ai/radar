import { describe, expect, it } from "vitest";
import { FakeConnection } from "../../src/transport/fake.js";

describe("FakeConnection", () => {
  it("records sent messages and delivers received ones", () => {
    const c = new FakeConnection();
    const got: unknown[] = [];
    c.onMessage((m) => got.push(m));
    c.send({ a: 1 });
    c.receive({ b: 2 });
    expect(c.sent).toEqual([{ a: 1 }]);
    expect(got).toEqual([{ b: 2 }]);
  });

  it("notifies close handlers once", () => {
    const c = new FakeConnection();
    const reasons: string[] = [];
    c.onClose((r) => reasons.push(r));
    c.drop("gone");
    c.close();
    expect(reasons).toEqual(["gone"]);
  });
});
