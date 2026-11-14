import { useCallback, useEffect, useRef, useState } from "react";

/**
 * 「刷新类动作」的共享反馈层 —— **点击后立刻进入进行中态，动作完成后才退出**。
 *
 * ## 为什么需要它（本次修复的根因）
 *
 * 审查发现大量刷新按钮是 `void doSomething()` 的 fire-and-forget：点击后按钮
 * 没有任何变化，请求在后台跑完也没有提示 —— 用户读到的就是「点了没反应」。
 * 本 hook 把「进入 / 退出进行中态」的时机管理收成一份实现，按钮只需绑 `busy`。
 *
 * ## ★ 最小展示时长（`minSpinMs`，默认 500ms）
 *
 * 本地调用（Tauri IPC / 进程探测）常在 50–200ms 内返回 —— 若「进行中态」跟真
 * 实耗时走，spinner 会**一闪而过甚至一帧都不渲染**，感官上仍等于「没反馈」。
 * 因此动作快于 `minSpinMs` 时**补足**展示时长；动作本身更慢时**不额外等待**。
 * 500ms 对齐原型的刷新动效（`doRefresh` 的 loading 固定 500ms）。
 *
 * ⚠️ 展示时长只影响**视觉**：`action` 的完成时刻、错误传播都不被推迟
 * （错误在 `action` 内自行处理，或在调用方 `catch`）。
 *
 * ## 用法
 *
 * ```tsx
 * const { busy, run } = useRefreshAction();
 * <Button disabled={busy} onClick={() => void run(() => store.reload())}>
 *   {busy ? <Loader2 className="animate-spin" /> : <RefreshCw />}
 * </Button>
 * ```
 *
 * ⚠️ 调用方应把 `busy` 绑到 `disabled` 上防重复点击（本 hook 不做点击节流：
 * 它不知道「重复点击」该排队、该忽略还是该取消防）。
 */
export function useRefreshAction(minSpinMs = 500): {
  /** 动作进行中（含最小展示时长的补足段）。 */
  busy: boolean;
  /** 执行动作并管理 `busy` 的进出时机。返回的 Promise 在**展示结束**后 resolve。 */
  run: (action: () => unknown | Promise<unknown>) => Promise<void>;
} {
  const [busy, setBusy] = useState(false);

  /**
   * 存活标志：组件卸载后不再 `setBusy`（React 18 里 setState-after-unmount 是
   * 无害 no-op，但吸底的 setTimeout 尾段还可能跑到，别让它去碰卸载组件）。
   *
   * ★ **setup 里必须把它置回 `true`**：`main.tsx` 开着 StrictMode，开发模式会做
   * 「挂载 → 卸载 → 再挂载」演练 —— 演练的卸载会执行一次 cleanup 把标志打成
   * `false`；若 setup 不重置，标志**永远为 false** ⇒ `setBusy(false)` 被静默
   * 跳过 ⇒ 按钮点了以后**永远转**（CDP 实测踩到：点击 8 秒后仍 spinning）。
   */
  const aliveRef = useRef(true);
  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
    };
  }, []);

  const run = useCallback(
    async (action: () => unknown | Promise<unknown>) => {
      setBusy(true);
      const startedAt = Date.now();
      try {
        await action();
      } finally {
        const remaining = minSpinMs - (Date.now() - startedAt);
        if (remaining > 0) await new Promise((resolve) => setTimeout(resolve, remaining));
        if (aliveRef.current) setBusy(false);
      }
    },
    [minSpinMs],
  );

  return { busy, run };
}
