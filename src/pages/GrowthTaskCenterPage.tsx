import { useMemo, useState } from "react";
import { Loader2, RefreshCcw, Play } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import * as api from "@/lib/api";
import { useT } from "@/lib/i18n";

type GrowthTask = {
  task_code: string;
  title: string;
  desc?: string;
  status?: string;
  reward?: string;
  claimed?: boolean;
};

type GrowthRunItem = {
  account: string;
  region: string;
  task_code: string;
  status: string;
  detail?: string;
};

export default function GrowthTaskCenterPage() {
  const t = useT();
  const [tasks, setTasks] = useState<GrowthTask[]>([]);
  const [loading, setLoading] = useState(false);
  const [running, setRunning] = useState(false);
  const [logs, setLogs] = useState<GrowthRunItem[]>([]);
  const [runText, setRunText] = useState("");

  const loadTasks = async () => {
    setLoading(true);
    try {
      const res = await api.growthTasks();
      const list = Array.isArray((res as any)?.tasks) ? ((res as any).tasks as GrowthTask[]) : [];
      setTasks(list);
    } catch (e) {
      console.error(e);
    } finally {
      setLoading(false);
    }
  };

  const acceptTask = async (code: string) => {
    await api.growthAccept({ code });
    await loadTasks();
  };

  const claimTask = async (code: string) => {
    await api.growthClaim(code);
    await loadTasks();
  };

  const runCycle = async () => {
    setRunning(true);
    setRunText("");
    try {
      const res = await api.growthRun();
      const items = Array.isArray((res as any)?.accounts) ? ((res as any).accounts as GrowthRunItem[]) : [];
      setLogs(items);
      setRunText(
        items
          .map((item) => `${item.region}::${item.account} | ${item.task_code} -> ${item.status} ${item.detail ?? ""}`)
          .join("\n") || JSON.stringify(res, null, 2),
      );
    } catch (e) {
      setRunText(String(e));
    } finally {
      setRunning(false);
    }
  };

  const pendingCount = useMemo(() => tasks.filter((task) => !task.claimed && task.status !== "completed").length, [tasks]);
  const completedCount = useMemo(() => tasks.filter((task) => task.claimed || task.status === "completed").length, [tasks]);
  const progress = tasks.length ? Math.round((completedCount / tasks.length) * 100) : 0;

  return (
    <div className="mx-auto max-w-5xl space-y-4 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 className="text-lg font-semibold">{t("growth.title")}</h1>
          <p className="text-xs text-muted-foreground">
            {t("growth.progress", { fallback: "Completed" })} {completedCount}/{tasks.length} ({progress}%)
          </p>
        </div>
        <div className="flex gap-2">
          <Button variant="secondary" onClick={loadTasks} disabled={loading}>
            {loading ? <Loader2 className="mr-2 size-4 animate-spin" /> : <RefreshCcw className="mr-2 size-4" />}
            {loading ? t("growth.refresh", { fallback: "Loading" }) : t("growth.refresh", { fallback: "Refresh" })}
          </Button>
          <Button onClick={runCycle} disabled={running}>
            <Play className="mr-2 size-4" />
            {running ? t("growth.running", { fallback: "Running" }) : t("growth.runAll", { fallback: "Run all" })}
          </Button>
        </div>
      </div>

      <Card className="space-y-3 p-4">
        <div className="flex items-center gap-2">
          <span className="text-xs text-muted-foreground">{t("growth.progressBar", { fallback: "Progress" })}</span>
          <div className="h-2 flex-1 overflow-hidden rounded-full bg-sidebar-border">
            <div className="h-full rounded-full bg-primary transition-all" style={{ width: `${progress}%` }} />
          </div>
        </div>
        <Tabs defaultValue="pending">
          <TabsList>
            <TabsTrigger value="pending">
              {t("growth.pending", { fallback: "Pending" })} ({pendingCount})
            </TabsTrigger>
            <TabsTrigger value="completed">
              {t("growth.completed", { fallback: "Completed" })} ({completedCount})
            </TabsTrigger>
            <TabsTrigger value="logs">
              {t("growth.logs", { fallback: "Run logs" })} ({logs.length})
            </TabsTrigger>
          </TabsList>
          <TabsContent value="pending">
            <div className="max-h-64 space-y-2 overflow-y-auto">
              {tasks
                .filter((task) => !task.claimed && task.status !== "completed")
                .map((task) => (
                  <Card key={task.task_code} className="flex flex-wrap items-center justify-between gap-2 p-3">
                    <div>
                      <div className="font-medium">{task.title || task.task_code}</div>
                      {task.desc ? <div className="text-xs text-muted-foreground">{task.desc}</div> : null}
                      {task.reward ? (
                        <div className="text-xs text-muted-foreground">
                          {t("growth.reward", { fallback: "Reward" })}: {task.reward}
                        </div>
                      ) : null}
                    </div>
                    <Button size="sm" onClick={() => acceptTask(task.task_code)}>
                      {t("growth.accept", { fallback: "Accept" })}
                    </Button>
                  </Card>
                ))}
              {pendingCount === 0 ? (
                <div className="p-3 text-xs text-muted-foreground">{t("growth.emptyPending", { fallback: "No pending tasks" })}</div>
              ) : null}
            </div>
          </TabsContent>
          <TabsContent value="completed">
            <div className="max-h-64 space-y-2 overflow-y-auto">
              {tasks
                .filter((task) => task.claimed || task.status === "completed")
                .map((task) => (
                  <Card key={task.task_code} className="flex flex-wrap items-center justify-between gap-2 p-3">
                    <div>
                      <div className="font-medium">{task.title || task.task_code}</div>
                      <div className="text-xs text-muted-foreground">{task.status ?? "completed"}</div>
                    </div>
                    <Button size="sm" variant="secondary" onClick={() => claimTask(task.task_code)}>
                      {t("growth.claim", { fallback: "Claim" })}
                    </Button>
                  </Card>
                ))}
              {completedCount === 0 ? (
                <div className="p-3 text-xs text-muted-foreground">{t("growth.emptyCompleted", { fallback: "No completed tasks" })}</div>
              ) : null}
            </div>
          </TabsContent>
          <TabsContent value="logs">
            <div className="max-h-64 overflow-y-auto whitespace-pre-wrap font-mono text-xs">{runText}</div>
          </TabsContent>
        </Tabs>
      </Card>
    </div>
  );
}
