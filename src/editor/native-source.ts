import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Annotation, ChangeSet, type Text } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";
import type { resolveTypography } from "../fonts/typography";

export const fromGtk = Annotation.define<boolean>();

type Span = { anchor: number; head: number };
type Anchor = { offset: number; fraction: number };
type Edit = { from: number; to: number; text: string };
type Key = { key: string; ctrl: boolean; alt: boolean; shift: boolean };
type Reply = {
  accepted: boolean;
  edits: Edit[];
  selection: Span;
  anchor: Anchor;
  focused: boolean;
  keys: Key[];
};
type Style = {
  ink: string;
  paper: string;
  quiet: string;
  selection: string;
  reader: ReturnType<typeof resolveTypography>["reader"];
  source: ReturnType<typeof resolveTypography>["source"];
  numbers: boolean;
  wrap: boolean;
  tabWidth: number;
};

function astral(doc: Text, from = 0) {
  const offsets: number[] = [];
  for (const line of doc) {
    for (const match of line.matchAll(/[\u{10000}-\u{10ffff}]/gu))
      offsets.push(from + match.index);
    from += line.length;
  }
  return offsets;
}

// GTK uses scalar offsets. The source protocol and CodeMirror use UTF-16.
// Index only the characters whose width differs between the two.
class NativeDocument {
  private offsets: number[];

  constructor(
    public text: Text,
    offsets = astral(text),
  ) {
    this.offsets = offsets;
  }

  copy() {
    return new NativeDocument(this.text, this.offsets);
  }

  private before(predicate: (offset: number, index: number) => boolean) {
    let start = 0;
    let end = this.offsets.length;
    while (start < end) {
      const middle = (start + end) >>> 1;
      if (predicate(this.offsets[middle], middle)) start = middle + 1;
      else end = middle;
    }
    return start;
  }

  scalar(pos: number) {
    return pos - this.before((offset) => offset < pos);
  }

  utf16(pos: number) {
    return pos + this.before((offset, index) => offset - index < pos);
  }

  apply(changes: ChangeSet) {
    if (changes.empty) return;
    const offsets: number[] = [];
    let index = 0;
    let shift = 0;
    changes.iterChanges((from, to, start, end, inserted) => {
      while (index < this.offsets.length && this.offsets[index] < from)
        offsets.push(this.offsets[index++] + shift);
      while (index < this.offsets.length && this.offsets[index] < to) index++;
      for (const offset of astral(inserted, start)) offsets.push(offset);
      shift = end - to;
    });
    while (index < this.offsets.length)
      offsets.push(this.offsets[index++] + shift);
    this.offsets = offsets;
    this.text = changes.apply(this.text);
  }
}

function boxFor(element: HTMLElement) {
  if (
    element.closest("[hidden], [inert]") ||
    document.querySelector(
      "dialog[open], .formatting-tools[open], .document-actions[open]",
    )
  )
    return null;
  const box = element.getBoundingClientRect();
  const x = Math.max(0, Math.round(box.left));
  const y = Math.max(0, Math.round(box.top));
  const width = Math.min(innerWidth, Math.round(box.right)) - x;
  const height = Math.min(innerHeight, Math.round(box.bottom)) - y;
  if (width < 16 || height < 16) return null;

  // GTK is above the WebView. Lend the space back while a web menu overlaps it.
  for (const [px, py] of [
    [x + 5, y + 5],
    [x + width - 5, y + 5],
    [x + 5, y + height - 5],
    [x + width - 5, y + height - 5],
    [x + width / 2, y + height / 2],
  ]) {
    if (!element.contains(document.elementFromPoint(px, py))) return null;
  }
  return { x, y, width, height };
}

export class NativeSource {
  private readonly token = crypto.randomUUID();
  private readonly base: NativeDocument;
  private readonly sizeObserver: ResizeObserver;
  private readonly domObserver: MutationObserver;
  private readonly ready: Promise<void>;
  private unlisten?: UnlistenFn;
  private pending: ChangeSet;
  private running?: Promise<void>;
  private dirty = false;
  private closed = false;
  private frame = 0;
  private layoutRunning = false;
  private lastBox = "";
  private lastStyle = "";
  private enabled = false;
  private style?: Style;
  private moveSelection = false;
  private takeFocus = false;
  private reveal?: { line: number; fraction: number };
  private lastAnchor: Anchor | undefined;
  visible = false;
  editable = true;

