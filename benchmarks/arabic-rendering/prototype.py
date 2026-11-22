"""Run the GTK and WebKitGTK Arabic editing prototypes on Linux."""

import argparse
import ctypes
import hashlib
import json
import math
import os
from pathlib import Path
import statistics
import sys
import time
from urllib.parse import urlencode

import cairo
import gi

gi.require_foreign("cairo")
gi.require_version("Gtk", "3.0")
gi.require_version("Gdk", "3.0")
gi.require_version("PangoCairo", "1.0")
gi.require_version("WebKit2", "4.1")
gi.require_version("JavaScriptCore", "4.1")
from gi.repository import Gdk, GLib, Gtk, JavaScriptCore, Pango, PangoCairo, WebKit2

HERE = Path(__file__).resolve().parent
SAMPLE = (HERE / "sample.txt").read_text(encoding="utf-8").rstrip()
DOCUMENT = (SAMPLE + "\n") * 399 + SAMPLE
FONT = "Noto Naskh Arabic,Cousine"
FONT_SIZE = 26
LINE_HEIGHT = 44
ARABIC_INSERT = "حَقًّا عَرَبِيٌّ وَكِتَابٍ "
PATH_COMMANDS = {
    cairo.PATH_MOVE_TO: "M",
    cairo.PATH_LINE_TO: "L",
    cairo.PATH_CURVE_TO: "C",
    cairo.PATH_CLOSE_PATH: "Z",
}


class OutlineRenderer:
    def __init__(self):
        self.surface = cairo.RecordingSurface(cairo.CONTENT_ALPHA, None)
        self.cairo = cairo.Context(self.surface)
        self.layout = PangoCairo.create_layout(self.cairo)
        self.layout.get_context().set_round_glyph_positions(False)
        font = Pango.FontDescription.from_string(FONT)
        font.set_absolute_size(FONT_SIZE * Pango.SCALE)
        self.layout.set_font_description(font)
        self.timings = []

    def outline(self, text):
        self.layout.set_text(text, -1)
        _, bounds = self.layout.get_extents()
        self.cairo.new_path()
        self.cairo.move_to(0, (LINE_HEIGHT - bounds.height / Pango.SCALE) / 2)
        PangoCairo.layout_path(self.cairo, self.layout)
        path = " ".join(
            PATH_COMMANDS[kind] + " ".join(f"{v:.3f}" for v in coordinates)
            for kind, coordinates in self.cairo.copy_path()
        )
        return {"path": path, "width": bounds.width / Pango.SCALE}

    def reply(self, _manager, value, reply):
        started = time.perf_counter()
        texts = json.loads(value.to_json(0))
        rows = [self.outline(text) for text in texts]
        result = JavaScriptCore.Value.new_from_json(
            value.get_context(), json.dumps(rows, separators=(",", ":"))
        )
        self.timings.append((time.perf_counter() - started) * 1000)
        reply.return_value(result)
        return True


class X11Keyboard:
    def __init__(self):
        self.x11 = ctypes.CDLL("libX11.so.6")
        self.xtest = ctypes.CDLL("libXtst.so.6")
        self.x11.XOpenDisplay.argtypes = [ctypes.c_char_p]
        self.x11.XOpenDisplay.restype = ctypes.c_void_p
        self.x11.XStringToKeysym.argtypes = [ctypes.c_char_p]
        self.x11.XStringToKeysym.restype = ctypes.c_ulong
        self.x11.XKeysymToKeycode.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
        self.x11.XKeysymToKeycode.restype = ctypes.c_uint
        self.x11.XFlush.argtypes = [ctypes.c_void_p]
        self.xtest.XTestFakeKeyEvent.argtypes = [
            ctypes.c_void_p, ctypes.c_uint, ctypes.c_int, ctypes.c_ulong
        ]
        self.display = self.x11.XOpenDisplay(None)
        if not self.display:
            raise RuntimeError("The benchmark requires an X11 display.")

    def press(self, key):
        symbol = self.x11.XStringToKeysym(key.encode("ascii"))
        code = self.x11.XKeysymToKeycode(self.display, symbol)
        if not code:
            raise RuntimeError(f"Unable to map X11 key {key}")
        for pressed in (1, 0):
            self.xtest.XTestFakeKeyEvent(self.display, code, pressed, 0)
        self.x11.XFlush(self.display)


def describe(samples):
    ordered = sorted(samples)
    return {
        "count": len(samples),
        "median_ms": round(statistics.median(ordered), 2),
        "p95_ms": round(ordered[math.ceil(0.95 * len(ordered)) - 1], 2),
        "max_ms": round(ordered[-1], 2),
    }


