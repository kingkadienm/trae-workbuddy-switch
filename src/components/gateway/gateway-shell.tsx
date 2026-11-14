/**
 * 网关工作台的**左栏外壳**：类型切换（`Seg`）+ 该类型的分区子导航。
 *
 * ## 为什么抽成外壳
 *
 * 网关页现在有**两个轴**（原型 `.side-col`）：
 * - **类型**（独立网关 / 统一网关）—— 回答「哪个网关」；
 * - **分区** —— 回答「这个网关的哪个分区」。
 *
 * 三个产品线页（WorkBuddy / Trae / Qoder）共用同一套外壳，差异只走两个入参：
 * `sections`（Trae / Qoder 去掉「请求代理」）与 `children`。抽出来是因为
 * 「同款实现散落三处必然漂移」——本仓已发生过多次（Trae 页曾缺 `data-split-layout`
 * 与 `goSub`，窄屏不转纵向、URL 也不跟着变）。
 *
 * ## 类型切换用既有 `Seg` 原语，不自造
 *
 * ⚠️ 原型里它是 `.gw-type-seg`（因为原型的 `.seg` 已被用量页 / 接入方式占用，
 * 重定义会**全局改掉那些控件**）。真实实现里 `Seg` 是**可复用的原语**且带 `className`
 * ⇒ 直接用它（`className="grid w-full grid-cols-2"` 覆盖成两列），**不要**再写一套分段控件。
 *
 * ## 类型与分区的联动由调用方负责
 *
 * 切类型时**必须重置分区**到该类型的第一项（两个类型的分区 id 集合不相交，
 * 停在另一个类型的分区上会渲染成空）。这一步放在页面的 `goType` 里，外壳不管 ——
 * 外壳只负责渲染与回调。
 */

import type { ReactNode } from "react";

import { Seg } from "@/components/ui/seg";
import { useT } from "@/lib/i18n";

import {
  GW_SECTIONS,
  GW_TYPES,
  GatewaySubnav,
  type GatewaySectionId,
  type GatewayType,
} from "@/components/gateway/gateway-subnav";
import type { SideSubnavItem } from "@/components/shell/side-subnav";

interface Props {
  type: GatewayType;
  section: GatewaySectionId;
  /**
   * 该类型的分区表。缺省用 `GW_SECTIONS[type]`；
   * Trae / Qoder 传 `GW_INDEPENDENT_SECTIONS_NO_PROXY`（缺能力不画空壳）。
   */
  sections?: readonly SideSubnavItem<GatewaySectionId>[];
  onTypeChange: (next: GatewayType) => void;
  onSelect: (id: GatewaySectionId) => void;
  children: ReactNode;
}

export function GatewayShell({ type, section, sections, onTypeChange, onSelect, children }: Props) {
  const t = useT();
  const items = sections ?? GW_SECTIONS[type];
  return (
    /* 方向 / 对齐 / 间距整组在 `index.css` 的 `[data-split-layout]`（原型 `.split`，含 900px 转纵向）。 */
    <div data-split-layout className="min-w-0">
      <div data-gateway-sidebar>
        <div>
          <div className="px-0.5 pb-1.5 text-[10.5px] font-semibold tracking-[0.02em] text-muted-foreground">
            {t("gateway.type.label")}
          </div>
          <Seg<GatewayType>
            className="grid w-full grid-cols-2"
            value={type}
            onChange={onTypeChange}
            options={GW_TYPES.map((item) => ({ value: item.id, label: t(item.labelKey) }))}
            ariaLabel={t("gateway.type.label")}
          />
        </div>
        <GatewaySubnav active={section} items={items} onSelect={onSelect} />
      </div>
      <div className="min-w-0 flex-1">{children}</div>
    </div>
  );
}