  constructor(
    private view: EditorView,
    private reportAnchor: (line: number, fraction: number) => void,
    private handleKey: (key: Key) => void,
    private reportError: (error: unknown) => void,
  ) {
    this.base = new NativeDocument(view.state.doc);
    this.pending = ChangeSet.empty(this.base.text.length);
    const selection = this.offsets(this.base);
    const initial = this.base.text.toString();
    this.ready = (async () => {
      this.unlisten = await listen<string>("native-source-change", (event) => {
        if (event.payload === this.token) void this.flush().catch(() => {});
      });
      if (this.closed) {
        this.unlisten();
        return;
      }
      await invoke("native_source_open", {
        token: this.token,
        text: initial,
        selection,
      });
      this.scheduleLayout();
    })();
    void this.ready.catch((error) => this.fail(error));

    this.sizeObserver = new ResizeObserver(this.scheduleLayout);
    this.sizeObserver.observe(view.scrollDOM);
    this.domObserver = new MutationObserver(this.scheduleLayout);
    this.domObserver.observe(document.body, {
      subtree: true,
      attributes: true,
      attributeFilter: ["open", "inert", "hidden", "class", "style"],
      childList: true,
    });
    window.addEventListener("resize", this.scheduleLayout);
    document.addEventListener("scroll", this.scheduleLayout, true);
  }

  configure(enabled: boolean, style: Style) {
    this.enabled = enabled;
    this.style = style;
    this.scheduleLayout();
  }

  private scheduleLayout = () => {
    if (!this.closed && !this.frame) {
      this.frame = requestAnimationFrame(() => {
        this.frame = 0;
        void this.layout();
      });
    }
  };

  private async layout() {
    if (this.closed || this.layoutRunning) return;
    this.layoutRunning = true;
    try {
      await this.ready;
      if (this.closed) return;
      const rect =
        this.enabled &&
        this.view.state.selection.ranges.length === 1 &&
        !this.view.dom.querySelector(".cm-search, .cm-foldPlaceholder")
          ? boxFor(this.view.scrollDOM)
          : null;
      const box = JSON.stringify(rect);
      const style = rect ? JSON.stringify(this.style) : this.lastStyle;
      if (box === this.lastBox && style === this.lastStyle) return;
      const visible = await invoke<boolean>("native_source_layout", {
        token: this.token,
        rect,
        appearance: style === this.lastStyle ? null : this.style,
      });
      if (this.closed) return;
      this.visible = visible;
      if (visible) this.view.scrollDOM.setAttribute("aria-hidden", "true");
      else this.view.scrollDOM.removeAttribute("aria-hidden");
      this.view.contentDOM.tabIndex = visible ? -1 : 0;
      this.view.dom.toggleAttribute("data-native-source", visible);
      this.lastBox = box;
      this.lastStyle = style;
      // Layout and tab visibility can change while the GTK call is in flight.
      this.scheduleLayout();
    } catch (error) {
      this.fail(error);
    } finally {
      this.layoutRunning = false;
    }
  }

  changes(changes: ChangeSet, selectionSet: boolean) {
    if (this.closed) return;
    this.pending = this.pending.compose(changes);
    this.moveSelection ||= selectionSet;
    void this.flush().catch(() => {});
  }

  focus() {
    this.takeFocus = true;
    this.moveSelection = true;
    void this.flush().catch(() => {});
  }

  setEditable(editable: boolean) {
    if (editable === this.editable) return;
    this.editable = editable;
    void this.flush().catch(() => {});
  }

  scrollTo(line: number, fraction: number) {
    if (!this.visible) return;
    this.reveal = { line, fraction };
    void this.flush().catch(() => {});
  }

  private offsets(doc: NativeDocument): Span {
    const { anchor, head } = this.view.state.selection.main;
    return { anchor: doc.scalar(anchor), head: doc.scalar(head) };
  }

  flush(): Promise<void> {
    if (this.closed) return Promise.resolve();
    this.dirty = true;
    return (this.running ??= this.run().finally(() => {
      this.running = undefined;
    }));
  }

  private async run() {
    try {
      await this.ready;
      while (this.dirty && !this.closed) {
        this.dirty = false;
        await this.synchronize();
      }
    } catch (error) {
      this.fail(error);
      throw error;
    }
  }

