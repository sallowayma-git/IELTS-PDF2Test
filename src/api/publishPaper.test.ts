import { beforeEach, describe, expect, it, vi } from "vitest";
import { publishItems, type PaperSpec } from "./publishClient";

// 证据层级：pure unit（IPC 被 mock）。组卷请求体与后端 PublishItemsInput.paper 同形。
const { commandMock } = vi.hoisted(() => ({ commandMock: vi.fn() }));
vi.mock("./tauriCommands", () => ({ command: commandMock }));

beforeEach(() => {
  commandMock.mockReset();
  commandMock.mockResolvedValue({ destination: "D:/nas", succeeded: [], failed: [] });
});

const spec: PaperSpec = {
  reading: { P1: "a", P2: "b", P3: "c" },
  listening: [{ part: "Part 1", itemId: "l1" }]
};

describe("publishItems 组卷参数", () => {
  it("带 paper 时请求体原样携带 reading/listening", async () => {
    await publishItems(["a", "b", "c", "l1"], "D:/nas", undefined, { paper: spec });
    const [name, args] = commandMock.mock.calls[0];
    expect(name).toBe("publish_items");
    expect(args.input.paper).toEqual(spec);
    expect(Object.keys(args.input).sort()).toEqual(["destination", "itemIds", "paper"]);
  });

  it("不带 paper 时请求体不出现该字段（普通批量发布不变）", async () => {
    await publishItems(["a"], "D:/nas");
    expect(Object.keys(commandMock.mock.calls[0][1].input).sort()).toEqual(["destination", "itemIds"]);
  });
});
