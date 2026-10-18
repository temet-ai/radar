// The output-request boundary is pure: fake callbacks and a caller-owned microtask queue,
// without mounting Ink, drawing frames, advancing clocks or scheduling timers.
import { afterEach, describe, expect, it, vi } from "vitest";
import { afterMotionLayout } from "../../src/motion/motion.js";

afterEach(() => { vi.unstubAllGlobals(); });

function microtasks() {
  const pending: Array<() => void> = [];
  const queue = vi.fn((callback: () => void) => { pending.push(callback); });
  vi.stubGlobal("queueMicrotask", queue);
  return {
    queue,
    pending,
    next: () => {
      expect(pending.length, "an existing output request queued a microtask").toBeGreaterThan(0);
      pending.shift()!();
    },
  };
}

describe("motion output follows committed layout", () => {
  it("defers only an existing request and coalesces rapid requests into the latest committed layout", () => {
    const tasks = microtasks();
    const order: string[] = [];
    let pane = "nav";
    const render = vi.fn(() => { order.push(`render:${pane}`); });
    const root = { onRender: render };
    const detach = afterMotionLayout(root);
    expect(tasks.queue).not.toHaveBeenCalled();
    expect(render).not.toHaveBeenCalled();

    root.onRender();
    pane = "middle";
    root.onRender();
    pane = "detail";
    root.onRender();
    expect(tasks.queue).toHaveBeenCalledTimes(1);
    expect(render).not.toHaveBeenCalled();
    order.push("child-layout", "parent-layout");
    tasks.next();
    expect(order).toEqual(["child-layout", "parent-layout", "render:detail"]);
    expect(render).toHaveBeenCalledTimes(1);
    expect(tasks.pending).toHaveLength(0);
    detach();
  });

  it.each(["normal-first", "urgent-first"] as const)("keeps one urgent output when coalescing %s requests", (order) => {
    const tasks = microtasks();
    const render = vi.fn();
    const immediate = vi.fn();
    const root = { onRender: render, onImmediateRender: immediate };
    const detach = afterMotionLayout(root);
    if (order === "normal-first") root.onRender();
    root.onImmediateRender();
    root.onRender();
    root.onImmediateRender();
    expect(tasks.queue).toHaveBeenCalledTimes(1);
    expect(render).not.toHaveBeenCalled();
    expect(immediate).not.toHaveBeenCalled();
    tasks.next();
    expect(render).not.toHaveBeenCalled();
    expect(immediate).toHaveBeenCalledTimes(1);
    expect(tasks.pending).toHaveLength(0);
    detach();
  });

  it("falls back to the existing normal callback when an urgent callback is absent", () => {
    const tasks = microtasks();
    const render = vi.fn();
    const root: { onRender?: () => void; onImmediateRender?: () => void } = { onRender: render };
    const detach = afterMotionLayout(root);
    root.onImmediateRender!();
    root.onRender!();
    expect(render).not.toHaveBeenCalled();
    expect(tasks.queue).toHaveBeenCalledTimes(1);
    tasks.next();
    expect(render).toHaveBeenCalledTimes(1);
    expect(tasks.pending).toHaveLength(0);
    detach();
    expect(root.onRender).toBe(render);
    expect(root.onImmediateRender).toBeUndefined();
  });

  it("does not reenter output or invent an extra render when output requests another layout", () => {
    const tasks = microtasks();
    let depth = 0;
    let deepest = 0;
    let calls = 0;
    const root = {
      onRender: () => {
        depth++;
        deepest = Math.max(deepest, depth);
        calls++;
        if (calls === 1) {
          root.onRender();
          root.onRender();
        }
        depth--;
      },
    };
    const detach = afterMotionLayout(root);
    root.onRender();
    expect(calls).toBe(0);
    tasks.next();
    expect(calls).toBe(1);
    expect(deepest).toBe(1);
    expect(tasks.queue).toHaveBeenCalledTimes(2);
    expect(tasks.pending).toHaveLength(1);
    tasks.next();
    expect(calls).toBe(2);
    expect(deepest).toBe(1);
    expect(tasks.pending).toHaveLength(0);
    detach();
  });

  it("cancels old queued output on detach and lets a replacement root draw only its latest layout", () => {
    const tasks = microtasks();
    const oldRender = vi.fn();
    const oldImmediate = vi.fn();
    const oldRoot = { onRender: oldRender, onImmediateRender: oldImmediate };
    const detachOld = afterMotionLayout(oldRoot);
    oldRoot.onRender();
    oldRoot.onImmediateRender();
    detachOld();
    expect(oldRoot.onRender).toBe(oldRender);
    expect(oldRoot.onImmediateRender).toBe(oldImmediate);

    let pane = "middle";
    const seen: string[] = [];
    const newRender = vi.fn(() => { seen.push(pane); });
    const newRoot = { onRender: newRender };
    const detachNew = afterMotionLayout(newRoot);
    newRoot.onRender();
    pane = "detail";
    newRoot.onRender();
    expect(tasks.queue).toHaveBeenCalledTimes(2);
    tasks.next(); // The detached root's callback is still in the queue.
    expect(oldRender).not.toHaveBeenCalled();
    expect(oldImmediate).not.toHaveBeenCalled();
    expect(newRender).not.toHaveBeenCalled();
    tasks.next();
    expect(newRender).toHaveBeenCalledTimes(1);
    expect(seen).toEqual(["detail"]);
    expect(tasks.pending).toHaveLength(0);
    detachNew();
  });

  it("restores callbacks without overwriting a newer owner of the same root", () => {
    const tasks = microtasks();
    const render = vi.fn();
    const immediate = vi.fn();
    const root = { onRender: render, onImmediateRender: immediate };
    const detach = afterMotionLayout(root);
    root.onRender();
    const replacement = vi.fn();
    const replacementImmediate = vi.fn();
    root.onRender = replacement;
    root.onImmediateRender = replacementImmediate;
    detach();
    detach();
    expect(root.onRender).toBe(replacement);
    expect(root.onImmediateRender).toBe(replacementImmediate);
    tasks.next();
    expect(render).not.toHaveBeenCalled();
    expect(immediate).not.toHaveBeenCalled();
    expect(replacement).not.toHaveBeenCalled();
    expect(replacementImmediate).not.toHaveBeenCalled();
    expect(tasks.pending).toHaveLength(0);
  });

  it("preserves immediate ordinary and urgent output while disabled", () => {
    const tasks = microtasks();
    const order: string[] = [];
    const render = vi.fn(() => { order.push("render"); });
    const immediate = vi.fn(() => { order.push("urgent"); });
    const root = { onRender: render, onImmediateRender: immediate };
    const detach = afterMotionLayout(root, () => false);
    root.onRender();
    expect(order).toEqual(["render"]);
    root.onImmediateRender();
    expect(order).toEqual(["render", "urgent"]);
    root.onRender();
    expect(order).toEqual(["render", "urgent", "render"]);
    expect(render).toHaveBeenCalledTimes(2);
    expect(immediate).toHaveBeenCalledTimes(1);
    expect(tasks.queue).not.toHaveBeenCalled();
    expect(tasks.pending).toHaveLength(0);
    detach();
  });

  it("does not replay pending full-motion output after disabled output or a later re-enable", () => {
    const tasks = microtasks();
    const render = vi.fn();
    const immediate = vi.fn();
    const root = { onRender: render, onImmediateRender: immediate };
    let enabled = true;
    const detach = afterMotionLayout(root, () => enabled);
    root.onImmediateRender();
    expect(immediate).not.toHaveBeenCalled();
    enabled = false;
    root.onRender();
    root.onImmediateRender();
    expect(render).toHaveBeenCalledTimes(1);
    expect(immediate).toHaveBeenCalledTimes(1);
    expect(tasks.queue).toHaveBeenCalledTimes(1);

    enabled = true;
    root.onRender();
    root.onRender();
    expect(tasks.queue).toHaveBeenCalledTimes(2);
    tasks.next(); // The pre-disable urgent request cannot consume or upgrade the new request.
    expect(render).toHaveBeenCalledTimes(1);
    expect(immediate).toHaveBeenCalledTimes(1);
    tasks.next();
    expect(render).toHaveBeenCalledTimes(2);
    expect(immediate).toHaveBeenCalledTimes(1);
    expect(tasks.pending).toHaveLength(0);
    detach();
  });
  it("ignores saved normal and urgent wrappers after detach, even when disabled", () => {
    const tasks = microtasks();
    const render = vi.fn(), immediate = vi.fn();
    const root = { onRender: render, onImmediateRender: immediate };
    let enabled = true;
    const detach = afterMotionLayout(root, () => enabled);
    const saved = root.onRender, savedImmediate = root.onImmediateRender;
    detach();
    saved();
    enabled = false;
    savedImmediate();
    expect(tasks.queue).not.toHaveBeenCalled();
    expect(render).not.toHaveBeenCalled();
    expect(immediate).not.toHaveBeenCalled();
  });

});
