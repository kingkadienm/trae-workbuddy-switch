import { useCallback, useMemo, useState } from "react";
import {
  Bot,
  Crown,
  Loader2,
  Plus,
  RefreshCw,
  ScanSearch,
  Trash2,
  Timer,
  User,
  Users,
} from "lucide-react";
import { toast } from "sonner";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import * as api from "@/lib/api";
import { useT } from "@/lib/i18n";
import type { DoubaoAccount } from "@/lib/types";
import { cn } from "@/lib/utils";
import { useCachedResource } from "@/lib/use-cached-resource";

/** 会话状态对应的中文/英文徽标语义（走 t() 翻译）。 */
function sessionBadgeClass(state: DoubaoAccount["sessionState"]) {
  switch (state) {
    case "ok":
      return "border-emerald-500/30 bg-emerald-500/15 text-emerald-700 dark:text-emerald-300";
    case "expired":
      return "border-rose-500/30 bg-rose-500/15 text-rose-700 dark:text-rose-300";
    case "unknown":
      return "border-amber-500/30 bg-amber-500/15 text-amber-700 dark:text-amber-300";
    default:
      return "border-zinc-500/30 bg-zinc-500/15 text-zinc-600 dark:text-zinc-400";
  }
}

function sessionStateLabel(t: ReturnType<typeof useT>, state: DoubaoAccount["sessionState"]) {
  switch (state) {
    case "ok":
      return t("doubao.session.ok");
    case "expired":
      return t("doubao.session.expired");
    case "unknown":
      return t("doubao.session.unknown");
    default:
      return t("doubao.session.none");
  }
}

