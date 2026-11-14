import { useState } from "react";
import { Check, Copy } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { copyText } from "@/lib/clipboard";
import {
  GATEWAY_PROTOCOLS,
  accessTools,
  gatewayCurl,
  gatewaySnippet,
  protocolOf,
  type GatewayProtocolKey,
} from "@/lib/gateway-snippets";
import { useT } from "@/lib/i18n";
import { cn } from "@/lib/utils";

/**
 * 接入说明（P11 §1.7）—— **协议 Tab × 工具 Tab** 两段式，与 WorkBuddy 侧同构。
 *
 * ## 由谁使用（2026-10-15：Qoder 接入后**两处分区共用**）
 *
 * 本组件最初为 Trae 写，但它的**全部差异都是数据**（base URL / 默认模型 / Key 前缀 /
 * 两处平台化文案），因此 Qoder 分区直接复用同一份 —— 另写一份必然让「协议 Tab ×
 * 认证头」这类契约各自漂移。平台化的两处文案走 **[`noKeyLabel`]** 与 **[`streamNote`]**
 * props（缺省 = Trae 侧文案），不在这里写 `platform === …`。
 *
 * ## 为什么升级为两段式（与 WB 侧同一理由）
 *
 * 协议是**端点属性**（不是模型属性）：Anthropic 用 `x-api-key`、OpenAI 用
 * `Authorization: Bearer`。把协议做成第一段 Tab，认证头就不可能给错。
 *
 * ## 与 WorkBuddy 版的差别：**单条 Base URL**，没有「按版本」双条目
 *
 * WorkBuddy 版顶部有一排 region Tabs（国内版 / 国际版各自一个 Base URL）；
 * Trae / Qoder 的页面本身已由**作用域**（URL）承载区域 ⇒ 一个分区一条 Base URL，
 * 把 `REGIONS.map` 搬过来会造出一个「切了也没差别」的假维度。
 *
 * `keyPrefix` 由页面从网关状态（`apiKeyPrefix`，最近一把未吊销 Key 的前缀）传入；
 * 为空时给出「请先在上方创建 Key」的占位，而不是伪造一段看起来可用的 Key。
 *
 * `model` 由页面传入（该分区配置里的默认模型）—— 不在这里硬编码模型名（那正是 P3 修掉的缺陷）。
 */

export function TraeIntegrationGuide({
  baseUrl,
  model,
  keyPrefix,
  className,
  /** 无 Key 时占位文案里的前缀示例（缺省 = Trae 的 `sk-trae-…`）。 */
  noKeyLabel,
  /** 「上游只支持流式」那条说明（缺省 = Trae 侧文案）。 */
  streamNote,
}: {
  baseUrl: string;
  model: string;
  keyPrefix?: string;
  className?: string;
  noKeyLabel?: string;
  streamNote?: string;
}) {
  const t = useT();
  const [protocol, setProtocol] = useState<GatewayProtocolKey>("openai");
  const [tool, setTool] = useState("codex");
  const [copied, setCopied] = useState(false);

  const tools = accessTools(t, protocol);
  /* 切协议后原工具可能不在新协议下（如 `codex` 不在 Anthropic 侧）⇒ 回落该协议的第一个工具。 */
  const activeTool = tools.some((item) => item.key === tool) ? tool : tools[0].key;

  // Anthropic 系客户端（Claude Code）自己会拼 `/v1/messages`，因此要给它**根地址**。
  // 与 WorkBuddy 侧的 `integration-guide.tsx` 同算法。
  const rootUrl = baseUrl.replace(/\/v1\/?$/, "");
  const key = keyPrefix ? `${keyPrefix}…` : (noKeyLabel ?? t("trae.gateway.guide.noKey"));
  const snippet = gatewaySnippet({ t, protocol, tool: activeTool, baseUrl, rootUrl, key, model });
  /* 「原始请求（调试用）」：认证头随协议 Tab 变化（Anthropic → x-api-key；OpenAI → Bearer）。
     与 WB 侧 `integration-guide.tsx` 同一算法、同一来源函数。 */
  const curl = gatewayCurl({ protocol, rootUrl, model });

  async function onCopy() {
    await copyText(snippet, t("trae.gateway.guide.copied"));
    setCopied(true);
    window.setTimeout(() => setCopied(false), 1500);
  }

  return (
    <Card className={cn("gap-0 py-0", className)}>
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/60 px-5 py-3">
        <span className="text-sm font-semibold">{t("trae.gateway.guide.title")}</span>
        <Button variant="outline" size="sm" onClick={() => void onCopy()}>
          {copied ? <Check /> : <Copy />}
          {t("trae.gateway.guide.copy")}
        </Button>
      </div>

      {/* 第一段：协议 —— 决定端点路径与认证头 */}
      <Tabs value={protocol} onValueChange={(value) => setProtocol(value as GatewayProtocolKey)}>
        <div className="border-b border-border/60 px-5 py-2">
          <TabsList>
            {GATEWAY_PROTOCOLS.map((item) => (
              <TabsTrigger key={item.key} value={item.key} data-act={`gw-api|${item.key}`}>
                {t(item.labelKey)}
              </TabsTrigger>
            ))}
          </TabsList>
        </div>
      </Tabs>

      {/* 第二段：工具 —— 该协议下的已知客户端 */}
      <Tabs value={activeTool} onValueChange={setTool}>
        <div className="px-5 py-2">
          <TabsList className="flex-wrap">
            {tools.map((item) => (
              <TabsTrigger key={item.key} value={item.key} data-act={`gw-tool|${item.key}`}>
                {item.label}
              </TabsTrigger>
            ))}
          </TabsList>
        </div>
        <div className="px-5 pb-4">
          <pre className="min-w-0 overflow-x-auto rounded-lg border border-border bg-muted/40 p-4 font-mono text-xs leading-6">
            {snippet}
          </pre>
          {/* 端点说明：两种协议**都已支持**（§3.18.2，含流式）。 */}
          <p className="mt-2 text-xs text-muted-foreground">
            {t("shared.gateway.access.endpointNote", {
              openai: protocolOf("openai").endpoint,
              anthropic: protocolOf("anthropic").endpoint,
            })}
          </p>
          {/* 认证头：由**协议**决定（Anthropic → x-api-key；OpenAI → Bearer）。 */}
          <p className="mt-1 text-xs text-muted-foreground">
            {t("shared.gateway.access.authNote", { header: protocolOf(protocol).authHeader })}
          </p>
          <p className="mt-1 text-xs text-muted-foreground">
            {streamNote ?? t("trae.gateway.guide.streamNote")}
          </p>
          {/* 自动发现说明（P10 §1.7）：与本仓 WB 侧 `integration-guide.tsx` 同构 ——
              外部客户端会从本网关的 `/v1/models` 自动读取清单，不需要手填模型名。 */}
          <p className="mt-1 text-xs text-muted-foreground">
            {t("trae.gateway.guide.autoDiscover")}
          </p>
          {/* ★「原始请求（调试用）」（2026-10-13 跨平台对齐）：WB 侧一直有这一块 ——
              方法 + URL + 协议对应的认证头，用户可直接粘贴核验网关是否通。 */}
          <p className="mt-3 text-xs text-muted-foreground">{t("trae.gateway.guide.rawRequest")}</p>
          <pre className="mt-1.5 min-w-0 overflow-x-auto rounded-[10px] bg-card-2 p-3 font-mono text-xs leading-6">
            {curl}
          </pre>
        </div>
      </Tabs>
    </Card>
  );
}
