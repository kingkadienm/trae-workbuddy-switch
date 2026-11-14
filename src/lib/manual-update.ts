import { checkUpdate } from "@/lib/api";

/** 显式点击检查更新时绕过缓存；自动轮询始终沿用缓存。 */
export function checkManualUpdate() {
  return checkUpdate(undefined, true);
}
