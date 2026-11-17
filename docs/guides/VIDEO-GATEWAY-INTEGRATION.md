# WorkBuddy 视频网关接入

> 状态：现行（2026-11-17，版本 `v2026.11.171425`）。适用范围：WorkBuddy 独立网关的视频 P1，以及用户本地增强版火宝的 `custom` 视频服务。

视频网关提供模型目录、提交任务和查询任务三条接口。**提交会真实生成视频，可能消耗 WorkBuddy 积分；火宝 custom 的「测试连接」也会真实提交生成，可能扣费。保存配置不等于连通性验证。** 本文给出配置与命令，本轮文档核对没有调用真实媒体或付费接口。

任务只保存在网关进程内存中，重启后丢失。POST 超时或连接断开时，上游可能已经受理；当前没有幂等和重启恢复能力，**不要盲重发 POST，也不要把 POST 放进自动重试或轮询脚本**。

## 1. 准备地址、Key 和账号

在应用中启动 **WorkBuddy 独立网关**，查看它的实际监听地址。默认端口是 `57891`，已有本地配置也可能使用 `7863`。下文统一通过 `VIDEO_BASE` 指定根地址，**不带 `/v1`**；不要直接套用统一网关或其它产品线的端口。

在对应 region 创建无配额限制的 `sk-wb` Key：国内版选 CN，国际版选 Global。这里的「无配额限制」指网关 Key 的 token 配额模式，不代表上游免费或积分无限。P1 会拒绝配额受限 Key。

账号必须有有效凭据、未停用，凭据 `domain` 对应的 region 要与 Key 一致，并处于 Key 允许访问的账号或分组范围内。模型目录也需要可用账号。客户端请求体里的 `region` 不能切换归属域。

**同一任务从提交到查询必须保持同一个 Key。** 另建一个同 region 的 Key 也不能接管旧任务。非终态查询固定使用提交时的原账号；账号被移除、凭据失效或移出 Key 范围后，不会换号接管该任务。

以下 CLI 示例需要 Bash、`curl` 和 `jq`，逐段执行并查看结果。Key 通过隐藏输入读入，不写进示例、命令历史或文件：

```bash
export VIDEO_BASE="${VIDEO_BASE:-http://127.0.0.1:57891}"
# 如果应用实际监听 7863，先执行：
# export VIDEO_BASE="http://127.0.0.1:7863"
VIDEO_BASE="${VIDEO_BASE%/}"
printf '输入对应 region 的无配额限制 WorkBuddy Key：'
IFS= read -r -s VIDEO_KEY
printf '\n'
export VIDEO_KEY
```

若火宝后端位于容器或另一台机器，`127.0.0.1` 指向火宝后端自身；Base URL 必须换成该后端能访问的网关地址。

## 2. 查目录，再提交一次任务

| 方法 | 路径 | 用途 |
|:--|:--|:--|
| GET | `/v1/video/models` | 获取 Key 所属 region 的动态视频模型及能力 |
| POST | `/v1/videos` | 提交一次生成，返回本地任务 `id` |
| GET | `/v1/videos/:id` | 用本地任务 `id` 查询进度和原始上游结果 |

目录来自 WorkBuddy 上游 `/v3/config` 的 `videoGeneration` 能力声明，每次目录请求及提交前都会读取。它不是固定供应商清单；以当前 Key 的返回为准：

```bash
curl --silent --show-error --fail-with-body \
  "${VIDEO_BASE}/v1/video/models" \
  -H "Authorization: Bearer ${VIDEO_KEY:?请先输入 Key}" \
  | jq .
```

返回为 `{"object":"list","data":[...]}`。先确认模型 `id`、`min_seconds`、`max_seconds`、`seconds`、`resolutions`、`aspect_ratios`、`audio_mode` 及默认值。`seconds` 数组为空表示按最小/最大范围校验，并不表示不能生成。目录可见不等于生成成功。

