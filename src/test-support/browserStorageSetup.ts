// Node 25+ exposes its own lazy Web Storage getters. Vitest 3 can keep those
// instead of copying jsdom's storage, yielding undefined without a Node file.
// DOM tests must use the storage belonging to their actual jsdom window.
const dom = (globalThis as typeof globalThis & { jsdom?: { window: Window } }).jsdom;
if (dom) {
  for (const key of ["localStorage", "sessionStorage"] as const) {
    Object.defineProperty(globalThis, key, {
      configurable: true,
      writable: true,
      value: dom.window[key]
    });
  }
}
