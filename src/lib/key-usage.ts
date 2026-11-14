/**
 * 密钥用量的**纯聚合逻辑**（issue #76 / 规范 U4）。
 *
 * 单独成文件而不是留在 `key-ranking.tsx` 里：它是**纯函数**，应当能被脚本直接跑
 * （`scripts/check-key-usage.mjs`）。留在 `.tsx` 里就得为了跑断言而引入 React 运行时。
 *
 * ## 数据源：网关**账本**的 `keys` 维度（不是请求日志）
 *
 * 2026-10-28 之前这里聚合的是 `get_gateway_logs` 的条目。那是个**环形缓冲**
 * （只留最近 `log_keep` 条，缺省 200）—— 本机实测：繁忙时 200 条只覆盖 **17 分钟**，
 * 而页面标的是「近 30 天」⇒ 出现「44.2M Token / 200 次 / 100%」这种
 * **量级正确、区间错误**的读数（用户报障的原始截图）。
 *
 * 现在改读账本的 `days[].keys`：按本地日累积、可跨 90 天，与「按模型 / 按账号」
 * 同一份账、同一个水位线 ⇒ 窗口对齐后「按密钥合计 = 网关总量」才成立。
 *
 * ## 三条判据（都有测试守着）
 *
 * 1. `__legacy__`（升级前的条目：那时日志还没有 `keyId` 字段）与 `__none__`
 *    （确实没有 Key 的调用）**分开统计** —— 合并会让历史数据凭空多出一批
 *    「匿名调用」，而这个数字一旦出现在排行里就会被当成真的；
 * 2. 桶键带归属（`{keyScope}:{keyId}`）—— 同一个 id 字符串在两套台账里是两把不同的 Key；
 * 3. 升级前的日期**没有** `keys` 维度（空数组）⇒ 那段窗口不可得，由视图披露，
 *    绝不画成「0 用量」。
 */

/** 一行排行（按密钥聚合后）。 */
export interface KeyUsageSlice {
  /** 桶键（`{scope}:{id}` / 两个哨兵）；同时用作 React key。 */
  bucket: string;
  /** 密钥 id；哨兵行（老条目 / 无密钥）为 `null`。 */
  keyId: string | null;
  /** 展示名（Key 名；取不到时用短 id）。 */
  name: string;
  tokens: number;
  requests: number;
  credit: number;
}

/** 账本里「升级前的历史条目」桶键（与后端 `token_daily::KEY_DIM_LEGACY` 逐字相同）。 */
export const LEGACY_BUCKET = "__legacy__";
/** 账本里「确实没有 Key 的调用」桶键（与后端 `token_daily::KEY_DIM_NONE` 逐字相同）。 */
export const NONE_BUCKET = "__none__";

/** 台账标签（跨台账的 Key 在「按密钥」里要标清来自哪里）。 */
export const SCOPE_LABELS: Record<string, string> = {
  wb: "WorkBuddy",
  trae: "Trae",
  qoder: "Qoder",
  unified: "统一网关",
};

/** 老条目（升级前写入、没有 `keyId` 键）单独一行展示的标识。 */
export const LEGACY_KEY_LABEL = "（升级前的历史条目）";

/** 无 Key 调用的展示名。 */
export const NO_KEY_LABEL = "无密钥调用";

/**
 * 账本的「按密钥」维度**只要这几个字段**（结构最小接口）。
 *
 * 不直接绑 `GatewayUsageDimDay`：三条产品线的 `usage_dim` 是**三个命令**
 * （WB 的 `token_daily` 与 Trae/Qoder 的 `gateway_ledger`），绑死某一个类型就得
 * 在三个页面各写一遍适配器。
 */
export interface KeyDimEntry {
  /** 维度桶键（见文件头判据 2）。 */
  key?: string;
  input: number;
  output: number;
  requests: number;
  credit: number;
}

/** 一天的密钥维度。 */
export interface KeyDimDay {
  /** 本地日期键 `YYYY-MM-DD`。 */
  key: string;
  keys: readonly KeyDimEntry[];
  /** 该日的密钥维度是从中间开始记的（v3 → v4 播种）⇒ 只是下界。 */
  keysPartial?: boolean;
}

