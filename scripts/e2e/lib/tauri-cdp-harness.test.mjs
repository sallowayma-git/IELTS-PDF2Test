// CDP 会话「断线重连必须可见」的自测（层 1：纯逻辑，不驱动真实应用）。
//
// 背景（2026-09-23 第二轮复核 F3）：
//   `TauriCdpSession.evaluate` 在连接断开时会自动重新附着 page target 并重试一次，
//   但**只写日志**——报告里一个字都没有。WebView2 重建 page target 是常态，所以
//   一次真实的 renderer 崩溃 + 重建会被静默掩盖成「一切正常」：脚本接到一个刚重载
//   的页面上继续跑，断言全过，读者却不知道中间断过。
//
// 本文件把三件事固定下来：
//   1. 每次重连都要留下一条记录（时间、发生在哪一步、为什么断）；
//   2. 发生重连的运行，verdict 至少降级成 `passed_with_warnings`（要把它当成可接受
//      的脚本必须**显式声明** `acceptReattaches`）；
//   3. 显式关掉重连（`reattach:false`）时必须如实抛出，且不产生记录——不能既不重连
//      又不报错。

import { describe, expect, it } from "vitest";

import {
  applyReattachPolicy,
  createStepRecorder,
  summarizeReattaches,
  TauriCdpSession,
} from "./tauri-cdp-harness.mjs";

/** 一个只会在 `send` 上撒谎的最简连接：第一次调用按脚本失败，之后成功。 */
function fakeConnection({ failWith = null, failTimes = 0 } = {}) {
  let failures = failTimes;
  return {
    closed: false,
    sendCalls: [],
    async send(method, params, timeoutMs) {
      this.sendCalls.push({ method, params, timeoutMs });
      if (failures > 0) {
        failures -= 1;
        this.closed = true;
        throw new Error(failWith ?? "CDP 连接已关闭（renderer 或应用退出）");
      }
      this.closed = false;
      return { result: { value: { ok: true, method } } };
    },
    close() {},
  };
}

function makeSession({ connection, exitCode = null } = {}) {
  return new TauriCdpSession({
    connection,
    child: { kill() {} },
    devtoolsPort: 1,
    runDir: "unused",
    browserArgs: "",
    appOutput: () => "",
    exitCode: () => exitCode,
    log: () => {},
  });
}

describe("CDP 会话重连可见性", () => {
  it("连接断开后重连，报告里出现一条记录，并带上发生它的步骤名", async () => {
    const dead = fakeConnection({ failTimes: 1 });
    const session = makeSession({ connection: dead });
    const live = fakeConnection();
    // 真机上这一步是 HTTP /json/list + 建 WS；自测只关心「重连发生过」这件事本身。
    session.attachToPageTarget = async () => {
      session.cdp = live;
      return live;
    };

    const recorder = createStepRecorder({ session, artifactsDir: "unused" });
    await recorder.run("assume-nothing-broke", async () => {
      const value = await session.evaluate("1 + 1");
      // 重连后重试成功：调用方拿到的是**重试之后**的结果，不是异常。
      return value;
    });

    const step = recorder.steps.at(-1);
    expect(step.status).toBe("passed");
    expect(session.reattaches).toHaveLength(1);
    expect(session.reattaches[0].step).toBe("assume-nothing-broke");
    expect(String(session.reattaches[0].reason)).toContain("CDP 连接已关闭");
    expect(typeof session.reattaches[0].at).toBe("string");

    const summary = summarizeReattaches(session);
    expect(summary.count).toBe(1);
    expect(summary.entries).toHaveLength(1);
    // 默认策略：不接受重连。
    expect(summary.policy).toBe("warn-on-reattach");
  });

  it("步骤之外的断线也要记下来，不能因为不在步骤里就丢掉", async () => {
    const dead = fakeConnection({ failTimes: 1 });
    const session = makeSession({ connection: dead });
    session.attachToPageTarget = async () => {
      session.cdp = fakeConnection();
      return session.cdp;
    };

    await session.evaluate("1 + 1");

    expect(session.reattaches).toHaveLength(1);
    expect(session.reattaches[0].step).toBeNull();
  });

  it("发生过重连的运行 verdict 至少降为「通过但有警告」", () => {
    const entries = [{ at: "2026-09-23T00:00:00.000Z", step: "s", reason: "CDP 连接已关闭" }];

    const warned = applyReattachPolicy("passed", entries);
    expect(warned.verdict).toBe("passed_with_warnings");
    expect(warned.downgraded).toBe(true);
    expect(String(warned.warning)).toContain("重连");

    // 显式声明「重连可接受」的脚本才拿得到干净的 passed。
    const accepted = applyReattachPolicy("passed", entries, { acceptReattaches: true });
    expect(accepted.verdict).toBe("passed");
    expect(accepted.downgraded).toBe(false);

    // 没重连 => 一切照旧。
    expect(applyReattachPolicy("passed", []).verdict).toBe("passed");
    // 已经失败的运行不因为重连「降级」成警告：失败就是失败。
    expect(applyReattachPolicy("failed", entries).verdict).toBe("failed");
    expect(applyReattachPolicy("cannot-run", entries).verdict).toBe("cannot-run");
  });

  it("显式关掉重连时必须抛出，且不产生记录", async () => {
    const dead = fakeConnection({ failTimes: 1 });
    const session = makeSession({ connection: dead });

    await expect(session.evaluate("1 + 1", { reattach: false })).rejects.toThrow("CDP 连接已关闭");
    expect(session.reattaches).toHaveLength(0);
  });

  it("非断线错误不许被当成断线重试", async () => {
    const connection = fakeConnection();
    connection.send = async () => {
      throw new Error("页面脚本异常：boom");
    };
    const session = makeSession({ connection });
    let reattached = 0;
    session.attachToPageTarget = async () => {
      reattached += 1;
      return connection;
    };

    await expect(session.evaluate("throw new Error('boom')")).rejects.toThrow("页面脚本异常");
    expect(reattached).toBe(0);
    expect(session.reattaches).toHaveLength(0);
  });
});
