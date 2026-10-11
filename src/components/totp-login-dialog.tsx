import { useState } from "react";
import { Loader2 } from "lucide-react";

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
import * as api from "@/lib/api";

interface Props {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    onSuccess: () => void;
}

/** 两步验证登录弹框：输入 Google Authenticator 6 位动态口令。 */
export function TotpLoginDialog({ open, onOpenChange, onSuccess }: Props) {
    const [code, setCode] = useState("");
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState("");

    function handleSubmit(e: React.FormEvent) {
        e.preventDefault();
        console.log("[TotpLoginDialog] handleSubmit called, code length:", code.length, "busy:", busy);
        if (busy || code.length !== 6) return;
        setBusy(true);
        setError("");
        console.log("[TotpLoginDialog] calling api.totpValidate...");
        api
            .totpValidate(code)
            .then((res) => {
                console.log("[TotpLoginDialog] totpValidate success:", res);
                sessionStorage.setItem("totp_session", res.sessionToken);
                setCode("");
                onSuccess();
                onOpenChange(false);
            })
            .catch((err) => {
                console.log("[TotpLoginDialog] totpValidate error:", err);
                setError(api.asError(err));
            })
            .finally(() => {
                console.log("[TotpLoginDialog] totpValidate done");
                setBusy(false);
            });
    }

    return (
        <Dialog open={open} onOpenChange={onOpenChange}>
            <DialogContent className="sm:max-w-sm">
                <DialogHeader>
                    <DialogTitle>两步验证</DialogTitle>
                    <DialogDescription>请输入 Google Authenticator 中的 6 位动态口令</DialogDescription>
                </DialogHeader>
                <form onSubmit={handleSubmit}>
                    <div className="space-y-4">
                        <div className="space-y-2">
                            <Label htmlFor="totp-code">验证码</Label>
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
                    </div>
                    <DialogFooter className="mt-4">
                        <Button type="submit" disabled={busy || code.length !== 6} className="w-full">
                            {busy && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
                            验证
                        </Button>
                    </DialogFooter>
                </form>
            </DialogContent>
        </Dialog>
    );
}
