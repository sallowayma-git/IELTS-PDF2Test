import { afterEach, describe, expect, it, vi } from "vitest";
import { setPublishIntent, takePublishIntent } from "./publishIntent";

// 证据层级：pure unit（计划 §19.1 层 1）。
// publishIntent 是一次性 sessionStorage 意图（题库承接被退休的 /jobs/new 与 /export 入口）。
// 用替身 sessionStorage，不依赖真实浏览器环境。

function fakeStorage(): Storage {
  const map = new Map<string, string>();
  return {
    getItem: (key: string) => (map.has(key) ? (map.get(key) as string) : null),
    setItem: (key: string, value: string) => {
      map.set(key, String(value));
    },
    removeItem: (key: string) => {
      map.delete(key);
    },
    clear: () => map.clear(),
    key: (index: number) => [...map.keys()][index] ?? null,
    get length() {
      return map.size;
    }
  } as Storage;
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("publishIntent", () => {
  it("没有写入时返回 undefined", () => {
    vi.stubGlobal("window", { sessionStorage: fakeStorage() });
    expect(takePublishIntent()).toBeUndefined();
  });

  it("写入后可读回，并且只读一次（take 语义）", () => {
    vi.stubGlobal("window", { sessionStorage: fakeStorage() });
    setPublishIntent({ mode: "nas-library", jobId: "job-1" });
    expect(takePublishIntent()).toEqual({ mode: "nas-library", jobId: "job-1" });
    expect(takePublishIntent()).toBeUndefined();
  });

  it("支持 writing-library 意图", () => {
    vi.stubGlobal("window", { sessionStorage: fakeStorage() });
    setPublishIntent({ mode: "writing-library" });
    expect(takePublishIntent()).toEqual({ mode: "writing-library" });
  });

  it("写入会覆盖上一条意图", () => {
    vi.stubGlobal("window", { sessionStorage: fakeStorage() });
    setPublishIntent({ mode: "writing-library" });
    setPublishIntent({ mode: "nas-library", jobId: "job-2" });
    expect(takePublishIntent()).toEqual({ mode: "nas-library", jobId: "job-2" });
  });

  it("损坏的 JSON 不抛出，返回 undefined 并清除该键", () => {
    const storage = fakeStorage();
    vi.stubGlobal("window", { sessionStorage: storage });
    storage.setItem("ielts-author-studio.publish-intent", "{not json");
    expect(takePublishIntent()).toBeUndefined();
    expect(storage.getItem("ielts-author-studio.publish-intent")).toBeNull();
  });
});
