/**
 * 文案域：**豆包账号管理页**（账号池 / 探测 / 保存 / 移除 / 保活）。
 *
 * 域划分见 `src/locales/zh.ts` 的组合入口。改动本文件请遵守那里的三条约定。
 */
export const zh = {
  "doubao.title": "豆包账号",
  "doubao.empty": "暂无豆包账号，请先在豆包客户端登录，或手动添加 user_id。",
  "doubao.current": "当前登录",
  "doubao.addAccount": "添加账号",
  "doubao.detect.action": "探测登录",
  "doubao.keepalive.action": "保活",
  "doubao.cancel": "取消",

  "doubao.session.ok": "有效",
  "doubao.session.expired": "已过期",
  "doubao.session.unknown": "未知",
  "doubao.session.none": "无会话",

  "doubao.detect.success": "探测到当前登录账号：{uid}",
  "doubao.detect.empty": "未探测到豆包客户端登录标记（请先在客户端登录）",
  "doubao.detect.error": "探测失败：{error}",

  "doubao.addDialogTitle": "添加豆包账号",
  "doubao.addDialogDescription": "填入豆包 user_id（与快照槽目录名一致），可附带别名与备注。",
  "doubao.fieldUserId": "User ID",
  "doubao.fieldName": "别名",
  "doubao.fieldNote": "备注",
  "doubao.save.submit": "保存并切换",
  "doubao.save.success": "账号 {uid} 已保存并设为当前登录",
  "doubao.save.error": "保存失败：{error}",
  "doubao.save.emptyUserId": "User ID 不能为空",

  "doubao.removeDialogTitle": "移除豆包账号",
  "doubao.removeDialogDescription": "将移除账号 {uid}；勾选后同时删除其快照槽。",
  "doubao.removeSnapshotLabel": "同时删除快照槽",
  "doubao.remove.submit": "移除",
  "doubao.remove.success": "账号 {uid} 已移除",
  "doubao.remove.error": "移除失败：{error}",
  "doubao.removeAction": "移除该账号",

  "doubao.keepalive.success": "保活标记已更新",
  "doubao.keepalive.error": "保活失败：{error}",

  "doubao.lastModifiedTooltip": "快照槽最近修改时间",
} as const;
