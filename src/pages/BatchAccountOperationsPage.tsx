import { useCallback, useEffect, useState } from "react";
import { Download, Loader2, Play, Trash2 } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

import * as api from "@/lib/api";
import type { AccountMeta } from "@/lib/types";

export default function BatchAccountOperationsPage() {
  const [loading, setLoading] = useState(false);
  const [accounts, setAccounts] = useState<AccountMeta[]>([]);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [activeTab, setActiveTab] = useState("cn");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<string | null>(null);

  const load = useCallback(async (region?: string) => {
    setLoading(true);
    try {
      const data = await api.getAccounts(region as any);
      setAccounts(data.accounts ?? []);
    } catch (e) {
      toast.error("Failed to load accounts");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load(activeTab);
  }, [activeTab, load]);

  const toggle = (id: string) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const toggleAll = () => {
    setSelectedIds((prev) => {
      if (prev.size === accounts.length) return new Set();
      return new Set(accounts.map((a) => a.id));
    });
  };

  const handleBatchDelete = async () => {
    if (selectedIds.size === 0) return;
    setBusy(true);
    setResult(null);
    const errors: string[] = [];
    let count = 0;
    for (const id of selectedIds) {
      try {
        await api.deleteAccount(id, activeTab as any);
        count++;
      } catch (e) {
        errors.push(id);
      }
    }
    setResult(`Deleted ${count} accounts${errors.length ? `, failed: ${errors.join(", ")}` : ""}`);
    setSelectedIds(new Set());
    load(activeTab);
    setBusy(false);
  };

  const handleBatchSwitch = async () => {
    if (selectedIds.size === 0) return;
    setBusy(true);
    setResult(null);
    const errors: string[] = [];
    const ids = Array.from(selectedIds);
    for (let i = 0; i < ids.length; i++) {
      try {
        await api.switchAccount({ accountId: ids[i], region: activeTab as any });
      } catch (e) {
        errors.push(ids[i]);
      }
    }
    setResult(`Switched ${ids.length - errors.length} accounts${errors.length ? `, failed: ${errors.join(", ")}` : ""}`);
    setBusy(false);
  };

  const handleBatchExport = async () => {
    if (selectedIds.size === 0) return;
    setBusy(true);
    setResult(null);
    try {
      const res = await api.exportAccounts(Array.from(selectedIds), activeTab as any);
      setResult(`Exported ${res.accounts?.length ?? 0} accounts`);
    } catch (e) {
      setResult("Export failed");
    }
    setBusy(false);
  };

  const selectedCount = selectedIds.size;

  return (
    <div className="h-full space-y-4 overflow-auto p-1">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 className="text-xl font-semibold">Batch Account Operations</h1>
          <p className="text-xs text-muted-foreground">Select accounts to batch delete, switch, or export.</p>
        </div>
        <div className="flex gap-2">
          <Button variant="secondary" size="sm" onClick={() => load(activeTab)} disabled={loading || busy}>
            {loading ? <Loader2 className="mr-2 size-4 animate-spin" /> : null}
            Refresh
          </Button>
          <Button variant="destructive" size="sm" onClick={handleBatchDelete} disabled={busy || selectedCount === 0}>
            <Trash2 className="mr-2 size-4" />
            Delete ({selectedCount})
          </Button>
          <Button size="sm" onClick={handleBatchSwitch} disabled={busy || selectedCount === 0}>
            <Play className="mr-2 size-4" />
            Switch ({selectedCount})
          </Button>
          <Button variant="secondary" size="sm" onClick={handleBatchExport} disabled={busy || selectedCount === 0}>
            <Download className="mr-2 size-4" />
            Export ({selectedCount})
          </Button>
        </div>
      </div>

      <Tabs value={activeTab} onValueChange={setActiveTab}>
        <TabsList>
          <TabsTrigger value="cn">CN</TabsTrigger>
          <TabsTrigger value="global">Global</TabsTrigger>
        </TabsList>
        <TabsContent value="cn" className="mt-2">
          <AccountList
            accounts={accounts}
            selectedIds={selectedIds}
            onToggle={toggle}
            onToggleAll={toggleAll}
            loading={loading}
          />
        </TabsContent>
        <TabsContent value="global" className="mt-2">
          <AccountList
            accounts={accounts}
            selectedIds={selectedIds}
            onToggle={toggle}
            onToggleAll={toggleAll}
            loading={loading}
          />
        </TabsContent>
      </Tabs>

      {result && <p className="text-xs text-muted-foreground">{result}</p>}
    </div>
  );
}

function AccountList({
  accounts,
  selectedIds,
  onToggle,
  onToggleAll,
  loading,
}: {
  accounts: AccountMeta[];
  selectedIds: Set<string>;
  onToggle: (id: string) => void;
  onToggleAll: () => void;
  loading: boolean;
}) {
  if (loading) return <div className="py-10 text-center text-sm text-muted-foreground">Loading...</div>;
  if (accounts.length === 0) return <div className="py-10 text-center text-sm text-muted-foreground">No accounts</div>;

  const allSelected = accounts.length > 0 && selectedIds.size === accounts.length;

  return (
    <Card className="min-w-0">
      <div className="divide-y">
        <label className="flex cursor-pointer items-center gap-3 px-4 py-2">
          <input type="checkbox" checked={allSelected} onChange={onToggleAll} />
          <span className="text-xs text-muted-foreground">Toggle all</span>
        </label>
        {accounts.map((account) => (
          <label key={account.id} className="flex cursor-pointer items-center gap-3 px-4 py-2">
            <input
              type="checkbox"
              checked={selectedIds.has(account.id)}
              onChange={() => onToggle(account.id)}
            />
            <span className="min-w-0 flex-1 truncate text-sm">
              {account.nickname || account.email || account.id}
            </span>
            {account.region && <span className="text-xs text-muted-foreground">{account.region}</span>}
          </label>
        ))}
      </div>
    </Card>
  );
}
