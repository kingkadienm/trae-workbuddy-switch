import { useEffect, useState, type ReactNode } from "react";
import { Loader2, RefreshCw } from "lucide-react";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { TotpAuthPage } from "@/components/totp-auth-page";
import * as api from "@/lib/api";

/** WebUI 认证完成后才挂载内容；路由、快捷键和浮层都不能越过登录页。 */
export function WebuiAuthGate({ children }: { children: ReactNode }) {
  const bypass = api.isDesktop() || api.isDemoMode();
  const [authenticated, setAuthenticated] = useState(bypass);
  const [checking, setChecking] = useState(!bypass);
  const [error, setError] = useState("");
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    if (bypass) return;
    let cancelled = false;
    api.setUnauthorizedHandler(() => {
      sessionStorage.removeItem("totp_session");
      setAuthenticated(false);
    });
    setChecking(true);
    setError("");
    void api.totpStatus().then(async (status) => {
      if (cancelled) return;
      if (!status.enabled) {
        setAuthenticated(true);
        return;
      }
      if (!sessionStorage.getItem("totp_session")) return;
      // 由服务端校验会话；本地残留字符串不构成已登录的证据。
      try {
        await api.getStatus("cn");
        if (!cancelled) setAuthenticated(true);
      } catch {
        if (!cancelled) setAuthenticated(false);
      }
    }).catch((e) => {
      if (!cancelled) setError(api.asError(e));
    }).finally(() => {
      if (!cancelled) setChecking(false);
    });
    return () => { cancelled = true; };
  }, [attempt, bypass]);

  if (authenticated) return <>{children}</>;
  if (checking) return <div className="grid min-h-screen place-items-center bg-background" role="status"><Loader2 className="size-8 animate-spin" aria-label="正在检查登录状态" /></div>;
  if (error) return (
    <div className="grid min-h-screen place-items-center bg-background p-6">
      <Alert variant="destructive" className="max-w-md">
        <AlertDescription className="space-y-3">
          <p>检查登录状态失败：{error}</p>
          <Button variant="outline" onClick={() => setAttempt((value) => value + 1)}><RefreshCw />重试</Button>
        </AlertDescription>
      </Alert>
    </div>
  );
  return <TotpAuthPage mode="login" onSuccess={() => setAuthenticated(true)} />;
}
