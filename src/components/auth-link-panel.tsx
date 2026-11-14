/**
 * 授权链接面板 —— 二维码 + 链接 + 复制 + 倒计时 + 三步说明。
 *
 * ## 为什么要有这个组件（issue #62）
 *
 * 旧的三个授权弹窗（WB / Trae / Qoder）都是「点一下 → `openInBrowser(uri)` → 轮询」，
 * 链接只是塞在 `Alert` 里的一段文字。在**服务器 / 容器**部署下这条路整条断掉：
 *
 * - 宿主没有 GUI，`openInBrowser` 必然失败（Tauri 的 opener 插件、`window.open` 都可能静默失败）；
 * - 用户的浏览器在**另一台机器**上，只有把链接 / 二维码送到他面前才有用。
 *
 * ⇒ 弹窗本身就是完整路径：链接可复制、二维码可扫、状态自己走。
 * 桌面形态只是「额外帮你自动开一次浏览器」，失败不报错、不影响这里。
 *
 * ## 为什么三平台共用一份
 *
 * 交互链同构（发起 → 展示 → 轮询 → 入库），平台差异只有「额外说明块」
 * （Trae 要讲设备私钥与 `127.0.0.1:17388`），所以差异走 `children` 注入，
 * 而不是复制三份面板 —— 复制三份的下场是三处各自漂移（本仓踩过）。
 *
 * ## 二维码是**真二维码**
 *
 * 原型里那套是示意点阵（不引依赖），落地版用 `qrcode` 渲染后端给的
 * `verificationUri`：手机扫码即可打开授权页，这是远程部署最省事的一条路。
 * 渲染失败（极端情况）**不报错**：面板仍有可复制的链接，二维码位置留空。
 */