以下是 **CN `minimax-video-h3` 的文生视频**示例，仅在当前目录仍支持这些参数且准备消耗积分时执行一次：

```bash
VIDEO_TASK_ID=''
if VIDEO_SUBMIT="$(curl --silent --show-error --fail-with-body --max-time 120 \
  "${VIDEO_BASE}/v1/videos" \
  -H "Authorization: Bearer ${VIDEO_KEY:?请先输入 Key}" \
  -H 'Content-Type: application/json' \
  --data-binary '{
    "model": "minimax-video-h3",
    "prompt": "清晨的海边，镜头缓慢向前，海浪轻拍沙滩。",
    "seconds": 5,
    "resolution": "768P",
    "aspect_ratio": "16:9",
    "enable_audio": true
  }')"; then
  VIDEO_TASK_ID="$(printf '%s' "$VIDEO_SUBMIT" | jq -er '.id | select(type == "string" and length > 0)')"
  printf '%s' "$VIDEO_SUBMIT" | jq '{id, region, model, status, error}'
else
  printf '%s\n' '提交未取得成功响应；不要自动重发，请先核对上游是否已受理。' >&2
  printf '%s\n' "$VIDEO_SUBMIT" >&2
fi
```

`seconds` 必须是 JSON 正整数，不能写成 `"5"`。`resolution`、`aspect_ratio`、`enable_audio` 可放在顶层或 `extra_parameters` 中；两处同时填写且值冲突会被拒绝。网关按动态目录检查时长、分辨率、比例、提示词长度、音频模式及首帧能力，再提交上游。可选参数应省略而非传 `null`。

提交响应包含 `id`、`region`、`model`、`status`、`error`、`upstream_task_id`、`upstream`、`created_at_ms`、`updated_at_ms`。客户端保存顶层本地 `id` 用于查询，不要把 `upstream_task_id` 填入本地查询路径。

## 3. 查询状态，完成后下载

下面每执行一次只查询一次，不会提交新任务。等待一段时间后可手动再次执行；不要高频查询。

```bash
if VIDEO_RESULT="$(curl --silent --show-error --fail-with-body --max-time 60 \
  "${VIDEO_BASE}/v1/videos/${VIDEO_TASK_ID:?请使用成功提交返回的本地任务 id}" \
  -H "Authorization: Bearer ${VIDEO_KEY:?必须使用提交时的同一个 Key}")"; then
  printf '%s' "$VIDEO_RESULT" | jq '{id, status, error}'
else
  VIDEO_RESULT=''
  printf '%s\n' '查询失败；保留任务 id，核对网络、Key 和原账号后再查，不要重新提交生成。' >&2
fi
```

| 顶层 `status` | 含义 | 下一步 |
|:--|:--|:--|
| `pending` | 已受理，等待生成 | 稍后查询同一任务 |
| `generating` | 正在生成 | 稍后查询同一任务 |
| `completed` | 上游明确完成 | 检查结果 URL 非空后下载 |
| `failed` | 上游明确失败 | 查看顶层字符串 `error`，停止查询该任务 |

只有 `completed` 且 URL 为非空字符串时才执行下载。输出保存在执行命令时的**当前工作目录**，文件名包含日期时间和当前 Shell 的进程 ID；`noclobber` 会拒绝覆盖已有同名文件：

```bash
if printf '%s' "${VIDEO_RESULT:-null}" | jq -e '
  .status == "completed" and
  (.upstream.data.data[0].url | type == "string" and length > 0)
' >/dev/null; then
  VIDEO_URL="$(printf '%s' "$VIDEO_RESULT" | jq -er '.upstream.data.data[0].url')"
  VIDEO_OUTPUT="video-$(date '+%Y%m%d-%H%M%S')-$$.mp4"
  if (set -o noclobber; curl --location --silent --show-error --fail "$VIDEO_URL" > "$VIDEO_OUTPUT"); then
    printf '已保存到：%s/%s\n' "$PWD" "$VIDEO_OUTPUT"
  else
    printf '%s\n' '下载未完成或同名文件已存在；请检查当前目录中的文件。' >&2
  fi
else
  printf '%s\n' '尚未完成或结果 URL 为空；本次不下载。'
fi
```

