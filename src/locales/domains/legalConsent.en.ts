/**
 * Text domain: **first-launch risk acknowledgement**.
 *
 * See the composition entry in `src/locales/zh.ts`; the three conventions there apply.
 *
 * ⚠️ Wording is constrained by product facts: it must describe the same three
 * behaviours as the code (multi-account / automation / quota forwarding), state that
 * they are off by default, and state that consequences are the user's. Keep it in sync
 * with `docs/policies/DISCLAIMER.en.md` §2.
 */
export const en = {
  "legalConsent.title": "Before you start",
  "legalConsent.intro":
    "Buddy Switch shares login state with third-party clients on this machine and sends requests to upstream services on the schedule you configure. Before continuing, confirm you understand these behaviours:",
  "legalConsent.item.accounts":
    "Multiple accounts: it reads and writes the auth data your clients already keep on this machine, to switch and renew login state.",
  "legalConsent.item.automation":
    "Automation: automatic check-in, token keep-alive and scheduled tasks send requests to the official services (off by default, enabled per item).",
  "legalConsent.item.gateway":
    "Quota forwarding: the local gateway can expose your model quota through an OpenAI-compatible API to your own clients; it binds to 127.0.0.1 by default and only forwards after you create a key.",
  "legalConsent.consequence":
    "These behaviours may not comply with the terms of service you accepted. Account restrictions, quota clawbacks, bans or legal disputes arising from them are your responsibility. Full terms: docs/policies/DISCLAIMER.en.md. If you do not agree, close and uninstall the app.",
  "legalConsent.checkbox":
    "I have read and understood the risks above and accept the consequences of use",
  "legalConsent.accept": "Agree and continue",
  "legalConsent.accepting": "Saving…",
  "legalConsent.saveFailed":
    "Could not save the acknowledgement. Check that the data directory is writable and try again.",
};
