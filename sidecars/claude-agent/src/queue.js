// A push-fed async iterable: the streaming-input side of one Claude Code session.
// Turns are pushed as they are sent; `end()` finishes the iteration.
export class AsyncQueue {
  #items = [];
  #waiting = null;
  #ended = false;

  push(item) {
    if (this.#ended) return false;
    if (this.#waiting) {
      const resolve = this.#waiting;
      this.#waiting = null;
      resolve({ value: item, done: false });
    } else {
      this.#items.push(item);
    }
    return true;
  }

  end() {
    this.#ended = true;
    if (this.#waiting) {
      const resolve = this.#waiting;
      this.#waiting = null;
      resolve({ value: undefined, done: true });
    }
  }

  [Symbol.asyncIterator]() {
    return {
      next: () => {
        if (this.#items.length > 0) {
          return Promise.resolve({ value: this.#items.shift(), done: false });
        }
        if (this.#ended) return Promise.resolve({ value: undefined, done: true });
        return new Promise((resolve) => {
          this.#waiting = resolve;
        });
      },
      return: () => {
        this.end();
        return Promise.resolve({ value: undefined, done: true });
      },
    };
  }
}
