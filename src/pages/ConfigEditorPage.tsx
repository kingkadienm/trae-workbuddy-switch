import { useEffect, useMemo, useState } from "react";
import { Loader2, RefreshCw, Save } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

import { getGatewayConfig, saveGatewayConfig } from "@/lib/api";
import type { GatewayConfig } from "@/lib/types";

export default function ConfigEditorPage() {
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [raw, setRaw] = useState("");
  const [parseError, setParseError] = useState<string | null>(null);
  const [lastSaved, setLastSaved] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState("editor");

  const load = async () => {
    setLoading(true);
    try {
      const data = await getGatewayConfig();
      setRaw(JSON.stringify(data, null, 2));
      setParseError(null);
    } catch (e) {
      toast.error("Failed to load config");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  const handleSave = async () => {
    if (parseError) {
      toast.error("Fix JSON errors before saving");
      return;
    }
    setSaving(true);
    try {
      await saveGatewayConfig(raw as unknown as GatewayConfig);
      setLastSaved(new Date().toLocaleTimeString());
      toast.success("Config saved");
    } catch (e) {
      toast.error("Save failed");
    } finally {
      setSaving(false);
    }
  };

  const handleFormat = () => {
    try {
      const obj = JSON.parse(raw);
      setRaw(JSON.stringify(obj, null, 2));
      setParseError(null);
    } catch (e) {
      setParseError((e as Error).message);
    }
  };

  const preview = useMemo(() => {
    try {
      const obj = JSON.parse(raw);
      return JSON.stringify(obj, null, 2);
    } catch (e) {
      return null;
    }
  }, [raw]);

  return (
    <div className="h-full space-y-4 overflow-hidden p-1">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <h1 className="text-xl font-semibold">Config Editor</h1>
          {lastSaved && (
            <p className="text-xs text-muted-foreground">
              Last saved: {lastSaved}
            </p>
          )}
        </div>
        <div className="flex gap-2">
          <Button variant="secondary" size="sm" onClick={load} disabled={loading || saving}>
            {loading ? <Loader2 className="mr-2 size-4 animate-spin" /> : <RefreshCw className="mr-2 size-4" />}
            Reload
          </Button>
          <Button variant="secondary" size="sm" onClick={handleFormat}>
            Format
          </Button>
          <Button size="sm" onClick={handleSave} disabled={saving}>
            {saving ? <Loader2 className="mr-2 size-4 animate-spin" /> : <Save className="mr-2 size-4" />}
            Save
          </Button>
        </div>
      </div>

      <Tabs value={activeTab} onValueChange={setActiveTab} className="h-[calc(100vh-220px)]">
        <TabsList>
          <TabsTrigger value="editor">Editor</TabsTrigger>
          <TabsTrigger value="preview">Preview</TabsTrigger>
        </TabsList>
        <TabsContent value="editor" className="mt-2 h-full">
          <Card className="h-full">
            <textarea
              value={raw}
              onChange={(e) => {
                setRaw(e.target.value);
                try {
                  JSON.parse(e.target.value);
                  setParseError(null);
                } catch (e) {
                  setParseError((e as Error).message);
                }
              }}
              className="h-full w-full resize-none rounded-none border-0 bg-transparent p-3 font-mono text-xs"
            />
          </Card>
          {parseError && <p className="mt-1 text-xs text-red-500">{parseError}</p>}
        </TabsContent>
        <TabsContent value="preview" className="mt-2 h-full">
          <Card className="h-full overflow-auto">
            <pre className="h-full whitespace-pre-wrap p-3 font-mono text-xs">
              {preview ?? "Invalid JSON"}
            </pre>
          </Card>
        </TabsContent>
      </Tabs>
    </div>
  );
}
