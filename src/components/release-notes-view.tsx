import { useState } from "react";
import { ChevronDown, ChevronUp, Clock, TriangleAlert } from "lucide-react";

import { cn } from "@/lib/utils";
import { formatNotesDate, notesStats, olderNotes } from "@/lib/release-notes";
import { useLocale, useT } from "@/lib/i18n";
import type { ReleaseNoteSection, ReleaseNotes } from "@/lib/types";

/**
 * 版本说明（升级前弹窗的正文）。
 *
 * ## 只负责渲染，不负责取数
 *
 * 数据来自 `UpdateInfo.notes`（后端洗净后的 `release-notes.json`），本组件**不 fetch、
 * 不解析 markdown**。因此这里没有任何注入面：所有文案都经 JSX 文本插值（React 默认转义）。
 *
 * ## 三段结构，顺序是有判据的
 *
 * ① **升级前必读**（`breaking`，警示色 + `role="alert"`）——端口 / 数据格式这类漏看一次
 *    就是「升级完直接用不了」，混在长列表里必被划走 ⇒ 永远排在最前。
 * ② 本版更新（`sections`）：按主题分组的「一句话说清结果」，不是提交记录。
 * ③ 更早版本（`older`）：只在落后不止一版时才有意义，**默认折叠**，否则会把本版说明顶走。
 *
 * 拿不到说明时**如实说**（`sections` 为空），不留一块空标题假装「本版没有更新」。
 */

/** 一组「主题 + 条目」的渲染单元：本版与历史版本共用（历史只去掉必读置顶）。 */
function SectionGroup({ section }: { section: ReleaseNoteSection }) {
  return (
    /* 分组用一条左侧细线，而不是卡片堆叠：十余条内容时卡片会变成一堵墙。 */
    <div className="border-l-2 border-border-strong pl-3">
      <b className="text-[12.5px] font-semibold">{section.title}</b>
      <ul className="mt-1 list-outside list-disc pl-4 text-[12.5px] leading-relaxed text-muted-foreground">
        {section.items.map((item) => (
          <li key={item} className="mb-1 marker:text-border-strong">
            {item}
          </li>
        ))}
      </ul>
    </div>
  );
}

interface ReleaseNotesViewProps {
  notes: ReleaseNotes;
  /** 当前安装版本：决定「更早的 N 个版本」里到底铺哪些（落后 1 版与落后 8 版看到的不同）。 */
  current?: string;
}

export function ReleaseNotesView({ notes, current }: ReleaseNotesViewProps) {
  const t = useT();
  const locale = useLocale();
  const [olderOpen, setOlderOpen] = useState(false);

  const breaking = notes.breaking ?? [];
  const sections = notes.sections ?? [];
  const older = olderNotes(notes, current);
  const stats = notesStats(notes);

  return (
    <div className="flex flex-col gap-4">
      {breaking.length > 0 && (
        <div
          role="alert"
          data-release-notes="breaking"
          className="rounded-xl border border-warning/35 bg-warning/[0.09] p-3"
        >
          <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1 text-xs text-warning">
            <TriangleAlert className="size-3.5 shrink-0 self-center" aria-hidden="true" />
            <b className="font-semibold">{t("wbSettings.update.breakingTitle")}</b>
            <span className="text-[10.5px] font-normal text-muted-foreground">
              {t("wbSettings.update.breakingCount", { count: breaking.length })}
            </span>
          </div>
          <ul className="mt-2.5 flex list-none flex-col gap-2.5 pl-0">
            {breaking.map((item) => (
              <li key={item.title}>
                <b className="block text-[12.5px] font-semibold text-foreground">{item.title}</b>
                {item.desc && (
                  <span className="mt-0.5 block text-xs leading-relaxed text-muted-foreground">
                    {item.desc}
                  </span>
                )}
              </li>
            ))}
          </ul>
        </div>
      )}

      {sections.length > 0 ? (
        <section data-release-notes="sections" className="flex flex-col gap-2.5">
          <h4 className="flex items-baseline gap-2 text-[11.5px] font-medium text-muted-foreground">
            {t("wbSettings.update.sectionsTitle")}
            <span className="text-[10.5px] font-normal tabular-nums">
              {t("wbSettings.update.sectionsMeta", { groups: stats.groups, items: stats.items })}
            </span>
          </h4>
          {sections.map((section) => (
            <SectionGroup key={section.title} section={section} />
          ))}
        </section>
      ) : (
        <p data-release-notes="empty" className="text-xs text-muted-foreground">
          {t("wbSettings.update.notesEmpty")}
        </p>
      )}

      {older.length > 0 && (
        <div data-release-notes="older">
          <button
            type="button"
            data-act="toggle-older-notes"
            aria-expanded={olderOpen}
            onClick={() => setOlderOpen((value) => !value)}
            className={cn(
              "flex w-full items-center gap-2 rounded-[9px] border border-dashed border-border px-2.5 py-2",
              "text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
            )}
          >
            <Clock className="size-3.5 shrink-0" aria-hidden="true" />
            <span>
              {olderOpen
                ? t("wbSettings.update.olderHide")
                : t("wbSettings.update.olderShow", { count: older.length })}
            </span>
            {olderOpen ? (
              <ChevronUp className="ml-auto size-3 shrink-0" aria-hidden="true" />
            ) : (
              <ChevronDown className="ml-auto size-3 shrink-0" aria-hidden="true" />
            )}
          </button>
          {olderOpen && (
            <div className="mt-2.5 flex flex-col gap-2.5">
              {older.map((entry) => (
                <div key={entry.version} className="flex flex-col gap-1.5">
                  <div className="flex items-baseline gap-2">
                    <b className="text-[12.5px] font-semibold tabular-nums">v{entry.version}</b>
                    <span className="text-[10.5px] tabular-nums text-muted-foreground">
                      {formatNotesDate(entry.publishedAt, locale) ?? ""}
                    </span>
                  </div>
                  {entry.sections.map((section) => (
                    <SectionGroup key={`${entry.version}-${section.title}`} section={section} />
                  ))}
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