  private async synchronize() {
    const sent = this.pending;
    this.pending = ChangeSet.empty(sent.newLength);
    const edits: Edit[] = [];
    sent.iterChanges((from, to, _start, _end, text) => {
      edits.push({
        from: this.base.scalar(from),
        to: this.base.scalar(to),
        text: text.toString(),
      });
    });
    const projected = this.base.copy();
    projected.apply(sent);
    const selection = this.moveSelection ? this.offsets(projected) : null;
    const requestedAnchor = this.reveal;
    const reveal = requestedAnchor
      ? {
          offset: projected.scalar(
            projected.text.line(
              Math.max(1, Math.min(projected.text.lines, requestedAnchor.line)),
            ).from,
          ),
          fraction: requestedAnchor.fraction,
        }
      : null;
    const focus = this.takeFocus;
    this.moveSelection = this.takeFocus = false;
    this.reveal = undefined;
    const reply = await invoke<Reply>("native_source_sync", {
      token: this.token,
      update: { edits, selection, reveal, editable: this.editable, focus },
    });
    if (this.closed) return;

    if (reply.accepted) this.base.apply(sent);
    else {
      // A key or paste can land while a toolbar edit is on its way to GTK.
      // Rebase each side over the other using the same CodeMirror change algebra.
      const local = sent.compose(this.pending);
      this.pending = local;
      const original = this.base.text;
      let remote = ChangeSet.empty(original.length);
      for (const edit of reply.edits) {
        const change = ChangeSet.of(
          {
            from: this.base.utf16(edit.from),
            to: this.base.utf16(edit.to),
            insert: edit.text,
          },
          this.base.text.length,
        );
        remote = remote.compose(change);
        this.base.apply(change);
      }
      const mapped = remote.map(local, true);
      this.pending = local.map(remote);
      const expected = mapped.apply(this.view.state.doc);
      this.moveSelection ||= Boolean(selection);
      this.view.dispatch({
        changes: mapped,
        selection: this.moveSelection
          ? this.view.state.selection.map(mapped)
          : this.nativeSelection(reply.selection),
        annotations: fromGtk.of(true),
      });
      if (!expected.eq(this.view.state.doc)) {
        // The existing recovery/Unicode transaction filter refused the input.
        // Restore the native widget to the accepted document.
        this.pending = remote.invert(original).compose(local);
        this.moveSelection = true;
      }
      this.takeFocus ||= focus;
      this.dirty ||=
        !this.pending.empty || this.moveSelection || this.takeFocus;
    }
    if (
      reply.accepted &&
      this.visible &&
      reply.focused &&
      !selection &&
      !this.moveSelection
    ) {
      const selection = this.nativeSelection(reply.selection);
      const current = this.view.state.selection.main;
      if (
        current.anchor !== selection.anchor ||
        current.head !== selection.head
      )
        this.view.dispatch({ selection, annotations: fromGtk.of(true) });
    }
    if (
      this.visible &&
      !reveal &&
      (reply.anchor.offset !== this.lastAnchor?.offset ||
        reply.anchor.fraction !== this.lastAnchor?.fraction)
    ) {
      const offset = this.pending.mapPos(this.base.utf16(reply.anchor.offset));
      this.reportAnchor(
        this.view.state.doc.lineAt(offset).number,
        reply.anchor.fraction,
      );
    }
    this.lastAnchor = reply.anchor;
    for (const key of reply.keys) this.handleKey(key);
  }

  private nativeSelection(selection: Span) {
    return {
      anchor: this.pending.mapPos(this.base.utf16(selection.anchor)),
      head: this.pending.mapPos(this.base.utf16(selection.head)),
    };
  }

  private fail(error: unknown) {
    if (this.closed) return;
    this.reportError(error);
    this.destroy();
  }

  destroy() {
    if (this.closed) return;
    this.closed = true;
    this.visible = false;
    this.view.scrollDOM.removeAttribute("aria-hidden");
    this.view.contentDOM.tabIndex = 0;
    this.view.dom.removeAttribute("data-native-source");
    cancelAnimationFrame(this.frame);
    this.domObserver.disconnect();
    this.sizeObserver.disconnect();
    window.removeEventListener("resize", this.scheduleLayout);
    document.removeEventListener("scroll", this.scheduleLayout, true);
    this.unlisten?.();
    void this.ready
      .then(() => invoke("native_source_close", { token: this.token }))
      .catch(() => {});
  }
}