下载是客户端直取上游 URL，未附带网关 Key。不要把带签名的结果 URL 贴进文档、工单或共享日志。若状态已完成但 URL 为空或响应结构变化，应检查原始结果并调整映射，不能据此重新提交。

历史 WorkBuddy 完成响应中，结果地址为 `.upstream.data.data[0].url`，积分为 `.upstream.data.usage.credit`。**这两个路径仅是本次接入所依据的实测 WorkBuddy 响应形状，不是永久保证，也不是网关归一化字段。** 可在本地查看该次任务的积分字段：

```bash
printf '%s' "${VIDEO_RESULT:-null}" | jq '.upstream.data.usage.credit // null'
```

HTTP 或业务错误使用网关错误信封，通常读取 `.error.message`；成功返回的任务若 `status=failed`，读取顶层字符串 `.error`。瞬态查询错误会返回错误响应，但保留内存中原有任务状态和上游负载；它不等于生成失败。终态任务再次查询直接返回缓存，不再请求上游。

## 4. 火宝本地增强版 custom 配置

以下表格对应**用户本地增强版本**已有的 custom 请求模板、类型保真渲染、点路径取值和异步轮询功能；不承诺所有官方版本已经具备同样界面或能力。配置依据只读核对了火宝 `backend/src/services/adapters/custom.ts`、`template.ts`、`url.ts`、`backend/src/routes/aiConfigs.ts` 和 `frontend/app/pages/settings.vue`，本轮未修改火宝仓库，也未做火宝端到端生成验证。

**不要用「测试连接」当无费用探针。** 当前本地实现会按 custom 模板发出真实生成请求，以测试提示词、5 秒、16:9 和启用音频构造视频请求；得到任务 ID 即可认为提交配置可用，并不证明任务最终完成或可下载。需要只读检查时，使用上一节的模型目录 GET。保存仅保存配置，不代表网络、Key、模型或生成链路已验证。

### 在设置页逐项填写

| 设置项 | 填写值 |
|:--|:--|
| 服务类型 | 视频（`video`） |
| Provider | `custom` |
| 名称 | WorkBuddy 视频（CN） |
| Base URL | 填 `VIDEO_BASE` 的实际值，例如 `http://127.0.0.1:57891`；不含 `/v1`，不填字面量 `$VIDEO_BASE` |
| API Key | 对应 CN 的无配额限制 `sk-wb` Key；后续查询保持同 Key |
| 模型 | `minimax-video-h3`（先确认 CN 当前目录可见） |
| 请求路径 | `/v1/videos` |
| 请求方法 | `POST`（本地表单默认值） |
| 请求 Headers | 下方 Headers JSON |
| 请求体模板 | 下方 Body JSON |
| 提交响应视频 URL 字段 | **留空**，避免按同步 URL 提前判完成 |
| 提交响应任务 ID 字段 | `id` |
| 轮询路径 | `/v1/videos/{{task_id}}` |
| 轮询方法 | `GET`（本地表单默认值） |
| 轮询状态字段 | `status` |
| 完成状态值 | `completed` |
| 失败状态值 | `failed` |
| 轮询结果 URL 字段 | `upstream.data.data[0].url` |
| 轮询错误字段 | `error`，不要保留默认的 `error.message` |

Headers JSON：

```json
{
  "Authorization": "Bearer {{api_key}}",
  "Content-Type": "application/json"
}
```

Body JSON（仅文生视频）：

```json
{
  "model": "{{model}}",
  "prompt": "{{prompt}}",
  "seconds": "{{duration}}",
  "resolution": "768P",
  "aspect_ratio": "{{aspect_ratio}}",
  "enable_audio": true
}
```

