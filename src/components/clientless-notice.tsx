/**
 * 无客户端形态的提示块（issue #62）—— 服务器 / 容器部署里，「没装客户端」是**正常形态**，
 * 不是故障。
 *
 * ## 为什么需要它
 *
 * 旧 UI 把「未检测到客户端」渲染成一张红字卡片 + 只留「重新检测 / 从本机导入」两个按钮。
 * 在服务器上这两条都是死路：没有客户端可检测、没有客户端目录可读 —— 于是用户既登不进去，
 * 也就没有账号可以喂给网关。
 *
 * 但账号与网关**只依赖凭据本身**：客户端提供的只是「现成的登录态来源」+「程序位切换」+
 * 「本地会话 / 用量文件」。所以这里做两件事：
 *
 * 1. **说清影响面**（可用 / 需要客户端 两列），避免用户以为整页都废了；
 * 2. **给出真正能走的路**：浏览器授权（链接 / 二维码，任意设备）与导入备份。
 *
 * ## 两个形态
 *
 * - `ClientlessEmptyCard`：一个账号都没有时占位（三条出路 + 能力矩阵）；
 * - `ClientlessStrip`：已经有账号、只是没有客户端时的一行说明（网关页同款，换一句文案）。
 *
 * ⚠️ 判据是**客户端是否存在**（`installed === false`），不是「本机是不是服务器」：
 * 桌面用户也可能只装了其中一个平台，两种情形 UI 与行为完全一样。
 */

import { useState, type ReactNode } from "react";
import { ArrowRight, Download, KeyRound, LogIn, TriangleAlert } from "lucide-react";

import { scopeLabel } from "@/components/scope-chip";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { useT } from "@/lib/i18n";
import type { Scope } from "@/lib/scope";
import { useScopeStatus } from "@/lib/use-scope-status";
import type { TranslationKey } from "@/locales/zh";
import { cn } from "@/lib/utils";

/** 可用能力的键（顺序即展示顺序）。 */
const CAP_OK = [
  "shared.srv.capOk.oauth",
  "shared.srv.capOk.accounts",
  "shared.srv.capOk.gateway",
  "shared.srv.capOk.context",
] as const;

/** 依赖客户端的能力（顺序即展示顺序）。 */
const CAP_OFF = [
  "shared.srv.capOff.local",
  "shared.srv.capOff.programs",
  "shared.srv.capOff.usage",
  "shared.srv.capOff.sessions",
] as const;

interface EmptyCardProps {
  /** 作用域显示名（如「WorkBuddy 国际版」）。 */
  scopeLabel: string;
  /** 平台显示名（如「WorkBuddy」），用于标题。 */
  platformLabel: string;
  /** Trae 特例：授权与续期都要客户端里的设备私钥，必须提前说明。 */
  trae?: boolean;
  /** 期望的认证文件路径（仅 WB 有；给「想手工拷贝」的人一条线索）。 */
  expectedAuthFile?: string;
  onOauth: () => void;
  onBackup: () => void;
  /** 粘贴授权凭据（WB 有后端命令；另两家走各自的 PAT / JWT 入口，不传）。 */
  onPaste?: () => void;
  /** 重新检测（客户端刚装好时用）。 */
  /** 重新检测（客户端刚装好时用）。自带 busy：调用方不必为一次点击维护状态。 */
  onRecheck?: () => void | Promise<void>;
  /** 平台专属的额外出路（如 Qoder 的「粘贴 PAT」），接在两张卡之后。 */
  extra?: ReactNode;
}

export function ClientlessEmptyCard({
  scopeLabel,
  platformLabel,
  trae,
  expectedAuthFile,
  onOauth,
  onBackup,
  onPaste,
  onRecheck,
  extra,
}: EmptyCardProps) {
  const t = useT();
  const [rechecking, setRechecking] = useState(false);
  return (
    <Card className="gap-0 overflow-hidden py-0">
      {/* 头部：一句话定性 —— 不是报错，是另一种部署形态 */}
      <div className="border-b border-border bg-primary/5 px-5 py-5">
        <span className="inline-flex items-center rounded-md border border-primary/30 bg-primary/10 px-1.5 py-0.5 text-[11px] font-medium text-primary">
          {t("shared.srv.badge")}
        </span>
        <h2 className="mt-2.5 text-[15px] font-semibold leading-snug">
          {t("shared.srv.card.title", { platform: platformLabel })}
        </h2>
        <p className="mt-1.5 max-w-[68ch] text-[12px] leading-relaxed text-muted-foreground">
          {t("shared.srv.card.desc")}
        </p>
      </div>

      {/* 出路：浏览器授权 + 导入备份（+ 平台专属） */}
      <div className="grid gap-2.5 px-5 py-4 sm:grid-cols-2">
        <PathCard
          primary
          icon={<LogIn className="size-4" aria-hidden="true" />}
          title={t("shared.srv.oauthTitle")}
          desc={t("shared.srv.oauthDesc")}
          onClick={onOauth}
        />
        <PathCard
          icon={<Download className="size-4" aria-hidden="true" />}
          title={t("shared.addDrawer.backupTitle")}
          desc={t("shared.srv.help.moveDesc")}
          onClick={onBackup}
        />
        {onPaste ? (
          <PathCard
            icon={<KeyRound className="size-4" aria-hidden="true" />}
            title={t("shared.srv.paste.title")}
            desc={t("shared.srv.paste.hint.wb")}
            onClick={onPaste}
          />
        ) : null}
        {extra}
      </div>

      {/* Trae 例外：先说清，别让用户走完授权才在最后一步失败 */}
      {trae ? (
        <div className="flex items-start gap-2 px-5 pb-3 text-[11.5px] leading-relaxed text-muted-foreground">
          <TriangleAlert className="mt-0.5 size-[13px] shrink-0 text-warning" aria-hidden="true" />
          <span>{t("shared.srv.traeNote")}</span>
        </div>
      ) : null}

      {/* 能力矩阵：**必须两列并排**，只列「不可用」会让用户以为整页都废了 */}
      <div className="grid border-t border-border sm:grid-cols-2">
        <CapColumn tone="ok" title={t("shared.srv.capOk")} keys={CAP_OK} />
        <CapColumn tone="off" title={t("shared.srv.capOff")} keys={CAP_OFF} className="sm:border-l sm:border-border" />
      </div>

      {/* 次级动作：客户端刚装好、或想手工拷贝认证文件的人 */}
      {(onRecheck || expectedAuthFile) && (
        <div className="flex flex-wrap items-center gap-2 border-t border-border px-5 py-3">
          {onRecheck ? (
            <Button
              size="sm"
              variant="outline"
              disabled={rechecking}
              onClick={async () => {
                setRechecking(true);
                try {
                  await onRecheck();
                } finally {
                  setRechecking(false);
                }
              }}
            >
              {t("wbAccounts.empty.recheck")}
            </Button>
          ) : null}
          {expectedAuthFile ? (
            <span className="min-w-0 text-[11px] text-muted-foreground">
              {t("wbAccounts.empty.expectedAuthFile")}
              <code className="ml-1 break-all font-mono">{expectedAuthFile}</code>
            </span>
          ) : null}
          <span className="ml-auto text-[11px] text-muted-foreground">{scopeLabel}</span>
        </div>
      )}
    </Card>
  );
}

