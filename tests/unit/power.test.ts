import { describe, it, expect } from "vitest";
import { resources } from "../../src/power/assets";
describe("PDF capture resource planning", () => {
  it("uses the shared grammar for references and keeps executable fences inert", () => {
    const source =
      "# Capture\r\n\r\n![one][picture]\r\n![again][picture]\n\n[picture]: local.png\n\n```mermaid\ngraph TD\nA-->B\n```\n\n```sh\nprintf never-automatic\n```\n";
    const r = resources(source);
    expect(r.images).toEqual(["local.png"]);
    expect(r.diagrams).toEqual(["graph TD\nA-->B\n"]);
  });
  it("bounds image offers and diagram jobs without resolving paths", () => {
    expect(() =>
      resources(
        Array.from({ length: 17 }, (_, i) => `![x](file${i}.png)`).join("\n"),
      ),
    ).toThrow("16 image");
    const r = resources(
      "![untrusted](https://invalid/pixel.png)\n\n<img src='file:///etc/passwd'>",
    );
    expect(r.images).toEqual(["https://invalid/pixel.png"]);
  });
});
