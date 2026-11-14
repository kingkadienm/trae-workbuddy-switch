import { useEffect, useState } from "react";
import { gatewayOpenAiBaseUrlFrom } from "@/lib/gateway-url";
import { isWebui } from "@/lib/api";

export interface PublicConfig {
  workbuddyBase: string;
  traeBase: string;
}

const DEFAULT_CONFIG: PublicConfig = {
  workbuddyBase: "https://api.wangzs.vip/workbuddy",
  traeBase: "https://api.wangzs.vip/trae",
};

let cached: PublicConfig | null = null;

/** 读取后端公开配置（base URL）。失败时回退默认值。 */
export async function fetchPublicConfig(): Promise<PublicConfig> {
  if (cached) return cached;
  try {
    const res = await fetch("/api/public-config", { signal: AbortSignal.timeout(3000) });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const data = (await res.json()) as Partial<PublicConfig>;
    cached = {
      workbuddyBase: data.workbuddyBase?.trim() || DEFAULT_CONFIG.workbuddyBase,
      traeBase: data.traeBase?.trim() || DEFAULT_CONFIG.traeBase,
    };
  } catch {
    cached = { ...DEFAULT_CONFIG };
  }
  return cached;
}

/** 重置缓存（测试用）。 */
export function resetPublicConfigCache() {
  cached = null;
}

/** WebUI 展示部署后的对外地址；桌面端继续按本地监听派生。 */
export function usePublicGatewayBase(platform: "workbuddy" | "trae", localBase: string): string {
  const [publicBase, setPublicBase] = useState<string | null>(null);
  useEffect(() => {
    if (!isWebui()) return;
    let active = true;
    void fetchPublicConfig().then((config) => {
      if (active) setPublicBase(platform === "workbuddy" ? config.workbuddyBase : config.traeBase);
    });
    return () => { active = false; };
  }, [platform]);
  return isWebui() && publicBase ? gatewayOpenAiBaseUrlFrom(publicBase) : localBase;
}
