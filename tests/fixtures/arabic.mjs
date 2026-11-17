const paragraph =
  "تُحافظُ القراءةُ الواضحةُ على ترتيب الكلمات واتّصال الحروف، حتى عندما يطول المستند وتلتفّ الأسطر داخل مساحة ضيّقة. نكتب العربية مع علامات الترقيم (مثال توضيحي)، والأرقام ١٢٣ و456، ثم نتابع الجملة دون تغيير موضع الكلمات. ";

export const arabicOpening = [
  "# القراءة بالعربية مع English",
  paragraph.repeat(12),
  "2026/11/17 — هذه فقرة تبدأ بأرقام، وتضم **كلمات عربية بارزة** و*نصًا مائلًا* ثم [مرجعًا عربيًا](notes.md) و[English reference](notes.md). وفيها الأمر `printf '%s\\n' Arabic` بين كلمتين عربيتين، والمعادلة $a^2+b^2=c^2$ ضمن الفقرة نفسها.",
  "English paragraph with العربية in the middle, numbers 123.45, and **English emphasis**. Its base direction stays left to right.",
  "`git status` ثم نكمل الشرح بالعربية؛ الشيفرة مستقلة عن اتجاه الجملة.",
  "- عنصر عربي أول\n- English item with العربية\n  - عنصر عربي متداخل\n- [x] مهمة عربية مكتملة\n- [ ] مهمة عربية غير مكتملة",
  "1. خطوة عربية أولى\n2. English step with العربية\n3. خطوة عربية أخيرة",
  "> اقتباس عربي طويل يوضح موضع الحافة والمسافة الداخلية.\n>\n> English paragraph inside the same quotation.",
  "| الوصف | Code | القيمة |\n| --- | :---: | ---: |\n| كلمات عربية | `value + 1` | ١٢٣ |\n| English row | `x = 42` | 456 |",
  "## [عنوان عربي مرتبط](notes.md)",
  "[فقرة عربية مرتبطة بالكامل](notes.md)",
  "[English label](notes.md) ثم كلمات عربية.",
  "- فقرة عربية في قائمة متباعدة.\n\n- English paragraph in a loose list.",
  "> [!NOTE]\n> هذه ملاحظة عربية مع [رابط عربي](notes.md).",
  '```sh\nprintf "%s\\n" "العربية English ١٢٣"\n```',
].join("\n\n");

export const longArabicDocument =
  "\uFEFF" +
  arabicOpening +
  "\n\n" +
  Array.from(
    { length: 240 },
    (_, i) =>
      `## فصل ${i + 1}\n\n${paragraph.repeat(6)}\n\nEnglish section ${i + 1}: العربية alongside local notes, file.md and 123.45.\n`,
  ).join("\n");

export const readerFlowDocument = Array.from(
  { length: 72 },
  (_, chapter) =>
    `# Chapter ${chapter}\n\n` +
    Array.from({ length: 64 }, (_, item) =>
      `- [كلمات عربية ${item}](notes.md) with English text and 123.45.`,
    ).join("\n") + "\n\n",
).join("");
