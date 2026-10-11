import { useState, useEffect } from "react";
import { Loader2, Copy, Check, Shield, KeyRound } from "lucide-react";
import QRCode from "qrcode";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import * as api from "@/lib/api";

type Step = "setup-secret" | "setup-verify" | "login";

interface TotpAuthPageProps {
    mode: "setup" | "login";
    onSuccess: () => void;
}

/** 全屏 TOTP 认证页面：不可关闭，必须完成验证才能进入应用。 */
export function TotpAuthPage({ mode, onSuccess }: TotpAuthPageProps) {
    const [step, setStep] = useState<Step>(mode === "setup" ? "setup-secret" : "login");
    const [secret, setSecret] = useState("");
    const [otpauthUri, setOtpauthUri] = useState("");
    const [qrDataUrl, setQrDataUrl] = useState("");
    const [code, setCode] = useState("");
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState("");
    const [copied, setCopied] = useState(false);

    // 进入页面时加载密钥（setup 模式）
    useEffect(() => {
        if (mode === "login") return;
        if (step !== "setup-secret") return;
        setBusy(true);
        setError("");
        api
            .totpSetup()
            .then((res) => {
                setSecret(res.secret);
                setOtpauthUri(res.otpauthUri || "");
                if (res.otpauthUri) {
                    QRCode.toDataURL(res.otpauthUri, { width: 220, margin: 2 })
                        .then(setQrDataUrl)
                        .catch(() => setQrDataUrl(""));
                }
            })
            .catch((err) => setError(api.asError(err)))
            .finally(() => setBusy(false));
    }, [mode, step]);

    function copySecret() {
        if (!secret) return;
        navigator.clipboard.writeText(secret);
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
    }

    function handleVerify(e: React.FormEvent) {
        e.preventDefault();
        if (busy || code.length !== 6) return;
        setBusy(true);
        setError("");

        const promise = mode === "setup"
            ? api.totpVerify(code)
            : api.totpValidate(code);

        promise
            .then((res: { ok: boolean; sessionToken?: string }) => {
                if (mode === "login" && res.sessionToken) {
                    sessionStorage.setItem("totp_session", res.sessionToken);
                }
                onSuccess();
            })
            .catch((err) => setError(api.asError(err)))
            .finally(() => setBusy(false));
    }

    const isSetup = mode === "setup";

    return (
        <div className="flex min-h-screen items-center justify-center bg-background">
            <div className="w-full max-w-md space-y-6 px-4">
                {/* 标题 */}
                <div className="flex flex-col items-center gap-2 text-center">
                    <div className="flex h-12 w-12 items-center justify-center rounded-full bg-primary/10">
                        <Shield className="h-6 w-6 text-primary" />
                    </div>
                    <h1 className="text-xl font-semibold tracking-tight">
                        {isSetup ? "设置两步验证" : "两步验证登录"}
                    </h1>
                    <p className="text-sm text-muted-foreground">
                        {isSetup
                            ? "使用 Google Authenticator 扫描二维码或手动输入密钥"
                            : "请输入 Google Authenticator 中的 6 位动态口令"}
                    </p>
                </div>

                {/* 加载态 */}
                {busy && !secret && isSetup ? (
                    <div className="flex items-center justify-center py-12">
                        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
                    </div>
                ) : step === "setup-secret" && isSetup ? (
                    /* Setup 第一步：展示密钥 + 二维码 */
                    <div className="space-y-4">
                        {qrDataUrl && (
                            <div className="flex justify-center">
                                <img src={qrDataUrl} alt="TOTP QR Code" className="rounded-lg border" />
                            </div>
                        )}
                        <Alert>
                            <KeyRound className="h-4 w-4" />
                            <AlertTitle>密钥</AlertTitle>
                            <AlertDescription className="flex items-center gap-2">
                                <code className="flex-1 break-all font-mono text-xs">{secret}</code>
                                <Button
                                    variant="ghost"
                                    size="icon"
                                    onClick={copySecret}
                                    title="复制密钥"
                                >
                                    {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
                                </Button>
                            </AlertDescription>
                        </Alert>
                        {otpauthUri && (
                            <p className="text-xs text-muted-foreground break-all">{otpauthUri}</p>
                        )}
                        <p className="text-sm text-muted-foreground">
                            打开 Google Authenticator，选择「扫描二维码」或「手动输入密钥」，添加后请输入验证码确认。
                        </p>
                        <Button variant="outline" className="w-full" onClick={() => setStep("setup-verify")}>
                            已添加，下一步
                        </Button>
                    </div>
                ) : (
                    /* Setup 第二步 / Login：输入验证码 */
                    <form onSubmit={handleVerify}>
                        <div className="space-y-4">
                            <div className="space-y-2">
                                <Label htmlFor="totp-code">
                                    {isSetup && step === "setup-verify" ? "验证首次口令" : "验证码"}
                                </Label>
                                <Input
                                    id="totp-code"
                                    value={code}
                                    onChange={(e) => setCode(e.target.value.replace(/\D/g, "").slice(0, 6))}
                                    placeholder="000000"
                                    maxLength={6}
                                    autoFocus
                                    disabled={busy}
                                    className="text-center text-2xl tracking-widest"
                                />
                            </div>
                            {error && <p className="text-sm text-red-500">{error}</p>}
                            {isSetup && step === "setup-verify" && (
                                <Button
                                    type="button"
                                    variant="ghost"
                                    className="w-full"
                                    onClick={() => setStep("setup-secret")}
                                    disabled={busy}
                                >
                                    返回查看密钥
                                </Button>
                            )}
                        </div>
                        <Button type="submit" className="w-full mt-4" disabled={busy || code.length !== 6}>
                            {busy && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
                            {isSetup ? "验证并启用" : "验证"}
                        </Button>
                    </form>
                )}
            </div>
        </div>
    );
}
