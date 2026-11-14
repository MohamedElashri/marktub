// @vitest-environment jsdom
import { expect, it } from "vitest";
import { sanitizePreview } from "../../src/reader/sanitize";
import { renderSection } from "../../src/reader/render";

it("keeps hostile Markdown inert without link/image/native surfaces", () => {
  const source =
    '<img src="https://attacker.invalid/a" onerror="alert(1)">\n\n![secret](https://attacker.invalid/a)\n\n![denied](file:///etc/passwd)\n\n[run](javascript:alert(1))\n\n[remote](https://attacker.invalid/)';
  const host = document.createElement("div");
  host.innerHTML = sanitizePreview(renderSection(source).html);
  expect(host.querySelector("img,script,iframe,a,svg,math,object")).toBeNull();
  expect(host.textContent).toContain("Image: secret");
});

it("sanitizes generated output even if a renderer emits dangerous markup", () => {
  const host = document.createElement("div");
  host.innerHTML = sanitizePreview(
    '<p id="__TAURI__" onclick="run()" style="background:url(https://attacker.invalid)">ok</p><svg onload="run()"></svg><iframe src="file:///etc/passwd"></iframe>',
  );
  expect(host.innerHTML).toBe("<p>ok</p>");
});
