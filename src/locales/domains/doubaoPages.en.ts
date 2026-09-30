/**
 * Copy domain: **Doubao accounts page** (account pool / detection / save / remove / keepalive).
 *
 * Domain layout is defined by the composition entry in `src/locales/zh.ts`.
 */
export const en = {
  "doubao.title": "Doubao Accounts",
  "doubao.empty": "No Doubao accounts yet. Log in on the Doubao client first, or add a user_id manually.",
  "doubao.current": "Current",
  "doubao.addAccount": "Add Account",
  "doubao.detect.action": "Detect Login",
  "doubao.keepalive.action": "Keepalive",
  "doubao.cancel": "Cancel",

  "doubao.session.ok": "Valid",
  "doubao.session.expired": "Expired",
  "doubao.session.unknown": "Unknown",
  "doubao.session.none": "No Session",

  "doubao.detect.success": "Detected current login: {uid}",
  "doubao.detect.empty": "No Doubao client login marker found (log in on the client first)",
  "doubao.detect.error": "Detection failed: {error}",

  "doubao.addDialogTitle": "Add Doubao Account",
  "doubao.addDialogDescription": "Enter the Doubao user_id (same as the snapshot slot directory name), with an optional alias and note.",
  "doubao.fieldUserId": "User ID",
  "doubao.fieldName": "Alias",
  "doubao.fieldNote": "Note",
  "doubao.save.submit": "Save & Switch",
  "doubao.save.success": "Account {uid} saved and set as current login",
  "doubao.save.error": "Save failed: {error}",
  "doubao.save.emptyUserId": "User ID must not be empty",

  "doubao.removeDialogTitle": "Remove Doubao Account",
  "doubao.removeDialogDescription": "Remove account {uid}; when checked, its snapshot slot is deleted too.",
  "doubao.removeSnapshotLabel": "Delete snapshot slot as well",
  "doubao.remove.submit": "Remove",
  "doubao.remove.success": "Account {uid} removed",
  "doubao.remove.error": "Remove failed: {error}",
  "doubao.removeAction": "Remove this account",

  "doubao.keepalive.success": "Keepalive marker updated",
  "doubao.keepalive.error": "Keepalive failed: {error}",

  "doubao.lastModifiedTooltip": "Last snapshot modification time",
} as const;
