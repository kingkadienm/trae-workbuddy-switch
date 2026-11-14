#!/bin/sh
# Buddy Switch 容器入口。
#
# 只做一件事：**保证 `BUDDY_SWITCH_HOME` 存在**，然后把参数原样交给 CLI。
#
# ## 为什么必须有这一步
#
# CLI 对 `BUDDY_SWITCH_HOME` 是 **fail-closed** 的（`main.rs::refuse_when_home_override_is_invalid`）：
# 变量被设置、但指向一个**不存在**的目录时，它会**拒绝启动**（exit 2）并打印理由。
# 这不是过度谨慎 —— 2026-09-30 的真实事故就是「调用方以为在隔离目录里跑，
# 实际把账号 / 网关 Key / 签到与积分记录写到了用户真实数据上」。
#
# 容器里这个目录常常是刚挂上来的空卷（尤其是 bind mount），所以入口负责建它。
# 建不出来（权限不足）就让 `mkdir` 自己失败退出 —— 那说明挂载的属主不对，
# 静默继续只会得到一个「启动失败」且看不出原因的容器。

set -eu

if [ -n "${BUDDY_SWITCH_HOME:-}" ]; then
  mkdir -p "$BUDDY_SWITCH_HOME"
fi

exec buddy-switch "$@"
