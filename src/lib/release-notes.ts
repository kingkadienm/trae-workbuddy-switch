/**
 * 版本说明（升级前弹窗的内容）—— **纯函数**，无 React、无 DOM。
 *
 * ## 为什么这里还要再校验一次形状
 *
 * 后端（`update::update_check`）已经把 release 资产洗过一遍，但这份数据仍然跨越了
 * 「外部输入 → 渲染」的边界：发布资产是**可以被替换的**（仓库 owner / 用户自定义更新源）。
 * 因此渲染前再走一次白名单归一：
 *   - 只保留契约里的字段（`version` / `breaking` / `sections` / `older` / …）；
 *   - 非字符串、空串、超长条目一律丢弃；
 *   - 结果里**没有 markdown**，渲染层只做文本插值（React 默认转义）。
 *
 * 判据：宁可少显示一条，也不把未知结构送进渲染树。
 *
 * ## 为什么单独成文件、且不依赖 i18n
 *
 * 前端没有单测框架，可验证性全靠「纯函数 + 门禁直接调」。`scripts/check-scope-registry.cjs`
 * 会 `require` 这个模块断言几条不变量（见 `★ 版本说明：归一化只留白名单字段`）。
 */

import type { ReleaseNoteBreaking, ReleaseNoteOlder, ReleaseNoteSection, ReleaseNotes } from "@/lib/types";

/** 单条文本上限：超过就当脏数据丢弃（一条 release note 不可能这么长）。 */
const MAX_TEXT = 400;
/** 分组 / 条目数量上限：防止异常资产把弹窗撑爆（分组 ≤ 12，每条分组 ≤ 30 条）。 */
const MAX_SECTIONS = 12;
const MAX_ITEMS_PER_SECTION = 30;
const MAX_OLDER = 10;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** 取一段可显示的文本：非字符串 / 空白 / 超长 ⇒ `null`。**不 trim 内部换行**，但压掉首尾空白。 */
function text(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  if (!trimmed || trimmed.length > MAX_TEXT) return null;
  return trimmed;
}

function breakingOf(value: unknown): ReleaseNoteBreaking[] {
  if (!Array.isArray(value)) return [];
  const out: ReleaseNoteBreaking[] = [];
  for (const item of value) {
    if (!isRecord(item)) continue;
    const title = text(item.title);
    if (!title) continue;
    const desc = text(item.desc);
    out.push(desc ? { title, desc } : { title });
    if (out.length >= 8) break;
  }
  return out;
}

function sectionsOf(value: unknown): ReleaseNoteSection[] {
  if (!Array.isArray(value)) return [];
  const out: ReleaseNoteSection[] = [];
  for (const group of value) {
    if (!isRecord(group)) continue;
    const title = text(group.title);
    if (!title) continue;
    const items = Array.isArray(group.items)
      ? group.items
          .map(text)
          .filter((item): item is string => Boolean(item))
          .slice(0, MAX_ITEMS_PER_SECTION)
      : [];
    /* 空分组不渲染：一个只有标题、没有条目的小节比不显示更让人困惑。 */
    if (!items.length) continue;
    out.push({ title, items });
    if (out.length >= MAX_SECTIONS) break;
  }
  return out;
}

function olderOf(value: unknown): ReleaseNoteOlder[] {
  if (!Array.isArray(value)) return [];
  const out: ReleaseNoteOlder[] = [];
  for (const entry of value) {
    if (!isRecord(entry)) continue;
    const version = text(entry.version);
    if (!version) continue;
    const sections = sectionsOf(entry.sections);
    if (!sections.length) continue;
    const publishedAt = text(entry.publishedAt);
    out.push(publishedAt ? { version, publishedAt, sections } : { version, sections });
    if (out.length >= MAX_OLDER) break;
  }
  return out;
}

/**
 * 版本号比较（`2026.10.171833` 这种「日期 + 时分」口径，逐段按数值比）。
 *
 * 为什么不用字符串比较：段长不固定（`2026.9.221126` vs `2026.10.171833`），
 * 字典序会把 9 月排到 10 月之后。与后端 `update.rs::compare_versions` 同一口径。
 */
