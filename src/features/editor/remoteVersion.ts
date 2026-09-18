/**
 * 「远端权威稿版本推进了，本地该怎么办」的判定。
 *
 * 抽成纯函数的原因（与 `conflictRecovery.ts` 同一套路）：这是一条**会被误解的规则**，
 * 而它的两种错法都很贵——
 *   - 一律重拉：用户正在打字时后台事件到来，草稿被服务端内容覆盖，输入凭空消失；
 *   - 一律不重拉：云端自主修复了内容，编辑器永远不知道，用户继续在过期题面上改，
 *     直到某次保存撞上 `EDIT_VERSION_CONFLICT` 才发现。
 * 所以判定本身必须可被单独断言，而不是埋在 hook 的 if 里。
 */

export type RemoteVersionAction = "ignore" | "reload" | "defer";

/**
 * 已推迟的远端刷新记录。
 *
 * `version` 是当时看到的远端版本号；后端没给（`null`）时保持 `undefined`，表示
 * 「确实有远端变更，但不知道它是什么版本」——保存完成后仍需重拉，不能因为拿不到
 * 版本号就把这次变更忘掉。
 */
export interface DeferredRemoteRefresh {
  version?: number;
}

/**
 * 收到一条远端版本通知时的处置。
 *
 * 规则：
 *   - 远端版本**不高于**本地已知版本 → `ignore`。这是自己刚保存引起的回声；
 *     重拉等于把刚提交的稿再取一遍，白费一次往返，还会重置本地撤销栈的基线。
 *   - 确实更新了，且本地**有未保存修改** → `defer`。此刻重拉会覆盖用户正在编辑的
 *     内容；记录下来，等保存排空后再读。
 *   - 确实更新了，本地干净 → `reload`。
 *   - 远端版本未知（`null`/`undefined`）→ 按「可能有变更」保守处理：脏则 `defer`，
 *     干净则 `reload`。宁可多刷一次，也不能把一次真实变更漏掉。
 */
export function decideRemoteVersionAction(input: {
  /** 事件携带的远端版本号；后端读不到时为 `null`/`undefined`。 */
  incoming: number | null | undefined;
  /** 本地已知的权威稿版本号。 */
  current: number;
  /** 尚未保存到服务端的改动数量。 */
  unsavedCount: number;
}): RemoteVersionAction {
  const known = typeof input.incoming === "number" && Number.isFinite(input.incoming);
  if (known && (input.incoming as number) <= input.current) return "ignore";
  return input.unsavedCount > 0 ? "defer" : "reload";
}

/**
 * 保存循环排空后，此前被推迟的远端变更是否还需要读取。
 *
 * `current` 传入保存完成后的本地版本号：如果保存本身已经把版本推到（或越过）当时
 * 记录的远端版本，说明这次保存就是在最新基线上完成的，那次变更已经在稿里了，
 * 不必再拉一次。
 *
 * 返回 `true` 时调用方应重新加载；调用方**必须**在读取后清空推迟记录，否则同一次
 * 变更会在每次保存后重复触发刷新。
 */
export function shouldApplyDeferredRemoteRefresh(
  deferred: DeferredRemoteRefresh | undefined,
  current: number
): boolean {
  if (!deferred) return false;
  // 版本未知：拿不到「是否已经包含」的证据，只能重拉一次以确认。
  if (typeof deferred.version !== "number" || !Number.isFinite(deferred.version)) return true;
  return deferred.version > current;
}

/**
 * 推迟刷新期间给用户的如实说明。
 *
 * 措辞刻意不写「云端修复了」：版本推进同样可能来自另一个窗口里的人为编辑，界面
 * 没有证据区分二者，把两者都说成云端自动修复就是在编造一个它并不知道的原因。
 * 能确定的事实只有两条——这份稿被别处改过、以及本地修改还没有落盘。
 */
export function describeDeferredRemoteRefresh(deferred: DeferredRemoteRefresh): string {
  const tail = "你的修改正在保存，保存完成后会自动加载最新版本。";
  const version = deferred.version;
  if (typeof version === "number" && Number.isFinite(version)) {
    return `这份题稿在别处已被更新（版本 ${version}）。${tail}`;
  }
  return `这份题稿在别处已被更新。${tail}`;
}
