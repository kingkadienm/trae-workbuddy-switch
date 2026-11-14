import { useEffect, useState } from "react";
import { ShieldAlert } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { getAppPrefs, isDesktop, setAppPrefs } from "@/lib/api";
import { demoModeEnabled } from "@/lib/demo-mode";
import { useT } from "@/lib/i18n";

/**
 * 首启「风险确认」弹层。
 *
 * ## 为什么要有它
 *
 * README / `docs/policies/DISCLAIMER.md` 里的免责声明是**单向告知**：用户不点开文件
 * 就等于没看过，也就谈不上「合意」。这个弹层把告知变成一次**显式的勾选动作**，
 * 并把确认时间落进 `app_prefs.json` 的 `legal_consent_at`（0 = 未确认）——
 * 它是「作者 ↔ 用户」之间风险分配的凭证，对上游权利人没有任何对抗力，别指望它更多。
 *
 * ## 只在桌面包里出现
 *
 * - **webui**：没有宿主进程，`app_prefs.json` 无人维护 ⇒ 无法持久化，弹了也只会每次都弹；
 * - **demo / 截图模式**：公开演示站与 README 截图（`VITE_SCREENSHOT_DEMO=1`）要直接截页面，
 *   一个全屏弹层会把所有截图变成同一张「使用前请确认」。
 *
 * ## 三条硬约束
 *
 * 1. **不能默认已同意**：读失败、返回 0、脏值都一律弹（宁可多问一次）。
 * 2. **不能假装保存成功**：按后端**回读值**判断，`legal_consent_at` 仍为 0 就是失败，
 *    保持弹层打开并报错。后端归一化会把字符串 / 负数 / 小数一律打回 0。
 * 3. **不能写成可关的提示**：关掉按钮与 Esc / 点外部全部禁用 —— 可关的「确认」等于没有确认。
 *    想拒绝的用户看到的是「如不同意，请关闭并卸载本应用」。
 */
export function LegalConsentDialog() {
  const t = useT();
  const [open, setOpen] = useState(false);
  const [checked, setChecked] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (demoModeEnabled || !isDesktop()) return;
    let cancelled = false;
    void getAppPrefs()
      .then((prefs) => {
        if (!cancelled && !(prefs.legal_consent_at > 0)) setOpen(true);
      })
      // 读不到偏好 ⇒ 也不能当作「已确认」：弹一次，让用户在本次会话里明确表态。
      .catch(() => {
        if (!cancelled) setOpen(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function onAccept() {
    setSaving(true);
    setError(null);
    try {
      /* 先读回完整偏好：`set_app_prefs` 按已知键归一化，只传 `legal_consent_at`
         会把 `close_to_tray` / `watch_agent_sessions` 冲回默认值。 */
      const prefs = await getAppPrefs();
      const saved = await setAppPrefs({ ...prefs, legal_consent_at: Date.now() });
      if (!(saved.legal_consent_at > 0)) throw new Error("consent not persisted");
      setOpen(false);
    } catch {
      setError(t("legalConsent.saveFailed"));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Dialog open={open}>
      <DialogContent
        showCloseButton={false}
        width="wide"
        onEscapeKeyDown={(event) => event.preventDefault()}
        onPointerDownOutside={(event) => event.preventDefault()}
        onInteractOutside={(event) => event.preventDefault()}
      >
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <ShieldAlert className="size-4 text-amber-500" aria-hidden="true" />
            {t("legalConsent.title")}
          </DialogTitle>
          <DialogDescription>{t("legalConsent.intro")}</DialogDescription>
        </DialogHeader>

        <ul className="space-y-2 text-[12.5px] leading-relaxed">
          <li className="flex gap-2">
            <span aria-hidden="true" className="text-muted-foreground">
              ·
            </span>
            <span>{t("legalConsent.item.accounts")}</span>
          </li>
          <li className="flex gap-2">
            <span aria-hidden="true" className="text-muted-foreground">
              ·
            </span>
            <span>{t("legalConsent.item.automation")}</span>
          </li>
          <li className="flex gap-2">
            <span aria-hidden="true" className="text-muted-foreground">
              ·
            </span>
            <span>{t("legalConsent.item.gateway")}</span>
          </li>
        </ul>

        <p className="text-muted-foreground text-[12.5px] leading-relaxed">
          {t("legalConsent.consequence")}
        </p>

        <label className="flex cursor-pointer items-start gap-2.5 text-[12.5px] leading-relaxed">
          <Checkbox
            checked={checked}
            onCheckedChange={(value) => setChecked(value === true)}
            className="mt-[1px]"
          />
          <span>{t("legalConsent.checkbox")}</span>
        </label>

        {error ? (
          <p className="text-destructive text-[12.5px] leading-relaxed" role="alert">
            {error}
          </p>
        ) : null}

        <DialogFooter>
          {/* 未勾选时禁用：确认动作必须是用户主动发出的，不能靠默认焦点 + 回车蒙过去。 */}
          <Button type="button" disabled={!checked || saving} onClick={() => void onAccept()}>
            {saving ? t("legalConsent.accepting") : t("legalConsent.accept")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