export function compareVersions(a: string, b: string): number {
  const parse = (value: string) =>
    value
      .trim()
      .replace(/^v/, "")
      .split(".")
      .map((part) => Number.parseInt(part, 10))
      .map((part) => (Number.isFinite(part) ? part : 0));
  const left = parse(a);
  const right = parse(b);
  const length = Math.max(left.length, right.length);
  for (let index = 0; index < length; index += 1) {
    const x = left[index] ?? 0;
    const y = right[index] ?? 0;
    if (x !== y) return x > y ? 1 : -1;
  }
  return 0;
}

/** 弹窗里最多铺几个历史版本：再多也不是「说明」，而是一份归档。 */
const MAX_OLDER_SHOWN = 5;

/**
 * 「更早的 N 个版本」= `history` 里落在 `(current, notes.version)` 开区间内的版本。
 *
 * 为什么要切片而不是直接给全部：落后 1 版和落后 8 版的用户看到的东西必须不同 ——
 * 前者不需要历史归档，后者需要知道漏掉了什么。切片放在这里（纯函数）而不是发布侧，
 * 因为**只有客户端知道自己装的是哪一版**。
 */
export function olderNotes(notes: ReleaseNotes | null | undefined, current?: string): ReleaseNoteOlder[] {
  const history = notes?.history ?? [];
  if (!history.length || !current) return [];
  return history
    .filter((entry) => entry.version !== notes?.version)
    .filter((entry) => compareVersions(entry.version, current) > 0)
    .sort((a, b) => compareVersions(b.version, a.version))
    .slice(0, MAX_OLDER_SHOWN);
}

/**
 * 把来路不明的对象归一成 `ReleaseNotes`；**连 `version` 都没有就返回 `null`**
 * （没有版本号的说明无法判断它属于哪一版，宁可不显示）。
 */
export function normalizeReleaseNotes(raw: unknown): ReleaseNotes | null {
  if (!isRecord(raw)) return null;
  const version = text(raw.version);
  if (!version) return null;

  const notes: ReleaseNotes = { version };

  const publishedAt = text(raw.publishedAt) ?? text(raw.pub_date);
  if (publishedAt) notes.publishedAt = publishedAt;

  const releaseUrl = text(raw.releaseUrl) ?? text(raw.release_url);
  if (releaseUrl) notes.releaseUrl = releaseUrl;

  const size = typeof raw.size === "number" && Number.isFinite(raw.size) && raw.size > 0 ? raw.size : null;
  if (size) notes.size = size;

  const breaking = breakingOf(raw.breaking);
  if (breaking.length) notes.breaking = breaking;

  const sections = sectionsOf(raw.sections);
  if (sections.length) notes.sections = sections;

  const history = olderOf(raw.history);
  if (history.length) notes.history = history;

  return notes;
}

/** 「3 组 · 11 条」里的两个数；供标题旁的小字用（也是门禁断言数据与渲染同源的锚点）。 */
export function notesStats(notes: ReleaseNotes | null | undefined): { groups: number; items: number } {
  const sections = notes?.sections ?? [];
  return {
    groups: sections.length,
    items: sections.reduce((sum, section) => sum + section.items.length, 0),
  };
}

/**
 * 发布时间 → 本地化字符串；解析失败返回 `null`（**不留一个假日期**）。
 *
 * 只到分钟：版本说明不是排期表，秒级精度是噪声。
 */
export function formatNotesDate(iso: string | undefined, locale: "zh" | "en"): string | null {
  if (!iso) return null;
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return null;
  try {
    return new Intl.DateTimeFormat(locale === "zh" ? "zh-CN" : "en-US", {
      year: "numeric",
      month: locale === "zh" ? "numeric" : "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    }).format(date);
  } catch {
    return null;
  }
}

/** 字节数 → 「27.4 MB」；`0` / 非法值返回 `null`（不显示 0 B 这种噪声）。 */
export function formatNotesSize(bytes: number | undefined): string | null {
  if (!bytes || !Number.isFinite(bytes) || bytes <= 0) return null;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