export function DoubaoAccountsPage() {
  const t = useT();
  const { data, refresh, loading } = useCachedResource<{ accounts: DoubaoAccount[] }>(
    "doubao:accounts",
    () => api.doubaoAccountsList(),
    { freshMs: 5000 },
  );

  const accounts = useMemo(() => data?.accounts ?? [], [data]);
  const current = useMemo(() => accounts.find((a) => a.isCurrent), [accounts]);

  const [detecting, setDetecting] = useState(false);
  const [saving, setSaving] = useState(false);
  const [keepaliving, setKeepaliving] = useState(false);

  const [newAccountDialog, setNewAccountDialog] = useState(false);
  const [newUserId, setNewUserId] = useState("");
  const [newName, setNewName] = useState("");
  const [newNote, setNewNote] = useState("");
  const [removeSnapshot, setRemoveSnapshot] = useState(true);

  const [removeTarget, setRemoveTarget] = useState<DoubaoAccount | null>(null);
  const [removing, setRemoving] = useState(false);

  const handleDetect = useCallback(async () => {
    setDetecting(true);
    try {
      const result = await api.doubaoDetectUid();
      const uid = result.uid;
      if (uid) {
        toast.success(t("doubao.detect.success", { uid }), { id: "doubao-detect" });
      } else {
        toast.info(t("doubao.detect.empty"), { id: "doubao-detect" });
      }
    } catch (error) {
      toast.error(t("doubao.detect.error", { error: String(error) }), { id: "doubao-detect" });
    } finally {
      setDetecting(false);
    }
  }, [t]);

  const handleSave = useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      if (!newUserId.trim()) {
        toast.error(t("doubao.save.emptyUserId"));
        return;
      }
      setSaving(true);
      try {
        await api.doubaoAccountSave(newUserId.trim(), newName.trim() || undefined, newNote.trim() || undefined);
        toast.success(t("doubao.save.success", { uid: newUserId.trim() }));
        setNewAccountDialog(false);
        setNewUserId("");
        setNewName("");
        setNewNote("");
        setRemoveSnapshot(true);
        await refresh();
      } catch (error) {
        toast.error(t("doubao.save.error", { error: String(error) }));
      } finally {
        setSaving(false);
      }
    },
    [newUserId, newName, newNote, refresh],
  );

  const handleKeepalive = useCallback(async () => {
    setKeepaliving(true);
    try {
      await api.doubaoKeepaliveRun();
      toast.success(t("doubao.keepalive.success"));
      await refresh();
    } catch (error) {
      toast.error(t("doubao.keepalive.error", { error: String(error) }));
    } finally {
      setKeepaliving(false);
    }
  }, [refresh]);

  const handleRemove = useCallback(async () => {
    if (!removeTarget) return;
    setRemoving(true);
    try {
      await api.doubaoAccountRemove(removeTarget.userId, removeSnapshot);
      toast.success(t("doubao.remove.success", { uid: removeTarget.userId }));
      setRemoveTarget(null);
      await refresh();
    } catch (error) {
      toast.error(t("doubao.remove.error", { error: String(error) }));
    } finally {
      setRemoving(false);
    }
  }, [removeTarget, removeSnapshot, refresh]);

  return (
    <TooltipProvider delayDuration={250}>
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-4 p-4">
        <header className="flex flex-wrap items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <Bot className="size-5" />
            <h1 className="text-lg font-semibold">{t("doubao.title")}</h1>
            {current && (
              <Badge variant="secondary" className="gap-1">
                <User className="size-3" />
                <span className="max-w-48 truncate">{current.name}</span>
              </Badge>
            )}
          </div>
          <div className="flex items-center gap-2">
            <Button size="sm" variant="outline" onClick={handleDetect} disabled={detecting}>
              {detecting ? (
                <Loader2 className="mr-1 size-3.5 animate-spin" />
              ) : (
                <ScanSearch className="mr-1 size-3.5" />
              )}
              {t("doubao.detect.action")}
            </Button>
            <Button size="sm" variant="outline" onClick={handleKeepalive} disabled={keepaliving}>
              {keepaliving ? (
                <Loader2 className="mr-1 size-3.5 animate-spin" />
              ) : (
                <Timer className="mr-1 size-3.5" />
              )}
              {t("doubao.keepalive.action")}
            </Button>
            <Button size="sm" onClick={() => setNewAccountDialog(true)}>
              <Plus className="mr-1 size-3.5" />
              {t("doubao.addAccount")}
            </Button>
          </div>
        </header>

        <div className="grid gap-3">
          {loading && accounts.length === 0 ? (
            <>
              <Skeleton className="h-24" />
              <Skeleton className="h-24" />
            </>
          ) : accounts.length === 0 ? (
            <Card className="flex flex-col items-center justify-center gap-2 p-8 text-center">
              <Users className="size-8 text-muted-foreground/40" />
              <p className="text-sm text-muted-foreground">{t("doubao.empty")}</p>
              <Button size="sm" variant="outline" onClick={() => setNewAccountDialog(true)}>
                <Plus className="mr-1 size-3.5" />
                {t("doubao.addAccount")}
              </Button>
            </Card>
          ) : (
            accounts.map((account) => (
              <AccountRow
                key={account.userId}
                account={account}
                onRemove={() => {
                  setRemoveTarget(account);
                  setRemoveSnapshot(true);
                }}
              />
            ))
          )}
        </div>

        <Dialog open={newAccountDialog} onOpenChange={setNewAccountDialog}>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>{t("doubao.addDialogTitle")}</DialogTitle>
              <DialogDescription>{t("doubao.addDialogDescription")}</DialogDescription>
            </DialogHeader>
            <form onSubmit={handleSave} className="flex flex-col gap-3">
              <div className="flex flex-col gap-1">
                <Label>{t("doubao.fieldUserId")}</Label>
                <Input
                  value={newUserId}
                  onChange={(e) => setNewUserId(e.target.value)}
                  placeholder="user_id"
                  autoFocus
                />
              </div>
              <div className="flex flex-col gap-1">
                <Label>{t("doubao.fieldName")}</Label>
                <Input value={newName} onChange={(e) => setNewName(e.target.value)} placeholder="Work / 备用" />
              </div>
              <div className="flex flex-col gap-1">
                <Label>{t("doubao.fieldNote")}</Label>
                <Input value={newNote} onChange={(e) => setNewNote(e.target.value)} placeholder="备注（可选）" />
              </div>
              <DialogFooter className="mt-2">
                <Button type="button" variant="outline" onClick={() => setNewAccountDialog(false)}>
                  {t("doubao.cancel")}
                </Button>
                <Button type="submit" disabled={saving}>
                  {saving ? <Loader2 className="mr-1 size-3.5 animate-spin" /> : <RefreshCw className="mr-1 size-3.5" />}
                  {t("doubao.save.submit")}
                </Button>
              </DialogFooter>
            </form>
          </DialogContent>
        </Dialog>

        <Dialog open={removeTarget !== null} onOpenChange={(open) => !open && setRemoveTarget(null)}>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>{t("doubao.removeDialogTitle")}</DialogTitle>
              <DialogDescription>{t("doubao.removeDialogDescription", { uid: removeTarget?.userId ?? "" })}</DialogDescription>
            </DialogHeader>
            <div className="flex items-center gap-2">
              <Switch
                id="doubao-remove-snapshot"
                checked={removeSnapshot}
                onCheckedChange={setRemoveSnapshot}
              />
              <Label htmlFor="doubao-remove-snapshot">{t("doubao.removeSnapshotLabel")}</Label>
            </div>
            <DialogFooter>
              <Button variant="outline" onClick={() => setRemoveTarget(null)}>
                {t("doubao.cancel")}
              </Button>
              <Button variant="destructive" onClick={handleRemove} disabled={removing}>
                {removing ? <Loader2 className="mr-1 size-3.5 animate-spin" /> : <Trash2 className="mr-1 size-3.5" />}
                {t("doubao.remove.submit")}
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </div>
    </TooltipProvider>
  );
}

