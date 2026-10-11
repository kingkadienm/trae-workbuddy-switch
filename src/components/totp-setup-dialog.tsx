import { useState, useEffect } from "react";
import { Loader2, Copy, Check } from "lucide-react";

import {
    Dialog,
    DialogContent,
    DialogDescription,
    DialogFooter,
    DialogHeader,
    DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import * as api from "@/lib/api";

interface Props {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    onSuccess: () => void;
}

type Step = "secret" | "verify";

/** 两步验证首次设置弹框：展示密钥 → 用户录入验证码 → 启用。 */
export function TotpSetupDialog({ open, onOpenChange, onSuccess }: Props) {
    const [step, setStep] = useState<Step>("secret");
    const [secret, setSecret] = useState("");
    const [otpauthUri, setOtpauthUri] = useState("");
    const [code, setCode] = useState("");
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState("");
    const [copied, setCopied] = useState(false);

    function reset() {
        setStep("secret");
        setSecret("");
        setOtpauthUri("");
        setCode("");
        setError("");
        setBusy(false);
        setCopied(false);
    }

    // 打开时生成新密钥
    useEffect(() => {
        if (open && !secret) {
            setBusy(true);
            setError("");
            api
                .totpSetup()
                .then((res) => {
                    setSecret(res.secret);
                    setOtpauthUri(res.otpauthUri || "");
                })
                .catch((err) => {
                    setError(api.asError(err));
                })
                .finally(() => setBusy(false));
        }
        if (!open) reset();
    }, [open]);

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
        api.totpVerify(code)
            .then(() => {
                onSuccess();
                onOpenChange(false);
            })
            .catch((err) => {
                setError(api.asError(err));
            })
            .finally(() => setBusy(false));
    }

    return (
        <Dialog open={open} onOpenChange={onOpenChange}>
            <DialogContent className="sm:max-w-sm">
                <DialogHeader>
                    <DialogTitle>设置两步验证</DialogTitle>
                    <DialogDescription>
                        使用 Google Authenticator 扫描二维码或手动输入下方密钥
                    </DialogDescription>
                </DialogHeader>

                {busy && !secret ? (
                    <div className="flex items-center justify-center py-8">
                        <Loader2 className="h-8 w-8 animate-spin" />
                    </div>
                ) : step === "secret" ? (
                    <div className="space-y-4">
                        <Alert>
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
                            <p className="text-xs text-muted-foreground break-all">
                                {otpauthUri}
                            </p>
                        )}
                        <p className="text-sm text-muted-foreground">
                            打开 Google Authenticator，选择「扫描二维码」或「手动输入密钥」，添加后请输入验证码确认。
                        </p>
                        <Button variant="outline" className="w-full" onClick={() => setStep("verify")}>
                            已添加，下一步
                        </Button>
                    </div>
                ) : (
                    <form onSubmit={handleVerify}>
                        <div className="space-y-4">
                            <div className="space-y-2">
                                <Label htmlFor="verify-code">验证码</Label>
                                <Input
                                    id="verify-code"
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
                        </div>
                        <DialogFooter className="mt-4">
                            <Button
                                type="button"
                                variant="outline"
                                onClick={() => setStep("secret")}
                                disabled={busy}
                            >
                                上一步
                            </Button>
                            <Button type="submit" disabled={busy || code.length !== 6}>
                                {busy && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
                                验证
                            </Button>
                        </DialogFooter>
                    </form>
                )}
            </DialogContent>
        </Dialog>
    );
}
