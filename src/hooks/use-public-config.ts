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
