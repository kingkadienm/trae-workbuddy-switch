import { useState } from "react";
import { Loader2, RefreshCw } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";

import { activityRun } from "@/lib/api";

export default function ActivityPage() {
  const [running, setRunning] = useState(false);
  const [lastResult, setLastResult] = useState<unknown | null>(null);

  const handleRun = async () => {
    setRunning(true);
    try {
      const result = await activityRun();
      setLastResult(result);
      toast.success("Activity cycle finished");
    } catch (e) {
      toast.error("Activity cycle failed");
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="h-full space-y-4 overflow-auto p-1">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 className="text-xl font-semibold">Activity</h1>
          <p className="text-xs text-muted-foreground">Run streak/lottery cycle for all accounts.</p>
        </div>
        <Button size="sm" onClick={handleRun} disabled={running}>
          {running ? <Loader2 className="mr-2 size-4 animate-spin" /> : <RefreshCw className="mr-2 size-4" />}
          Run activity
        </Button>
      </div>

      <Card className="p-3">
        <pre className="whitespace-pre-wrap font-mono text-xs">
          {lastResult ? JSON.stringify(lastResult, null, 2) : "No result yet"}
        </pre>
      </Card>
    </div>
  );
}
