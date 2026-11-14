/**
 * 「图片输入」列的**装配器**（issue #101）—— 三个平台模型页共用一份。
 *
 * 三页各自写一遍「读覆盖表 / 合并三态 / 写入 / toast / busy 态」会立刻出现三份判据，
 * 而这块恰恰是最容易写歪的一处（`null` 与 `false` 的语义差别只在一次 `??` 之间）。
 * 因此：**取数、判据、提示、忙态都在这里**，页面只传「作用域」与「怎么按 id 找目录条目」。
 *
 * 目录条目的取法三页不同（WB 有完整目录、Trae 的客户端清单没有能力字段、Qoder 走官方清单），
 * 所以用 `entryOf` 注入 —— 与 `model-table.tsx` 的能力列槽同一分工：壳不认平台。
 */

import { useEffect, useState } from "react";
import { toast } from "sonner";

import { ImageCapCell } from "@/components/gateway/model-image-cell";
import * as api from "@/lib/api";
import { useT } from "@/lib/i18n";
import { resolveImageCap, type ImageCapEntry } from "@/lib/model-image";
import type { GatewayModelRow } from "@/lib/types";
import type { Scope } from "@/lib/scope";
import { EMPTY_IMAGE_CAPS, useGatewayStore } from "@/stores/gateway";

/** `model-table.tsx` 的能力列槽形状（只取这里用到的部分）。 */
export interface ImageCapColumn {
  header: string;
  render: (row: GatewayModelRow) => React.ReactNode;
}

export interface UseImageCapColumnArgs {
  scope: Scope;
  /**
   * 按模型 id 取**目录条目**（含 `supports_images` / `image_input_source`）。
   *
   * 该平台清单没有这两个字段时返回 `undefined` ⇒ 三态只剩「覆盖 / 未声明」两种，
   * 这正是 Trae 的现状（客户端清单只有 name 与上下文窗口）。
   */
  entryOf: (modelId: string) => ImageCapEntry | undefined;
}

/**
 * 返回可直接传给 `<ModelTable imageColumn={...} />` 的列定义。
 *
 * 进入作用域时读一次覆盖表（幂等；页面级 `loadAll` 也会拉）。
 */
export function useImageCapColumn({ scope, entryOf }: UseImageCapColumnArgs): ImageCapColumn {
  const t = useT();
  const caps = useGatewayStore((s) => s.imageCaps[scope]) ?? EMPTY_IMAGE_CAPS;
  const loadImageCaps = useGatewayStore((s) => s.loadImageCaps);
  const setModelImageCap = useGatewayStore((s) => s.setModelImageCap);
  /** 正在写的行 id（驱动该行 `<Loader2>` + 防重复点击）。 */
  const [pendingId, setPendingId] = useState<string | null>(null);

  useEffect(() => {
    void loadImageCaps(scope);
  }, [loadImageCaps, scope]);

  async function onSet(row: GatewayModelRow, next: boolean | null) {
    setPendingId(row.id);
    try {
      await setModelImageCap(scope, row.id, next);
      toast.success(
        t(
          next === null
            ? "shared.gateway.models.imageResetToast"
            : next
              ? "shared.gateway.models.imageYesToast"
              : "shared.gateway.models.imageNoToast",
          { name: row.name },
        ),
      );
    } catch (e) {
      // 与「禁用 / 启用」同款：把后端原因透出来，别换成笼统「操作失败」。
      toast.error(t("shared.gateway.models.toggleFail"), { description: api.asError(e) });
    } finally {
      setPendingId(null);
    }
  }

  return {
    header: t("shared.gateway.models.colImages"),
    render: (row) => (
      <ImageCapCell
        cap={resolveImageCap(entryOf(row.id), caps[row.id])}
        busy={pendingId === row.id}
        onSet={(next) => void onSet(row, next)}
      />
    ),
  };
}