/** 一张「出路」卡（与添加账号抽屉的 `.opt` 同几何，视觉上是同一套语言）。 */
function PathCard({
  icon,
  title,
  desc,
  onClick,
  primary,
}: {
  icon: ReactNode;
  title: string;
  desc: string;
  onClick: () => void;
  primary?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex w-full items-start gap-2.5 rounded-[11px] border bg-card-2 px-3 py-3 text-left transition-all",
        "hover:border-border-strong hover:bg-accent",
        primary ? "border-primary/45" : "border-border",
      )}
    >
      <span className="grid size-8 shrink-0 place-items-center rounded-[9px] bg-primary/15 text-primary">
        {icon}
      </span>
      <span className="min-w-0 flex-1">
        <b className="block text-[12.5px] font-[550] leading-tight">{title}</b>
        <span className="mt-0.5 block text-[11px] leading-snug text-muted-foreground">{desc}</span>
      </span>
      <ArrowRight className="mt-1 size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
    </button>
  );
}

function CapColumn({
  tone,
  title,
  keys,
  className,
}: {
  tone: "ok" | "off";
  title: string;
  keys: readonly TranslationKey[];
  className?: string;
}) {
  const t = useT();
  return (
    <div className={cn("px-5 py-3.5", className)}>
      <h3
        className={cn(
          "mb-2 text-[11px] font-semibold tracking-wide",
          tone === "ok" ? "text-primary" : "text-warning",
        )}
      >
        {title}
      </h3>
      <ul className="flex flex-col gap-1.5">
        {keys.map((key) => (
          <li key={key} className="flex items-start gap-1.5 text-[11.5px] leading-snug text-muted-foreground">
            <span
              className={cn(
                "mt-1.5 size-1 shrink-0 rounded-full",
                tone === "ok" ? "bg-primary/70" : "bg-warning/70",
              )}
              aria-hidden="true"
            />
            <span>{t(key)}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

/**
 * 一行说明（账号页「已有账号」与网关页共用）。
 *
 * 刻意**不放按钮**：这里要传达的是「什么不受影响」，点开一个抽屉只会打断用户；
 * 需要细节的人在空态卡片里已经能看到能力矩阵。
 */
export function ClientlessStrip({
  scopeLabel,
  kind = "accounts",
  className,
}: {
  scopeLabel: string;
  /** `gateway` 换一句「网关照常可用」，其余同形。 */
  kind?: "accounts" | "gateway";
  className?: string;
}) {
  const t = useT();
  return (
    <div
      className={cn(
        "mb-3.5 flex items-start gap-2 rounded-xl border border-primary/30 bg-primary/[0.07] px-3 py-2.5 text-[12px] leading-relaxed",
        className,
      )}
      role="status"
    >
      <span className="shrink-0 rounded-md border border-primary/30 bg-primary/10 px-1.5 py-0.5 text-[10.5px] font-medium text-primary">
        {t("shared.srv.badge")}
      </span>
      <span className="min-w-0">
        {t(kind === "gateway" ? "shared.srv.strip.gateway" : "shared.srv.strip.accounts", { scope: scopeLabel })}
      </span>
    </div>
  );
}

/**
 * 网关页的「无客户端」说明条 —— 自己取状态，调用方只传作用域。
 *
 * 三个产品线的网关页共用同一句结论：**网关只依赖账号池里的凭据**，与客户端是否同机无关。
 * 放在这里而不是各页各写一遍：三处文案必然漂移，而这条结论是 issue #62 的核心承诺。
 */
export function GatewayClientlessNotice({ scope }: { scope: Scope }) {
  const t = useT();
  const { statuses } = useScopeStatus();
  /* 状态未知（还没探测回来）时**不渲染**：宁可晚一句，也不要先喊「无客户端」再收回。 */
  const status = statuses[scope];
  if (!status || status.installed !== false) return null;
  return <ClientlessStrip scopeLabel={scopeLabel(t, scope)} kind="gateway" className="mb-4" />;
}