function AccountRow({ account, onRemove }: { account: DoubaoAccount; onRemove: () => void }) {
  const t = useT();
  return (
    <Card className="p-4">
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-3">
          <div className="flex size-9 shrink-0 items-center justify-center rounded-full bg-zinc-100 dark:bg-zinc-800">
            <User className="size-4 text-muted-foreground" />
          </div>
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <span className="truncate text-sm font-medium">{account.name}</span>
              {account.isCurrent && <Badge variant="secondary">{t("doubao.current")}</Badge>}
              <Badge className={cn("border", sessionBadgeClass(account.sessionState))}>
                {sessionStateLabel(t, account.sessionState)}
              </Badge>
              {account.quotaLevel && (
                <Badge variant="outline" className="gap-1">
                  <Crown className="size-3" />
                  {account.quotaLevel}
                </Badge>
              )}
            </div>
            <div className="truncate text-xs text-muted-foreground">{account.userId}</div>
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-1 text-xs text-muted-foreground">
          <span className="tabular-nums">{formatBytes(account.sizeBytes)}</span>
          <span>·</span>
          <span>{account.fileCount}</span>
          <span>·</span>
          <Tooltip>
            <TooltipTrigger asChild>
              <span className="cursor-help underline decoration-dotted underline-offset-2">
                {account.lastModified}
              </span>
            </TooltipTrigger>
            <TooltipContent>{t("doubao.lastModifiedTooltip")}</TooltipContent>
          </Tooltip>
          <Button
            variant="ghost"
            size="icon"
            className="ml-2"
            onClick={onRemove}
            aria-label={t("doubao.removeAction")}
          >
            <Trash2 className="size-4" />
          </Button>
        </div>
      </div>
    </Card>
  );
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(units.length - 1, Math.floor(Math.log2(bytes) / 10));
  const value = bytes / 2 ** (10 * i);
  return `${value.toFixed(value >= 100 || i === 0 ? 0 : 1)} ${units[i]}`;
}

export default DoubaoAccountsPage;
