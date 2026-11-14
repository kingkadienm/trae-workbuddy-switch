import { useLocation, useNavigate } from "react-router-dom";
import { PlugZap } from "lucide-react";

import { ScopeChip, scopeLabel } from "@/components/scope-chip";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { EmptyState } from "@/components/ui/empty-state";
import { useT } from "@/lib/i18n";
import { platformDescriptor, platformOfPath } from "@/lib/platforms/registry";
import { scopeOf } from "@/lib/scope";
import { useScope } from "@/lib/use-scope";

/**
 * Qoder 侧**尚未接入**的目的地（概览 / 会话 / 网关 / 设置）—— **结构化空态**。
 *
 * ## 为什么给空态而不是复用其它平台的页面
 *
 * 复用会把**别的平台的账号**渲染进来（概览读的是 WorkBuddy 的 store），
 * 用户会以为「Qoder 的账号已经在用了」—— 这正是本仓反复防的「静默错平台」。
 * 空态把「还没有这个页面的数据源」讲清楚，并给**唯一可用的出口**（账号页）。
 *
 * ⚠️ 本页**不加载任何数据**，因此**不渲染任何计数**：报告一个编出来的「0」
 * 等于臆造（§3.5 不假承诺）。
 */
export default function QoderPendingPage() {
  const t = useT();
  const navigate = useNavigate();
  const location = useLocation();
  const product = platformOfPath(location.pathname);
  const [region] = useScope(product);
  const scope = scopeOf(product, region);
  /* 产品名走注册表（唯一来源），**不**写死 `Qoder` —— 与侧栏产品切换器同一纪律。 */
  const productLabel = t(platformDescriptor(product).labelKey);

  return (
    <div className="mx-auto flex w-full max-w-[1240px] flex-col gap-5 px-7 pt-[26px] pb-[60px]">
      <header className="flex items-start gap-4">
        <div className="min-w-0">
          <h1 className="text-[28px] font-[640] tracking-[-0.02em]">
            {t("qoder.page.pending.title")}
          </h1>
          <p className="mt-1.5 text-[12.5px] text-muted-foreground">
            {productLabel} · {scopeLabel(t, scope)}
          </p>
        </div>
        <div className="grow" />
        <ScopeChip scope={scope} />
      </header>

      <Card className="gap-0 py-0">
        <EmptyState
          icon={PlugZap}
          title={t("qoder.page.pending.title")}
          description={t("qoder.page.pending.description")}
          action={
            <Button variant="outline" size="sm" onClick={() => navigate(`/s/${scope}/accounts`)}>
              {t("qoder.page.pending.action")}
            </Button>
          }
        />
      </Card>
    </div>
  );
}
