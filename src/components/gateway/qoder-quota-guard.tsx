import { useEffect, useMemo, useState } from "react";
import { Loader2, RefreshCw, ShieldCheck } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { CardHead } from "@/components/ui/card-head";
import { Input } from "@/components/ui/input";
import { useT } from "@/lib/i18n";
import type { TranslationKey } from "@/locales/zh";
import type { QoderGatewayQuotaView } from "@/lib/api";
import { cn } from "@/lib/utils";

/**
 * 限额护栏（G1 余额地板 / G2 当日 token / G4 单模型当日 token）。
 *
 * ## 为什么是一张**独立卡**而不是塞进账号池卡
 *
 * 账号池卡的职责是「谁可用」，护栏的职责是「用多少就停」—— 两者的数据源也不同
 * （池是账号库视图，护栏是网关自己的记账 + 余额快照）。共用一张卡会让「为什么这个号
 * 显示可用但请求被拒」变得无从解释。
 *
 * ## 三条产品纪律（写在界面上，而不是只写在代码里）
 *
 * 1. **`0` = 关闭**：默认全关。个人自用形态下，默认限额只会让人莫名被拦。
 * 2. **未知一律放行**：余额没查过 / 计数缺失 ⇒ 不拦。界面明说这一点，
 *    用户才不会以为「开了护栏就一定精确到 token」。
 * 3. **拦截原因可读**：每一行都给出「为什么被拦」，不是一个 `blocked: true`。
 */
export interface QoderQuotaGuardProps {
  /** 护栏视图（含逐账号当日用量）；`null` = 还没加载到。 */
  quota: QoderGatewayQuotaView | null;
  /** 正在重取视图。 */
  refreshing: boolean;
  /** 正在保存阈值。 */
  saving: boolean;
  onRefresh: () => void;
  onSave: (limits: {
    reserveCredits: number;
    dailyTokenLimit: number;
    dailyCreditLimit: number;
    modelDailyTokenLimit: number;
  }) => void;
}

/** 拦截代号 → 徽标文案。**显式枚举**（不拼字符串）：漏一种在类型层就报错。 */
const BLOCK_LABELS: Record<string, TranslationKey> = {
  parked: "qoder.gateway.quota.why.parked",
  reserve: "qoder.gateway.quota.why.reserve",
  daily_tokens: "qoder.gateway.quota.why.daily_tokens",
  model_tokens: "qoder.gateway.quota.why.model_tokens",
};

/** 一条阈值输入（数字串草稿 + 非负校验）。 */
function LimitField({
  label,
  hint,
  value,
  invalid,
  onChange,
  testId,
}: {
  label: string;
  hint: string;
  value: string;
  invalid: boolean;
  onChange: (next: string) => void;
  testId: string;
}) {
  return (
    <label className="block min-w-0" data-act={testId}>
      <span className="text-[12.5px] font-medium">{label}</span>
      <Input
        className={cn("mt-1 h-8 text-[12.5px]", invalid && "border-destructive")}
        inputMode="numeric"
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
      <span
        className={cn(
          "mt-1 block text-[11px] leading-4",
          invalid ? "text-destructive" : "text-muted-foreground",
        )}
      >
        {hint}
      </span>
    </label>
  );
}

/** 解析阈值草稿：空串 ⇒ 0（关闭）；负数 / 小数 / 乱码 ⇒ `null`（让输入框标红）。 */
function parseLimit(raw: string): number | null {
  const text = raw.trim();
  if (text === "") return 0;
  if (!/^\d+$/.test(text)) return null;
  const value = Number.parseInt(text, 10);
  return Number.isFinite(value) ? value : null;
}

/** 解析 credit 阈值草稿：允许小数（上游按 0.0x 计价）；空串 = 0（关）。 */
function parseCredit(raw: string): number | null {
  const text = raw.trim();
  if (text === "") return 0;
  if (!/^\d+(\.\d+)?$/.test(text)) return null;
  const value = Number.parseFloat(text);
  return Number.isFinite(value) ? value : null;
}

