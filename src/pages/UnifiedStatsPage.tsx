import { useCallback, useEffect, useMemo, useState } from "react";
import { Loader2, RefreshCw } from "lucide-react";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { CircleAlert } from "lucide-react";

import * as api from "@/lib/api";
import type { CreditStatistics } from "@/lib/types";
import type { TraeTokenStatistics } from "@/lib/trae-types";

export default function UnifiedStatsPage() {
  const [tab, setTab] = useState<"workbuddy" | "trae">("workbuddy");
  const [wbLoading, setWbLoading] = useState(false);
  const [wbError, setWbError] = useState<string | null>(null);
  const [wbStats, setWbStats] = useState<CreditStatistics | null>(null);
  const [traeLoading, setTraeLoading] = useState(false);
  const [traeError, setTraeError] = useState<string | null>(null);
  const [traeStats, setTraeStats] = useState<TraeTokenStatistics | null>(null);

  const loadWorkBuddy = useCallback(async (refresh = false) => {
    setWbLoading(true);
    setWbError(null);
    try {
      const data = await api.getCreditStatistics(refresh, "all");
      setWbStats(data);
    } catch (e) {
      setWbError(api.asError(e));
    } finally {
      setWbLoading(false);
    }
  }, []);

  const loadTrae = useCallback(async () => {
    setTraeLoading(true);
    setTraeError(null);
    try {
      const data = await api.getTraeTokenStatistics(30, "all");
      setTraeStats(data);
    } catch (e) {
      setTraeError(api.asError(e));
    } finally {
      setTraeLoading(false);
    }
  }, []);

  useEffect(() => {
    loadWorkBuddy();
  }, [loadWorkBuddy]);

  useEffect(() => {
    loadTrae();
  }, [loadTrae]);

  // Refresh both when user explicitly asks for it.
  const handleRefresh = useCallback(() => {
    loadWorkBuddy(true);
    loadTrae();
  }, [loadWorkBuddy, loadTrae]);

  const wbSummary = useMemo(() => {
    if (!wbStats) return null;
    return {
      remaining: wbStats.summary.currentRemaining,
      today: wbStats.summary.usageToday,
      sevenDays: wbStats.summary.usage7Days,
      month: wbStats.summary.usageThisMonth,
    };
  }, [wbStats]);

  const traeSummary = useMemo(() => {
    if (!traeStats) return null;
    const s = traeStats.summary;
    return {
      total: s.total,
      input: s.input,
      output: s.output,
      records: s.records,
    };
  }, [traeStats]);

  return (
    <div className="h-full space-y-4 overflow-auto p-1">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 className="text-xl font-semibold">Unified Stats</h1>
          <p className="text-xs text-muted-foreground">
            Combined WorkBuddy and Trae usage overview
          </p>
        </div>
        <Button variant="secondary" size="sm" onClick={handleRefresh} disabled={wbLoading || traeLoading}>
          {(wbLoading || traeLoading) ? <Loader2 className="mr-2 size-4 animate-spin" /> : <RefreshCw className="mr-2 size-4" />}
          Refresh
        </Button>
      </div>

      <Tabs value={tab} onValueChange={(value) => setTab(value as "workbuddy" | "trae")} className="h-[calc(100vh-220px)]">
        <TabsList>
          <TabsTrigger value="workbuddy">WorkBuddy</TabsTrigger>
          <TabsTrigger value="trae">Trae</TabsTrigger>
        </TabsList>
        <TabsContent value="workbuddy" className="mt-2 h-full">
          <div className="h-full space-y-3 overflow-auto">
            {wbError && (
              <Alert variant="destructive">
                <CircleAlert className="size-4" />
                <AlertTitle>Load failed</AlertTitle>
                <AlertDescription>
                  <span>{wbError}</span>
                </AlertDescription>
              </Alert>
            )}
            {!wbError && wbSummary && (
              <Card className="grid grid-cols-2 gap-4 p-4 sm:grid-cols-4">
                <Stat label="Remaining" value={String(wbSummary.remaining)} />
                <Stat label="Today" value={String(wbSummary.today)} />
                <Stat label="7 days" value={String(wbSummary.sevenDays)} />
                <Stat label="Month" value={String(wbSummary.month)} />
              </Card>
            )}
            {!wbError && !wbStats && wbLoading && (
              <div className="flex items-center gap-2 py-10 text-sm text-muted-foreground">
                <Loader2 className="animate-spin" />
                Loading...
              </div>
            )}
          </div>
        </TabsContent>
        <TabsContent value="trae" className="mt-2 h-full">
          <div className="h-full space-y-3 overflow-auto">
            {traeError && (
              <Alert variant="destructive">
                <CircleAlert className="size-4" />
                <AlertTitle>Load failed</AlertTitle>
                <AlertDescription>
                  <span>{traeError}</span>
                </AlertDescription>
              </Alert>
            )}
            {!traeError && traeSummary && (
              <Card className="grid grid-cols-2 gap-4 p-4 sm:grid-cols-4">
                <Stat label="Total" value={String(traeSummary.total)} />
                <Stat label="Input" value={String(traeSummary.input)} />
                <Stat label="Output" value={String(traeSummary.output)} />
                <Stat label="Records" value={String(traeSummary.records)} />
              </Card>
            )}
            {!traeError && !traeStats && traeLoading && (
              <div className="flex items-center gap-2 py-10 text-sm text-muted-foreground">
                <Loader2 className="animate-spin" />
                Loading...
              </div>
            )}
          </div>
        </TabsContent>
      </Tabs>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col items-center gap-1 text-center">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className="text-lg font-semibold tabular-nums">{value}</span>
    </div>
  );
}
