import hljs from "highlight.js/lib/core";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import rust from "highlight.js/lib/languages/rust";
import bash from "highlight.js/lib/languages/bash";
import { escape } from "../reader/grammar";
for (const [name, language] of Object.entries({ javascript, json, rust, bash }))
  hljs.registerLanguage(name, language);
export function highlight(text: string, language: string) {
  return hljs.getLanguage(language)
    ? hljs.highlight(text, { language, ignoreIllegals: true }).value
    : escape(text);
}
