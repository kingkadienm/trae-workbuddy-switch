/**
 * 网关工作台的**两个轴**：类型（`GW_TYPES`）× 分区（`GW_SECTIONS`）。
 *
 * ## 为什么是「类型 + 分区」，而不是一张 7 项的表
 *
 * 改造前子导航是**一张 7 项的表**，第 7 项是「统一网关」。问题是它**跨全部作用域**，
 * 而另外 6 项**都随作用域过滤** —— 两个正交的轴挤在同一层，代价是必须写一行
 * 「上方作用域选择器对它不生效」去解释这个错位，页头还挂着一枚会误导的 `ScopeChip`。
 *
 * 三种放法都试过（记在这里免得再走一遍）：
 * 1. 塞进子导航第 7 项 —— 就是上面那个错位；
 * 2. 升成**侧栏独立目的地** —— 侧栏变乱，而且「它仍然是网关」这件事被拆散了；
 * 3. **当前方案**：网关页内分两个**类型**，类型切换在左栏顶部 ⇒ 两个轴各归其位，侧栏保持 6 项。
 *
 * ## 一份表，三条产品线共用
 *
 * WorkBuddy / Trae / Qoder 的网关是**三套后端**（上游、端口、Key 模型都不同），
 * 但**分类完全相同** ⇒ 表只声明一次，各页渲染自己的内容。词条落在 `shell` 域。
 *
 * ⚠️ Trae / Qoder **没有**「请求代理」（P12 的 C 路只覆盖 WorkBuddy 网关端口）⇒
 * 传 `GW_INDEPENDENT_SECTIONS_NO_PROXY`。缺能力就不画空壳（§3.4 规则 1），
 * 由页面按 `sections` 传表，而不是在这里写死平台判断。
 *
 * ⚠️ **原型子导航没有图标**（`demo-app.js:3089` 只渲染 label）⇒ 本仓此前的
 * 每项一枚 16px 图标属于「demo 里不存在的自定义设计」，已删除（与设置子导航同步）。
 *
 * ## 分区 id 为什么统一网关侧带 `u-` 前缀
 *
 * 两个类型的分区 id 集合**不相交** ⇒ 一个 `sub` state 就够，不必两个；
 * 且「切类型时重置分区」不会撞上另一个类型的同名分区（与原型 `gwSubId()` 同款做法）。
 */

import { SideSubnav, type SideSubnavItem } from "@/components/shell/side-subnav";
import { useT } from "@/lib/i18n";

/* ─────────── 类型 ─────────── */

/** 网关页的两个类型。 */
export type GatewayType = "independent" | "unified";

/** 类型切换表（顺序即展示顺序）。 */
export const GW_TYPES: readonly SideSubnavItem<GatewayType>[] = [
  { id: "independent", labelKey: "gateway.type.independent" },
  { id: "unified", labelKey: "gateway.type.unified" },
];

/* ─────────── 分区 ─────────── */

/** 网关页的分区 id（两个类型的**并集**）。 */
export type GatewaySectionId =
  /* 独立网关（6 项，随作用域过滤） */
  | "overview"
  | "keys"
  | "models"
  | "pool"
  | "proxy"
  | "logs"
  /* 统一网关（4 项，跨全部作用域）—— 前缀 `u-` 保证与上面不相交 */
  | "u-overview"
  | "u-namespaces"
  | "u-keys"
  | "u-pool";

/** 独立网关的分区（6 项）。 */
export const GW_INDEPENDENT_SECTIONS: readonly SideSubnavItem<GatewaySectionId>[] = [
  { id: "overview", labelKey: "gateway.subnav.overview" },
  { id: "keys", labelKey: "gateway.subnav.keys" },
  { id: "models", labelKey: "gateway.subnav.models" },
  { id: "pool", labelKey: "gateway.subnav.pool" },
  { id: "proxy", labelKey: "gateway.subnav.proxy" },
  { id: "logs", labelKey: "gateway.subnav.logs" },
];

/**
 * 无「请求代理」的独立网关分区（Trae / Qoder 用）。
 *
 * P12 的「C 路 base URL 重定向」只覆盖 **WorkBuddy 网关端口**（`proxy.rs` 的
 * `PROXY_GATEWAY_BASE`），另两条产品线没有对应后端 ⇒ 不渲染该项 ——
 * 一个点进去什么都没有的项，比不渲染更糟（§3.4 规则 1）。
 */
export const GW_INDEPENDENT_SECTIONS_NO_PROXY: readonly SideSubnavItem<GatewaySectionId>[] =
  GW_INDEPENDENT_SECTIONS.filter((item) => item.id !== "proxy");

/**
 * 统一网关的分区（4 项，**跨全部作用域**）。
 *
 * - 「接入说明」**并回概览**（与独立网关一致：它的接入说明也在概览里）；
 * - 「账号池」与独立网关**同名同位**，但语义不同：统一网关**没有自己的池**，
 *   它按命名空间前缀分派到三条产品线各自的池 ⇒ 这一区是**跨平台的只读汇总**。
 */
