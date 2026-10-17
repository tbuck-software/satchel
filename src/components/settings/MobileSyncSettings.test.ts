// @vitest-environment jsdom
import { mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

const backend = vi.hoisted(() => ({
  backendBackgroundSyncStatus: vi.fn(),
  backendListAccounts: vi.fn(),
  backendOpenBackgroundSyncSettings: vi.fn(),
}));
vi.mock("@/services/backend", () => backend);

import MobileSyncSettings from "./MobileSyncSettings.vue";

beforeEach(() => {
  vi.clearAllMocks();
  backend.backendBackgroundSyncStatus.mockResolvedValue({
    enabled: true,
    running: true,
    batteryOptimizationExempt: false,
  });
  backend.backendListAccounts.mockResolvedValue([
    { kind: "email", last_sync_at: "2026-08-20T10:30:00Z" },
  ]);
  backend.backendOpenBackgroundSyncSettings.mockResolvedValue(undefined);
});

describe("Android background sync diagnostics", () => {
  it("shows local service health instead of relay setup", async () => {
    const wrapper = mount(MobileSyncSettings);
    await vi.waitFor(() => expect(wrapper.text()).toContain("Active"));

    expect(wrapper.text()).toContain("Service");
    expect(wrapper.text()).toContain("Optimised");
    expect(wrapper.text()).toContain("Last successful fetch");
    expect(wrapper.text()).not.toContain("Relay URL");
  });

  it("opens Android settings so the user can remove battery restrictions", async () => {
    const wrapper = mount(MobileSyncSettings);
    await vi.waitFor(() => expect(wrapper.text()).toContain("Optimised"));

    const open = wrapper.findAll("button").find((button) => button.text().includes("Open Android app settings"));
    await open!.trigger("click");

    expect(backend.backendOpenBackgroundSyncSettings).toHaveBeenCalledOnce();
  });
});