模板 JSON 中 `"{{duration}}"` 由本地渲染器以整串占位符保留原始数值类型：火宝记录的时长为数值 `5` 时，实际发送 `"seconds": 5`。**不要改用派生变量 `{{seconds}}`，该变量生成的是字符串。** 时长必须选当前目录允许的正整数；本模板固定 `768P`、启用音频，比例来自火宝的 `{{aspect_ratio}}`。若缺少时长或比例，本地渲染器会省略字段，网关再使用模型目录默认值。

表格中的响应与轮询映射在 `settings.custom` 内对应如下 JSON；`generate` 对象另放入上述路径、方法、Headers 和 Body。表单按字段填写，无需把这段 JSON 填入请求体：

```json
{
  "response": {
    "video_url": "",
    "task_id": "id"
  },
  "poll": {
    "path": "/v1/videos/{{task_id}}",
    "method": "GET",
    "headers": {
      "Authorization": "Bearer {{api_key}}"
    },
    "status_path": "status",
    "completed_values": ["completed"],
    "failed_values": ["failed"],
    "result_url_path": "upstream.data.data[0].url",
    "error_path": "error"
  }
}
```

本地表单没有独立轮询 Headers 输入时，适配器会默认注入相同 API Key 的 Bearer 认证。`pending` 与 `generating` 均继续等待；`failed` 结束并展示错误。结果路径是前述历史响应映射，结构变化时需重新核对。火宝后端的自动查询与网关提供后台轮询是两回事：网关只有收到查询请求才会查询上游。

### 本模板仅用于文生视频

不要在这份配置里选择首帧、尾帧或本地参考素材：模板没有发送这些字段。火宝本地图片通常会转换为 Base64/data URL，而视频网关的 `image_url` 只接受有效 HTTPS URL，两者不匹配；当前没有实现本地素材上传或 HTTPS 素材桥。

网关 API 已实现单首帧 `image_url` 的参数与模型能力校验，但图生视频没有真实生成实测。它不代表尾帧、多参考图、参考视频或音频已接通；上游目录里出现能力声明也不代表 P1 全部实现。

## 5. 已验证范围与当前边界

本次发布沿用的 **2026-11-16 历史验收**只有 CN MiniMax 的一次真实生成完成记录：**生成视频文件时长约 4.46 秒、实扣 62.29 积分**。4.46 秒指文件时长，不是生成耗时。这是单次历史观测，不是固定价格、配额或成功率承诺；本轮没有为写文档再次调用生成接口。

历史 CN 目录包含 `minimax-video-h3`、`kling-video-v3`，Global 目录包含 `kling-video-v3`；除上述 CN MiniMax 样本外，其它 CN/Global 模型仅做目录验证。**没有 Seedance、Wan 或 Kling 真实生成成功的验收结论**，也不保证目录以后不变。

| P1 边界 | 使用影响 |
|:--|:--|
| 无 `/content`、`/credits`、上传、取消接口 | 客户端从原始上游结果提取 URL/credit；网关不代下载、不归一化计费、不取消任务 |
| 无持久化、幂等、重启恢复 | 重启丢失本地任务；POST 超时不能盲重发 |
| 无后台自动轮询接口或任务调度 | 客户端自行发 GET 查询；本指南没有自动轮询脚本 |
| 已绑定 Key/region/原账号，校验账号 domain 和 Key 范围 | 同任务保持同 Key，不能跨域或换账号接管 |
| 已检查上游业务码，瞬态查询保留原状态，不自动重试 POST | 错误不应被客户端误判成可重新生成的信号 |
| 只有 WorkBuddy 上游适配 | 不等于 MiniMax/Kling 等厂商通用 API，也不等于 OpenAI 视频完整兼容 |

开发侧模块、认证和状态边界见 [视频网关架构](../specs/VIDEO-GATEWAY-ARCH.md)；其它网关接入见 [客户端接入指南](GATEWAY-CLIENT-INTEGRATION.md)。