/** 解析维度桶键 → `{ scope, id }`；两个哨兵各自独立。 */
export function splitBucket(
  bucket: string,
  selfScope: string,
): { kind: "legacy" } | { kind: "none" } | { kind: "key"; scope: string; id: string } {
  if (bucket === LEGACY_BUCKET) return { kind: "legacy" };
  if (bucket === NONE_BUCKET) return { kind: "none" };
  const at = bucket.indexOf(":");
  // 没有归属（老账本 / Trae 自己的 Key）⇒ 就是本台账的 Key，不编一个 scope 出来。
  if (at < 0) return { kind: "key", scope: selfScope, id: bucket };
  return { kind: "key", scope: bucket.slice(0, at), id: bucket.slice(at + 1) };
}

/** 取不到 Key 名时的兜底：前 8 位 + 省略号（够辨认，不占宽度）。 */
export function shortId(id: string): string {
  return id.length > 8 ? `${id.slice(0, 8)}…` : id;
}

/**
 * 把账本「按密钥」维度在窗口内聚合成排行（**纯函数**，不依赖 React / 网络）。
 *
 * @param days 账本逐日记录（**未过滤**；窗口过滤在这里做，与另三个视图同一套日期键）
 * @param windowKeys 窗口内日期键集合（`usageWindowKeys` 的产物）
 * @param keyNames `id → 展示名`（来自密钥列表；缺失的 id 用短 id 兜底）
 * @param selfScope 本页所属台账（`"wb"` / `"trae"` / `"qoder"`）；跨台账的 Key 单独命名
 */
export function aggregateByKeyDim(
  days: readonly KeyDimDay[],
  windowKeys: ReadonlySet<string>,
  keyNames: ReadonlyMap<string, string>,
  selfScope: string,
): KeyUsageSlice[] {
  const buckets = new Map<string, KeyUsageSlice>();

  for (const day of days) {
    if (!windowKeys.has(day.key)) continue;
    for (const entry of day.keys) {
      const bucket = entry.key ?? "";
      let slice = buckets.get(bucket);
      if (!slice) {
        const parsed = splitBucket(bucket, selfScope);
        slice = {
          bucket,
          keyId: parsed.kind === "key" ? parsed.id : null,
          name:
            parsed.kind === "legacy"
              ? LEGACY_KEY_LABEL
              : parsed.kind === "none"
                ? NO_KEY_LABEL
                : parsed.scope !== selfScope
                  ? /* 跨台账：**明确标出**「这是网关（或另一条产品线）的 Key」，
                       而不是拿短 id 冒充本地 Key。 */
                    `${SCOPE_LABELS[parsed.scope] ?? parsed.scope} 的密钥（${shortId(parsed.id)}）`
                  : (keyNames.get(parsed.id) ?? shortId(parsed.id)),
          tokens: 0,
          requests: 0,
          credit: 0,
        };
        buckets.set(bucket, slice);
      }
      // 账本的维度项已经拆好 input/output（token 口径与「网关总量」一致）。
      slice.tokens += entry.input + entry.output;
      slice.requests += entry.requests;
      slice.credit += entry.credit;
    }
  }

  // 按 token 降序（token 为 0 的按请求数排 —— 业务代理只有请求数没有 token）。
  return [...buckets.values()].sort(
    (left, right) => right.tokens - left.tokens || right.requests - left.requests,
  );
}

/**
 * 密钥维度在窗口内的**覆盖情况**（给视图披露用）。
 *
 * - `days`：窗口内**有** key 维度的天数（有记录、且那天确实有密钥维度）；
 * - `missing`：窗口内**没有**该维度的天数 —— 升级前的日期，**不可得**，不是 0；
 * - `partial`：其中几天是从中间开始记的（值是下界）。
 */
export function keyCoverage(
  days: readonly KeyDimDay[],
  windowKeys: readonly string[],
): { days: number; missing: number; partial: number } {
  const byDay = new Map(days.map((day) => [day.key, day]));
  let covered = 0;
  let partial = 0;
  for (const key of windowKeys) {
    const day = byDay.get(key);
    if (!day || day.keys.length === 0) continue;
    covered += 1;
    if (day.keysPartial) partial += 1;
  }
  return { days: covered, missing: windowKeys.length - covered, partial };
}

/** 窗口内**最早**有密钥维度的那一天（`null` = 窗口内完全没有该维度）。 */
export function firstKeyDay(
  days: readonly KeyDimDay[],
  windowKeys: ReadonlySet<string>,
): string | null {
  let first: string | null = null;
  for (const day of days) {
    if (!windowKeys.has(day.key) || day.keys.length === 0) continue;
    if (first === null || day.key < first) first = day.key;
  }
  return first;
}
