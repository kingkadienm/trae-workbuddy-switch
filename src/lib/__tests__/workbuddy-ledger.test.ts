import { describe, expect, it } from "vitest";
import { mergeSources, workbuddyClientSources } from "../usage-charts";
import { combinedTotal } from "../usage-ledger";
import type { TokenStatsSource } from "../types";

function source(kind: TokenStatsSource["source"], tokens: number): TokenStatsSource {
  const totals = { total: tokens, input: tokens, output: 0, cacheRead: 0, cacheWrite: 0, uncachedInput: tokens, records: 1, cacheHitRate: null };
  return { source: kind, summary: totals, models: [], projects: [], sessions: [], daily: [{ ...totals, key: "2026-11-14" }], hours: [], filesScanned: 0, parseErrors: 0 };
}

describe("WorkBuddy新版双账本与旧虚拟源兼容", () => {
  it("客户端与网关各计一次，旧API虚拟源不进入客户端汇总", () => {
    const legacy = [source("workbuddy", 10), source("workbuddy-ai", 5), source("workbuddy-gateway", 20)];
    const client = mergeSources(workbuddyClientSources(legacy));
    expect(client.summary.total).toBe(15);
    expect(client.daily[0].total).toBe(15);
    expect(combinedTotal(client.summary.total, 20, 0)).toBe(35);
    // 原始响应保持不变，旧统计页仍可自行汇总兼容来源。
    expect(mergeSources(legacy).summary.total).toBe(35);
    expect(legacy).toHaveLength(3);
  });
  it("只有旧网关虚拟源时客户端账本为空，不虚构客户端用量", () => {
    const client = mergeSources(workbuddyClientSources([source("workbuddy-gateway", 20)]));
    expect(client.summary.total).toBe(0);
    expect(client.daily).toEqual([]);
    expect(combinedTotal(client.summary.total, 20, 0)).toBe(20);
  });
});
