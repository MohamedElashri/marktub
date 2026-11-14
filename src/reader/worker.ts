import { renderSection } from "./render";

self.onmessage = (
  event: MessageEvent<{ generation: number; text: string }>,
) => {
  const { generation, text } = event.data;
  try {
    self.postMessage({ generation, result: renderSection(text) });
  } catch (error) {
    self.postMessage({
      generation,
      error: error instanceof Error ? error.message : "Preview failed.",
    });
  }
};