export const GW_UNIFIED_SECTIONS: readonly SideSubnavItem<GatewaySectionId>[] = [
  { id: "u-overview", labelKey: "unifiedGateway.subnav.overview" },
  { id: "u-namespaces", labelKey: "unifiedGateway.subnav.namespaces" },
  { id: "u-keys", labelKey: "unifiedGateway.subnav.keys" },
  /* 「账号池」= 跨平台的**只读汇总**（统一网关没有自己的池）。放在配置之后、与独立网关**同名同位**。 */
  { id: "u-pool", labelKey: "unifiedGateway.subnav.pool" },
];

/**
 * 由统一网关的**分区名**构造分区 id（`u-` 前缀的约定**只在这里写一次**）。
 *
 * 为什么需要它：`UnifiedGatewayView` 只认裸分区名（`overview` / `namespaces` / …，那是它自己的
 * `section` prop 类型），而本表用的是带前缀的 id。少了这个收敛点，`"u-" + s` 这种拼接会散落到三个页面里
 * —— 前缀一旦改就要改四处，且没有任何东西会报错。
 *
 * ⚠️ **fail-closed**：不认识的裸名回该类型第一项，不把任意串塞进渲染。
 */
export function unifiedSectionId(section: string): GatewaySectionId {
  const id = `u-${section}` as GatewaySectionId;
  return GW_UNIFIED_SECTIONS.some((item) => item.id === id) ? id : GW_UNIFIED_SECTIONS[0].id;
}

/** 唯一的一份分区表（按类型取）。 */
export const GW_SECTIONS: Record<GatewayType, readonly SideSubnavItem<GatewaySectionId>[]> = {
  independent: GW_INDEPENDENT_SECTIONS,
  unified: GW_UNIFIED_SECTIONS,
};

/* ─────────── 解析（全部 fail-closed） ─────────── */

/** 从 URL 段解析类型（非法值 fail-closed 回 `independent`）。 */
export function parseGatewayType(raw: string | null): GatewayType {
  return raw === "unified" ? "unified" : "independent";
}

/**
 * 按**当前类型**取分区 id（非法值 fail-closed 回该类型的第一项）。
 *
 * ⚠️ 必须按类型校验：`sub=pool` 在独立网关下合法、在统一网关下非法
 * （统一网关的叫 `u-pool`）。少了这一步，切类型后会停在另一个类型的分区上。
 */
export function gatewaySection(type: GatewayType, raw: string | null): GatewaySectionId {
  const items = GW_SECTIONS[type];
  return items.some((item) => item.id === raw) ? (raw as GatewaySectionId) : items[0].id;
}

/**
 * 深链解析（**兼容两代旧链**）。
 *
 * 统一网关先后放过「子导航第 7 项」（`?sub=unified`）与「侧栏独立目的地」（`?dest=unified`），
 * 两条链都等价于 `type=unified`。**旧链接不许失效。**
 *
 * 另：`?sub=u-pool` 这类**统一网关的分区 id** 也能直接给 —— 此时类型自动跟上，
 * 否则会被缺省的 `independent` 先吃掉，深链静默落到概览。
 */
export function parseGatewayDeepLink(params: URLSearchParams): {
  type: GatewayType;
  section: GatewaySectionId;
} {
  const sub = params.get("sub");
  const wantsUnified =
    params.get("type") === "unified" || sub === "unified" || params.get("dest") === "unified";
  if (wantsUnified) {
    /* `sub=unified` 是旧链、不含分区 ⇒ 回该类型第一项；`sub=u-*` 是真分区 ⇒ 保留。 */
    return { type: "unified", section: gatewaySection("unified", sub === "unified" ? null : sub) };
  }
  /* `sub=u-*` 但 `type` 没给 ⇒ 类型跟着分区走。 */
  if (sub && GW_UNIFIED_SECTIONS.some((item) => item.id === sub)) {
    return { type: "unified", section: sub as GatewaySectionId };
  }
  return { type: "independent", section: gatewaySection("independent", sub) };
}

/* ─────────── 渲染 ─────────── */

interface Props {
  active: GatewaySectionId;
  onSelect: (id: GatewaySectionId) => void;
  /** 该类型的分区表（默认独立网关全 6 项；Trae / Qoder 传去掉「请求代理」的）。 */
  items?: readonly SideSubnavItem<GatewaySectionId>[];
  className?: string;
}

/** 网关分区子导航（版式由 `SideSubnav` 统一提供）。 */
export function GatewaySubnav({ active, onSelect, items = GW_INDEPENDENT_SECTIONS, className }: Props) {
  const t = useT();
  return (
    <SideSubnav
      active={active}
      items={items}
      onSelect={onSelect}
      ariaLabel={t("gateway.subnav.aria")}
      className={className}
    />
  );
}
