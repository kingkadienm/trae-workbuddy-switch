// @vitest-environment jsdom
import { act, createElement } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  isDesktop: vi.fn(() => false), isDemoMode: vi.fn(() => false),
  totpStatus: vi.fn(), getStatus: vi.fn(), setUnauthorizedHandler: vi.fn(),
  asError: (value: unknown) => String(value),
}));
vi.mock("@/lib/api", () => mocks);
vi.mock("@/components/totp-auth-page", () => ({
  TotpAuthPage: ({ onSuccess }: { onSuccess: () => void }) => createElement("button", { onClick: onSuccess }, "登录验证"),
}));
import { WebuiAuthGate } from "../webui-auth-gate";

let root: Root;
let container: HTMLDivElement;
const render = async () => { await act(async () => root.render(createElement(WebuiAuthGate, { children: createElement("main", null, "受保护内容") }))); };
beforeEach(() => {
  vi.clearAllMocks();
  sessionStorage.clear();
  mocks.isDesktop.mockReturnValue(false); mocks.isDemoMode.mockReturnValue(false);
  mocks.totpStatus.mockResolvedValue({ enabled: true }); mocks.getStatus.mockResolvedValue({});
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); });

describe("WebUI认证门", () => {
  it("启用TOTP且无会话时不挂载受保护路由", async () => {
    await render(); expect(container.textContent).toBe("登录验证"); expect(container.querySelector("main")).toBeNull();
  });
  it("禁用TOTP时直接显示内容", async () => {
    mocks.totpStatus.mockResolvedValue({ enabled: false }); await render(); expect(container.textContent).toBe("受保护内容");
  });
  it("状态检查失败时保持闭锁并允许重试", async () => {
    mocks.totpStatus.mockRejectedValueOnce(new Error("offline")); await render(); expect(container.querySelector("main")).toBeNull();
    expect(container.textContent).toContain("检查登录状态失败");
    mocks.totpStatus.mockResolvedValue({ enabled: false });
    await act(async () => container.querySelector("button")!.click()); expect(container.textContent).toBe("受保护内容");
  });
  it("服务端拒绝残留会话时不能显示内容", async () => {
    sessionStorage.setItem("totp_session", "expired"); mocks.getStatus.mockRejectedValue(new Error("401"));
    await render(); expect(mocks.getStatus).toHaveBeenCalledWith("cn"); expect(container.textContent).toBe("登录验证");
  });
  it("收到401后卸载全部受保护内容", async () => {
    sessionStorage.setItem("totp_session", "valid"); await render(); expect(container.textContent).toBe("受保护内容");
    const onUnauthorized = mocks.setUnauthorizedHandler.mock.calls[0][0] as () => void;
    await act(async () => onUnauthorized()); expect(container.querySelector("main")).toBeNull(); expect(sessionStorage.getItem("totp_session")).toBeNull();
  });
  it("桌面宿主保持原生认证路径", async () => {
    mocks.isDesktop.mockReturnValue(true); await render(); expect(container.textContent).toBe("受保护内容"); expect(mocks.totpStatus).not.toHaveBeenCalled();
  });
});
