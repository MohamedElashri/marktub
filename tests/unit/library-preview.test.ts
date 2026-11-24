// @vitest-environment jsdom
import { mount, tick, unmount } from "svelte";
import { expect, it, vi } from "vitest";
import Preview from "../../src/library/QuickPreview.svelte";
import type { Item } from "../../src/library";

const { preview } = vi.hoisted(() => ({
  preview: vi.fn<(id: string) => Promise<string>>(),
}));
vi.mock("../../src/library", () => ({ preview }));

it("keeps the newest preview and reports only errors from its current request", async () => {
  const requests = new Map<
    string,
    { resolve: (text: string) => void; reject: (error: Error) => void }
  >();
  preview.mockImplementation(
    (id) =>
      new Promise((resolve, reject) => {
        requests.set(id, { resolve, reject });
      }),
  );
  const target = document.createElement("div");
  document.body.append(target);
  const component = mount(Preview, { target });
  await tick();
  const dialog = target.querySelector("dialog")!;
  dialog.showModal = () => dialog.setAttribute("open", "");
  dialog.close = () => dialog.removeAttribute("open");
  const file = (id: string): Item => ({
    id,
    name: id,
    kind: "file",
    bytes: 1,
    modified: 0,
    directory: null,
    parent: { id: "root", name: "root", root: "root" },
  });
  try {
    const old = component.show(file("old.md"));
    dialog.close();
    const next = component.show(file("next.md"));
    requests.get("next.md")!.resolve("# New preview");
    await next;
    requests.get("old.md")!.resolve("# Obsolete preview");
    await old;
    await tick();
    expect(dialog.querySelector("h2")?.textContent).toBe("next.md");
    expect(dialog.querySelector("article h1")?.textContent).toBe("New preview");

    const obsolete = component.show(file("obsolete.md"));
    const error = component.show(file("missing.md"));
    requests.get("missing.md")!.reject(new Error("Cannot read this file."));
    await error;
    requests.get("obsolete.md")!.reject(new Error("Obsolete error"));
    await obsolete;
    await tick();
    expect(dialog.querySelector('[role="alert"]')?.textContent).toBe(
      "Cannot read this file.",
    );
    expect(dialog.querySelector("article")).toBeNull();
  } finally {
    await unmount(component);
    target.remove();
  }
});