export function QoderQuotaGuard({
  quota,
  refreshing,
  saving,
  onRefresh,
  onSave,
}: QoderQuotaGuardProps) {
  const t = useT();

  // 三个草稿：初始取视图里的阈值（视图没到 ⇒ 全 0 = 全关），保存成功后再同步。
  const limits = quota?.limits;
  const [reserve, setReserve] = useState(() => String(limits?.reserveCredits ?? 0));
  const [daily, setDaily] = useState(() => String(limits?.dailyTokenLimit ?? 0));
  const [credit, setCredit] = useState(() => String(limits?.dailyCreditLimit ?? 0));
  const [perModel, setPerModel] = useState(() => String(limits?.modelDailyTokenLimit ?? 0));

  useEffect(() => {
    if (!limits) return;
    setReserve(String(limits.reserveCredits));
    setDaily(String(limits.dailyTokenLimit));
    setCredit(String(limits.dailyCreditLimit));
    setPerModel(String(limits.modelDailyTokenLimit));
  }, [
    limits?.reserveCredits,
    limits?.dailyTokenLimit,
    limits?.dailyCreditLimit,
    limits?.modelDailyTokenLimit,
  ]);

  const parsed = useMemo(
    () => ({
      reserveCredits: parseLimit(reserve),
      dailyTokenLimit: parseLimit(daily),
      // credit 阈值允许小数（上游按 0.0x 计价），单独一条校验：非负有限数。
      dailyCreditLimit: parseCredit(credit),
      modelDailyTokenLimit: parseLimit(perModel),
    }),
    [reserve, daily, credit, perModel],
  );
  const invalid =
    parsed.reserveCredits === null ||
    parsed.dailyTokenLimit === null ||
    parsed.dailyCreditLimit === null ||
    parsed.modelDailyTokenLimit === null;
  const dirty =
    !!limits &&
    !invalid &&
    (parsed.reserveCredits !== limits.reserveCredits ||
      parsed.dailyTokenLimit !== limits.dailyTokenLimit ||
      parsed.dailyCreditLimit !== limits.dailyCreditLimit ||
      parsed.modelDailyTokenLimit !== limits.modelDailyTokenLimit);

  const accounts = quota?.accounts ?? [];
  const blockedCount = accounts.filter((row) => row.blocked).length;

  return (
    <Card className="overflow-hidden">
      <CardHead
        title={t("qoder.gateway.quota.title")}
        caption={t("qoder.gateway.quota.desc")}
        actions={
          <Button
            variant="ghost"
            size="sm"
            disabled={refreshing}
            onClick={onRefresh}
            data-act="quota-refresh"
          >
            {refreshing ? (
              <Loader2 className="size-3.5 animate-spin" />
            ) : (
              <RefreshCw className="size-3.5" />
            )}
            {t("qoder.gateway.quota.refresh")}
          </Button>
        }
      />

      <div className="grid gap-3 px-4 py-4 sm:grid-cols-2 lg:grid-cols-4">
        <LimitField
          testId="quota-reserve"
          label={t("qoder.gateway.quota.reserveLabel")}
          hint={t("qoder.gateway.quota.reserveHint")}
          value={reserve}
          invalid={parsed.reserveCredits === null}
          onChange={setReserve}
        />
        <LimitField
          testId="quota-daily"
          label={t("qoder.gateway.quota.dailyLabel")}
          hint={t("qoder.gateway.quota.dailyHint")}
          value={daily}
          invalid={parsed.dailyTokenLimit === null}
          onChange={setDaily}
        />
        <LimitField
          testId="quota-credit"
          label={t("qoder.gateway.quota.creditLabel")}
          hint={t("qoder.gateway.quota.creditHint")}
          value={credit}
          invalid={parsed.dailyCreditLimit === null}
          onChange={setCredit}
        />
        <LimitField
          testId="quota-model"
          label={t("qoder.gateway.quota.modelLabel")}
          hint={t("qoder.gateway.quota.modelHint")}
          value={perModel}
          invalid={parsed.modelDailyTokenLimit === null}
          onChange={setPerModel}
        />
      </div>

      <div className="flex flex-wrap items-center justify-between gap-2 border-t border-border/60 px-4 py-3">
        <p className="min-w-0 text-[11px] leading-4 text-muted-foreground">
          {invalid
            ? t("qoder.gateway.quota.invalid")
            : limits?.enabled
              ? t("qoder.gateway.quota.noteOn")
              : t("qoder.gateway.quota.noteOff")}
        </p>
        <Button
          size="sm"
          disabled={!dirty || saving}
          onClick={() =>
            onSave({
              reserveCredits: parsed.reserveCredits ?? 0,
              dailyTokenLimit: parsed.dailyTokenLimit ?? 0,
              dailyCreditLimit: parsed.dailyCreditLimit ?? 0,
              modelDailyTokenLimit: parsed.modelDailyTokenLimit ?? 0,
            })
          }
          data-act="quota-save"
        >
          {saving && <Loader2 className="size-3.5 animate-spin" />}
          {t("qoder.gateway.quota.save")}
        </Button>
      </div>

      {/* 逐账号当日用量：**只在拿到视图后渲染**（拿不到就明说，不摆空表）。 */}
      {quota === null ? (
        <div className="border-t border-border/60 px-4 py-4 text-[12px] text-muted-foreground">
          {t("qoder.gateway.quota.loading")}
        </div>
      ) : accounts.length === 0 ? (
        <div className="border-t border-border/60 px-4 py-4 text-[12px] text-muted-foreground">
          {t("qoder.gateway.quota.empty")}
        </div>
      ) : (
        <div className="border-t border-border/60">
          <div className="flex items-center justify-between gap-2 px-4 pt-3 text-[11.5px] text-muted-foreground">
            <span>
              {t("qoder.gateway.quota.today", {
                date: quota.date,
                count: accounts.length,
              })}
            </span>
            <span className="flex items-center gap-1.5">
              <ShieldCheck className="size-3.5" />
              {blockedCount > 0
                ? t("qoder.gateway.quota.blockedCount", { count: blockedCount })
                : t("qoder.gateway.quota.allClear")}
            </span>
          </div>
          <div className="divide-y divide-border/60 px-4 pb-3 pt-1">
            {accounts.map((row) => (
              <div key={row.uid} className="flex items-start justify-between gap-3 py-2">
                <div className="min-w-0">
                  <div className="truncate text-[12.5px]">{row.name || row.uid}</div>
                  <div className="mt-0.5 text-[11px] text-muted-foreground">
                    {row.perModel.length > 0
                      ? row.perModel
                          .slice(0, 3)
                          .map((item) => `${item.model} ${item.tokens.toLocaleString()}`)
                          .join(" · ")
                      : t("qoder.gateway.quota.noModelUsage")}
                    {row.credits > 0
                      ? ` · ${t("qoder.gateway.quota.creditsUsed", {
                          n: row.credits.toFixed(2),
                        })}`
                      : ""}
                    {row.creditsRemaining !== null && row.creditsRemaining !== undefined
                      ? ` · ${t("qoder.gateway.quota.credits", {
                          n: row.creditsRemaining.toLocaleString(),
                        })}`
                      : ""}
                  </div>
                </div>
                <div className="flex shrink-0 items-center gap-2">
                  <span className="text-[12px] tabular-nums">
                    {t("qoder.gateway.quota.tokens", { n: row.tokens.toLocaleString() })}
                  </span>
                  {row.blocked ? (
                    <Badge variant="warning" className="shrink-0" title={row.blockedReason ?? ""}>
                      {t(BLOCK_LABELS[row.blocked] ?? "qoder.gateway.quota.why.other")}
                    </Badge>
                  ) : (
                    <Badge variant="success" className="shrink-0">
                      {t("qoder.gateway.quota.ok")}
                    </Badge>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </Card>
  );
}