class Prototype:
    def __init__(self, args):
        self.args = args
        self.window = Gtk.Window(title=f"Marktub Arabic prototype · {args.mode}")
        self.window.set_default_size(args.width, 420)
        self.window.connect("destroy", Gtk.main_quit)
        self.renderer = OutlineRenderer() if args.mode == "overlay" else None
        self.failure = None
        self.timings = []

        if args.mode == "native":
            self.editor = Gtk.TextView(wrap_mode=Gtk.WrapMode.WORD_CHAR)
            style = Gtk.CssProvider()
            style.load_from_data(b'textview { font: 26px "Noto Naskh Arabic", Cousine; }')
            self.editor.get_style_context().add_provider(
                style, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION
            )
            self.editor.set_left_margin(16)
            self.editor.set_right_margin(16)
            self.editor.set_top_margin(8)
            self.editor.set_cursor_visible(not args.bench)
            self.editor.get_buffer().set_text(DOCUMENT)
            self.editor.get_buffer().place_cursor(self.editor.get_buffer().get_start_iter())
            scroll = Gtk.ScrolledWindow(overlay_scrolling=not args.bench)
            scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
            scroll.add(self.editor)
            self.window.add(scroll)
        else:
            manager = WebKit2.UserContentManager()
            if self.renderer:
                manager.connect("script-message-with-reply-received::pango", self.renderer.reply)
                manager.register_script_message_handler_with_reply("pango", None)
            manager.connect("script-message-received::report", self.web_report)
            manager.register_script_message_handler("report")
            self.editor = WebKit2.WebView.new_with_user_content_manager(manager)
            self.editor.get_settings().set_allow_file_access_from_file_urls(True)
            self.window.add(self.editor)
            params = {"mode": args.mode}
            if args.bench:
                params["bench"] = "1"
            url = (args.web / "index.html").resolve().as_uri()
            self.editor.load_uri(f"{url}?{urlencode(params)}")

        self.window.show_all()
        self.window.present()
        if args.mode == "native":
            self.editor.grab_focus()
            self.after_ready()

    def web_report(self, _manager, message):
        report = json.loads(message.get_js_value().to_json(0))
        if "error" in report:
            self.fail(report["error"])
        if "ready" in report:
            self.after_ready()

    def after_ready(self):
        if self.args.bench:
            GLib.timeout_add(350, self.start_benchmark)
        elif self.args.smoke:
            GLib.timeout_add(200, self.verify_text)

    def fail(self, message):
        self.failure = str(message)
        print(self.failure, file=sys.stderr, flush=True)
        Gtk.main_quit()

    def evaluate(self, script, callback):
        def finished(view, result, _data):
            try:
                value = view.evaluate_javascript_finish(result)
                callback(json.loads(value.to_json(0)))
            except Exception as error:
                self.fail(error)

        self.editor.evaluate_javascript(script, -1, None, None, None, finished, None)

    def frame(self, height):
        return Gdk.pixbuf_get_from_window(
            self.window.get_window(), 0, 0, self.args.width, height
        )

    def pixels(self):
        return hashlib.blake2b(self.frame(53).get_pixels(), digest_size=16).digest()

    def start_benchmark(self):
        self.keyboard = X11Keyboard()
        self.original = self.pixels()
        self.keyboard.press("x")
        GLib.timeout_add(220, self.capture_edit)
        return GLib.SOURCE_REMOVE

    def capture_edit(self):
        self.edited = self.pixels()
        if self.original == self.edited:
            self.fail("The first key press did not paint an edit.")
            return GLib.SOURCE_REMOVE
        self.keyboard.press("BackSpace")
        GLib.timeout_add(220, self.begin_samples)
        return GLib.SOURCE_REMOVE

    def begin_samples(self):
        if self.original != self.pixels():
            self.fail("Backspace did not restore the original pixels.")
            return GLib.SOURCE_REMOVE
        if self.args.screenshot:
            self.frame(420).savev(str(self.args.screenshot), "png", [], [])
        if self.renderer:
            self.renderer.timings.clear()
        self.prepare_sample()
        return GLib.SOURCE_REMOVE

    def prepare_sample(self):
        if self.args.uncached and len(self.timings) < self.args.samples:
            line = DOCUMENT.partition("\n")[0]
            if len(self.timings) % 2 == 0:
                line = "x" + line
            self.evaluate(
                f"window.prototypeEvict({json.dumps(line, ensure_ascii=False)})",
                lambda _removed: self.next_sample(),
            )
        else:
            self.next_sample()
        return GLib.SOURCE_REMOVE

    def next_sample(self):
        if len(self.timings) == self.args.samples:
            self.verify_text()
            return GLib.SOURCE_REMOVE
        add_letter = len(self.timings) % 2 == 0
        self.expected = self.edited if add_letter else self.original
        self.started = time.perf_counter()
        self.keyboard.press("x" if add_letter else "BackSpace")
        GLib.timeout_add(2, self.observe_frame)
        return GLib.SOURCE_REMOVE

    def observe_frame(self):
        painted = self.pixels() == self.expected
        elapsed = (time.perf_counter() - self.started) * 1000
        if painted:
            self.timings.append(elapsed)
            GLib.timeout_add(25, self.prepare_sample)
            return GLib.SOURCE_REMOVE
        if elapsed > 1000:
            if self.args.screenshot:
                path = self.args.screenshot.with_suffix(".failed.png")
                self.frame(420).savev(str(path), "png", [], [])
            message = f"Edit {len(self.timings) + 1} did not reach its expected frame."
            if self.args.mode == "native":
                self.fail(message)
            else:
                self.evaluate("window.prototypeState()", lambda state: self.fail(f"{message} {state}"))
            return GLib.SOURCE_REMOVE
        return GLib.SOURCE_CONTINUE

    def verify_text(self):
        self.outline_timings = self.renderer.timings.copy() if self.renderer else []
        if self.args.mode == "native":
            buffer = self.editor.get_buffer()
            original = buffer.get_text(*buffer.get_bounds(), True)
            self.editor.emit("insert-at-cursor", ARABIC_INSERT)
            inserted = buffer.get_text(*buffer.get_bounds(), True)
            self.finish({}, original == DOCUMENT and inserted == ARABIC_INSERT + DOCUMENT)
        else:
            text = json.dumps(ARABIC_INSERT, ensure_ascii=False)
            script = f"""
                (() => {{
                    const before = window.prototypeState();
                    window.prototypePaste({text});
                    return before;
                }})()
            """
            self.evaluate(script, lambda before: self.verify_web_insert(before, text))
        return GLib.SOURCE_REMOVE

    def verify_web_insert(self, before, inserted):
        def check():
            self.evaluate(
                f"window.prototypeState({inserted})",
                lambda after: self.finish(
                    before,
                    before["documentMatches"]
                    and before["cursor"] == 0
                    and after["documentMatches"],
                ),
            )
            return GLib.SOURCE_REMOVE

        GLib.timeout_add(200, check)

    def finish(self, stats, preserved):
        report = {
            "mode": self.args.mode,
            "width": self.args.width,
            "outline_cache": "cold for the edited line" if self.args.uncached else "normal",
            "document_lines": len(DOCUMENT.splitlines()),
            "arabic_insert_keeps_mark_order": preserved,
        }
        if self.timings:
            report["key_to_expected_frame"] = describe(self.timings)
        if self.renderer:
            report["overlay"] = {
                key: value
                for key, value in stats.items()
                if key not in {"prefix", "cursor", "documentMatches"}
            }
            if self.outline_timings:
                report["native_outline_reply"] = describe(self.outline_timings)
        print(json.dumps(report, ensure_ascii=False), flush=True)
        if not preserved:
            self.fail("The Arabic edit changed text or combining mark order.")
        else:
            Gtk.main_quit()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["native", "overlay", "plain"])
    parser.add_argument("--web", type=Path)
    parser.add_argument("--width", type=int, default=900)
    action = parser.add_mutually_exclusive_group()
    action.add_argument("--bench", action="store_true")
    action.add_argument("--smoke", action="store_true")
    parser.add_argument("--screenshot", type=Path)
    parser.add_argument("--samples", type=int, default=60)
    parser.add_argument("--uncached", action="store_true")
    args = parser.parse_args()
    if args.mode != "native" and (args.web is None or not (args.web / "index.html").is_file()):
        parser.error("Pass the prototype's built HTML directory with --web.")
    if args.uncached and (not args.bench or args.mode != "overlay"):
        parser.error("--uncached applies to the overlay benchmark.")
    if args.samples <= 0 or args.samples % 2:
        parser.error("--samples must be a positive even number.")
    if args.bench and os.environ.get("GDK_BACKEND") != "x11":
        parser.error("Use GDK_BACKEND=x11 for --bench.")
    app = Prototype(args)
    if args.bench or args.smoke:
        GLib.timeout_add_seconds(max(20, args.samples // 4), lambda: app.fail("Prototype test timed out."))
    Gtk.main()
    return int(app.failure is not None)


if __name__ == "__main__":
    sys.exit(main())
