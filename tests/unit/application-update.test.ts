// @vitest-environment jsdom
import { mount, unmount } from "svelte";
import { afterEach, expect, it, vi } from "vitest";
import Updates from "../../src/Updates.svelte";

const { invoke } = vi.hoisted(() => ({
  invoke: vi.fn<(command: string) => Promise<unknown>>(),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const ready = {
  phase: "ready",
  platform: "linux-x86_64-deb",
  canInstall: true,
  currentVersion: "0.3.7",
  version: "0.3.8",
  notes: "",
  received: 1024,
  total: 1024,
  error: null as string | null,
};
let component: ReturnType<typeof mount> | undefined;

function showUpdates() {
  const restart = vi.fn(async () => {});
  component = mount(Updates, {
    target: document.body,
    props: { active: true, restartDisabled: false, onrestart: restart },
  });
  return restart;
}

function button(label: string) {
  return Array.from(document.querySelectorAll("button")).find(
    (button) => button.textContent?.trim() === label,
  );
}

afterEach(async () => {
  if (component) await unmount(component);
  component = undefined;
  invoke.mockReset();
});

it("waits for Linux package installation before restarting and can retry restart alone", async () => {
  let complete = () => {};
  const installation = new Promise<void>((resolve) => {
    complete = resolve;
  });
  let status = ready;
  invoke.mockImplementation(async (command) => {
    if (command === "application_update_status") return status;
    if (command === "apply_linux_application_update") {
      status = { ...ready, phase: "installing" };
      await installation;
      status = { ...ready, phase: "installed" };
    }
  });
  const restart = showUpdates();
  await vi.waitFor(() => expect(button("Install and restart")).toBeDefined());
  expect(invoke.mock.calls).toEqual([["application_update_status"]]);

  button("Install and restart")?.click();
  await vi.waitFor(() =>
    expect(document.body.textContent).toContain(
      "Waiting for the system package installer",
    ),
  );
  expect(restart).not.toHaveBeenCalled();
  complete();
  await vi.waitFor(() => expect(restart).toHaveBeenCalledTimes(1));
  await vi.waitFor(() =>
    expect(button("Restart Marktub")?.disabled).toBe(false),
  );
  button("Restart Marktub")?.click();
  await vi.waitFor(() => expect(restart).toHaveBeenCalledTimes(2));
  expect(
    invoke.mock.calls.filter(
      ([cmd]) => cmd === "apply_linux_application_update",
    ),
  ).toHaveLength(1);
});

it("keeps the verified update ready and offers manual installation after an error", async () => {
  const failure = "System installation was cancelled.";
  let status = ready;
  invoke.mockImplementation(async (command) => {
    if (command === "application_update_status") return status;
    if (command === "apply_linux_application_update") {
      status = { ...ready, error: failure };
      throw new Error(failure);
    }
  });
  const restart = showUpdates();
  await vi.waitFor(() => expect(button("Install and restart")).toBeDefined());
  button("Install and restart")?.click();
  await vi.waitFor(() => expect(button("Open system installer")).toBeDefined());
  expect(document.querySelector('[role="alert"]')?.textContent).toContain(
    failure,
  );
  expect(restart).not.toHaveBeenCalled();

  button("Open system installer")?.click();
  await vi.waitFor(() =>
    expect(button("Restart after installation")?.disabled).toBe(false),
  );
  expect(invoke).toHaveBeenCalledWith("open_application_installer");
  expect(restart).not.toHaveBeenCalled();
});