import { useEffect, useRef, useState } from "react";
import QRCode from "qrcode";
import { Check, Copy, Loader2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { useT } from "@/lib/i18n";
import { cn } from "@/lib/utils";

export type AuthLinkStatus = "waiting" | "expired" | "done";

interface Props {
  /** 后端构造的授权页地址（前端**不拼参数**：各区域 / 产品线的形态都不同）。 */
  url: string;
  /** 后端给的过期秒数；缺省则不显示倒计时。 */
  expiresIn?: number | null;
  status: AuthLinkStatus;
  /** 平台专属补充块（Trae 的端口转发说明等）。 */
  children?: React.ReactNode;
  className?: string;
}

/** 三步说明的键。`as const` 是必须的：`t()` 收的是**字面量联合类型**，
 *  写成 `string[]` 会退化成 `string` 而编译不过（`TranslationKey` 不接受任意串）。 */
const AUTH_STEPS = [
  "shared.authPanel.step1",
  "shared.authPanel.step2",
  "shared.authPanel.step3",
] as const;

const fmtLeft = (sec: number) => `${Math.floor(sec / 60)}:${String(sec % 60).padStart(2, "0")}`;

export function AuthLinkPanel({ url, expiresIn, status, children, className }: Props) {
  const t = useT();
  const [qr, setQr] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [left, setLeft] = useState<number | null>(expiresIn ?? null);
  const copyTimer = useRef<number | undefined>(undefined);

  /* 二维码：URL 一变就重画（重新生成授权链接后旧码必须失效，否则用户会扫到作废的 state）。 */
  useEffect(() => {
    let cancelled = false;
    setQr(null);
    if (!url) return;
    QRCode.toDataURL(url, { margin: 1, width: 360, errorCorrectionLevel: "M" })
      .then((data) => {
        if (!cancelled) setQr(data);
      })
      .catch(() => {
        // 渲染失败只影响二维码，链接与轮询照旧 —— 不把「画不出码」升级成「登录失败」。
        if (!cancelled) setQr(null);
      });
    return () => {
      cancelled = true;
    };
  }, [url]);

  /* 倒计时：只在等待态走；`status` 变化即停（超时后继续转圈会让界面自相矛盾）。 */
  useEffect(() => {
    setLeft(expiresIn ?? null);
    if (expiresIn == null || status !== "waiting") return;
    const timer = window.setInterval(() => {
      setLeft((prev) => {
        if (prev == null) return prev;
        if (prev <= 1) {
          window.clearInterval(timer);
          return 0;
        }
        return prev - 1;
      });
    }, 1000);
    return () => window.clearInterval(timer);
  }, [expiresIn, status, url]);

  useEffect(() => () => window.clearTimeout(copyTimer.current), []);

  async function copy() {
    try {
      await navigator.clipboard.writeText(url);
      setCopied(true);
      window.clearTimeout(copyTimer.current);
      copyTimer.current = window.setTimeout(() => setCopied(false), 1600);
    } catch {
      // 剪贴板在非安全上下文（http + 非 localhost）会被拒 —— 此时链接本身仍可选中复制。
      setCopied(false);
    }
  }

  return (
    <div className={cn("grid gap-4 sm:grid-cols-[176px_minmax(0,1fr)]", className)}>
      {/* 左列：二维码。白底是**必须**的（深色主题下反色二维码很多扫码器读不出）。 */}
      <div className="flex flex-col items-center gap-2">
        <div className="grid size-[176px] place-items-center overflow-hidden rounded-xl bg-white p-2 shadow-[0_0_0_1px_var(--border)]">
          {qr ? (
            <img src={qr} alt={t("shared.authPanel.qrAlt")} className="size-full" />
          ) : (
            <span className="px-2 text-center text-[10.5px] leading-snug text-neutral-500">
              {t("shared.authPanel.copy")}
            </span>
          )}
        </div>
        <p className="text-center text-[10.5px] leading-snug text-muted-foreground">
          {t("shared.authPanel.scanHint")}
        </p>
      </div>

      {/* 右列：状态 + 链接 + 步骤 + 平台补充块 */}
      <div className="flex min-w-0 flex-col gap-3">
        <div className="flex items-center gap-2 text-[12px]">
          {status === "done" ? (
            <span className="inline-flex items-center gap-1.5 font-medium text-primary">
              <Check className="size-3.5" aria-hidden="true" />
              {t("shared.authPanel.authorized")}
            </span>
          ) : status === "expired" ? (
            <>
              <span className="rounded-md border border-warning/40 bg-warning/10 px-1.5 py-0.5 text-[11px] font-medium text-warning">
                {t("shared.authPanel.expired")}
              </span>
              <span className="text-[11px] text-muted-foreground">{t("shared.authPanel.expiredDesc")}</span>
            </>
          ) : (
            <>
              <Loader2 className="size-3.5 animate-spin text-muted-foreground" aria-hidden="true" />
              <span>{t("shared.authPanel.waiting")}</span>
              {left != null ? (
                <span className="ml-auto font-mono text-[11px] tabular-nums text-muted-foreground">
                  {t("shared.authPanel.remaining", { time: fmtLeft(left) })}
                </span>
              ) : null}
            </>
          )}
        </div>

        <div className="flex items-center gap-2 rounded-[10px] border border-border bg-card-2 px-2.5 py-2">
          <a
            href={url}
            target="_blank"
            rel="noreferrer"
            className="min-w-0 flex-1 truncate font-mono text-[11px] text-muted-foreground hover:text-foreground"
            title={url}
          >
            {url}
          </a>
          <Button
            type="button"
            size="sm"
            variant="ghost"
            className="h-6 shrink-0 px-2 text-[11px]"
            onClick={() => void copy()}
          >
            {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
            {copied ? t("shared.authPanel.copied") : t("shared.authPanel.copy")}
          </Button>
        </div>

        <ol className="flex flex-col gap-2 text-[11.5px] leading-relaxed">
          {AUTH_STEPS.map((key, index) => (
            <li key={key} className="flex gap-2.5">
              <span className="mt-px grid size-[18px] shrink-0 place-items-center rounded-full bg-primary/15 text-[10px] font-semibold text-primary">
                {index + 1}
              </span>
              <span className="text-muted-foreground">{t(key)}</span>
            </li>
          ))}
        </ol>

        {children}
      </div>
    </div>
  );
}
