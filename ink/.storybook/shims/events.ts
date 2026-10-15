// node:events for the browser: the part of EventEmitter Ink, its streams and radar's stubs use.
type Listener = (...args: any[]) => void;

export class EventEmitter {
  private handlers = new Map<string | symbol, Listener[]>();

  on(name: string | symbol, f: Listener): this {
    this.handlers.set(name, [...(this.handlers.get(name) ?? []), f]);
    return this;
  }
  addListener(name: string | symbol, f: Listener): this {
    return this.on(name, f);
  }
  prependListener(name: string | symbol, f: Listener): this {
    this.handlers.set(name, [f, ...(this.handlers.get(name) ?? [])]);
    return this;
  }
  once(name: string | symbol, f: Listener): this {
    const once: Listener = (...args) => {
      this.off(name, once);
      f(...args);
    };
    return this.on(name, once);
  }
  off(name: string | symbol, f: Listener): this {
    this.handlers.set(
      name,
      (this.handlers.get(name) ?? []).filter((g) => g !== f),
    );
    return this;
  }
  removeListener(name: string | symbol, f: Listener): this {
    return this.off(name, f);
  }
  removeAllListeners(name?: string | symbol): this {
    if (name === undefined) this.handlers.clear();
    else this.handlers.delete(name);
    return this;
  }
  emit(name: string | symbol, ...args: unknown[]): boolean {
    const fs = this.handlers.get(name) ?? [];
    for (const f of [...fs]) f(...args);
    return fs.length > 0;
  }
  listenerCount(name: string | symbol): number {
    return (this.handlers.get(name) ?? []).length;
  }
  listeners(name: string | symbol): Listener[] {
    return [...(this.handlers.get(name) ?? [])];
  }
  setMaxListeners(): this {
    return this;
  }
  getMaxListeners(): number {
    return 0;
  }
}

export default EventEmitter;
