#!/usr/bin/env python3
"""Exercise the packaged Linux attachment picker under an isolated X11 display.

Usage: linux-picker-probe.py EVIDENCE_DIR -- COMMAND [ARG ...]
The caller supplies DISPLAY, an empty XDG profile, and the executable to test.
"""

import os
import signal
import subprocess
import sys
import time
from pathlib import Path

from Xlib import X, XK, display
from Xlib.error import XError
from Xlib.ext import xtest


def windows(root, title):
    found = []
    for window in root.query_tree().children:
        try:
            if window.get_wm_name() == title and window.get_attributes().map_state == X.IsViewable:
                found.append(window)
        except XError:
            pass
    return found


def describe_windows(root, dpy):
    lines = []
    try:
        focus = dpy.get_input_focus().focus
        focus_id = getattr(focus, "id", None)
        lines.append(f"input_focus={focus_id}")
    except XError as error:
        lines.append(f"input_focus_error={error}")
    for window in root.query_tree().children:
        try:
            attributes = window.get_attributes()
            geometry = window.get_geometry()
            lines.append(
                "window="
                f"id={window.id} title={window.get_wm_name()!r} class={window.get_wm_class()!r} "
                f"map_state={attributes.map_state} geometry="
                f"{geometry.x},{geometry.y},{geometry.width},{geometry.height}"
            )
        except XError as error:
            lines.append(f"window_id={window.id} error={error}")
    return "\n".join(lines) + "\n"


def focus_window(dpy, window):
    # The CI Xvfb display has no window manager; explicitly focus and raise the
    # native chooser before sending keyboard events rather than assuming that
    # its mapping event has already transferred focus.
    window.configure(stack_mode=X.Above)
    window.set_input_focus(X.RevertToParent, X.CurrentTime)
    dpy.sync()
    time.sleep(0.25)


def click(dpy, window, x, y):
    geometry = window.get_geometry()
    # The probe runs on a dedicated Xvfb/Xephyr display without a window
    # manager, so top-level geometry is root-relative.
    screen_x = geometry.x + x
    screen_y = geometry.y + y
    xtest.fake_input(dpy, X.MotionNotify, x=screen_x, y=screen_y)
    xtest.fake_input(dpy, X.ButtonPress, 1)
    xtest.fake_input(dpy, X.ButtonRelease, 1)
    dpy.sync()


def capture(window, path):
    subprocess.run(["xwd", "-silent", "-id", hex(window.id), "-out", str(path)], check=True)


def wait_for_rendered_capture(window, evidence, name, timeout=20):
    scripts = Path(__file__).parent
    capture_path = evidence / f"{name}.xwd"
    png_path = evidence / f"{name}.png"
    analyzer = scripts / "analyze-window-capture.mjs"
    observations = []
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        capture(window, capture_path)
        result = subprocess.run(
            ["node", str(analyzer), str(capture_path), str(png_path)],
            capture_output=True,
            text=True,
            check=False,
        )
        detail = result.stdout.strip() or result.stderr.strip()
        observations.append(f"exit={result.returncode} {detail}")
        if result.returncode == 0:
            (evidence / f"{name}-render-readiness.txt").write_text(
                "\n".join(observations) + "\n"
            )
            return
        if result.returncode != 3:
            break
        time.sleep(0.25)

    (evidence / f"{name}-render-readiness.txt").write_text(
        "\n".join(observations) + "\n"
    )
    raise RuntimeError(
        f"Juniper window did not render within {timeout}s at {name}; "
        f"last analysis: {observations[-1] if observations else 'no capture'}"
    )


def choose_path(dpy, dialog, path, evidence):
    focus_window(dpy, dialog)
    control = dpy.keysym_to_keycode(XK.string_to_keysym("Control_L"))
    letter_l = dpy.keysym_to_keycode(XK.string_to_keysym("l"))
    xtest.fake_input(dpy, X.KeyPress, control)
    xtest.fake_input(dpy, X.KeyPress, letter_l)
    time.sleep(0.05)
    xtest.fake_input(dpy, X.KeyRelease, letter_l)
    xtest.fake_input(dpy, X.KeyRelease, control)
    dpy.sync()
    # GTK's location-entry transition is asynchronous. Give it time to receive
    # focus before typing, then pace key events as a user would.
    time.sleep(0.4)
    for character in str(path):
        symbol = {"/": "slash", "-": "minus", ".": "period"}.get(character, character)
        code = dpy.keysym_to_keycode(XK.string_to_keysym(symbol))
        if not code:
            raise RuntimeError(f"Cannot type picker fixture path character: {character}")
        xtest.fake_input(dpy, X.KeyPress, code)
        time.sleep(0.025)
        xtest.fake_input(dpy, X.KeyRelease, code)
        time.sleep(0.025)
    dpy.sync()
    time.sleep(0.3)
    capture(dialog, evidence / "picker-location-entry.xwd")
    enter = dpy.keysym_to_keycode(XK.string_to_keysym("Return"))
    xtest.fake_input(dpy, X.KeyPress, enter)
    time.sleep(0.05)
    xtest.fake_input(dpy, X.KeyRelease, enter)
    dpy.sync()


def wait_for(predicate, seconds, label, proc=None):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        if proc and proc.poll() is not None:
            raise RuntimeError(f"Juniper exited before {label}: {proc.returncode}")
        time.sleep(0.2)
    raise RuntimeError(f"Timed out waiting for {label}")


def main():
    if len(sys.argv) < 4 or sys.argv[2] != "--":
        raise SystemExit("Usage: linux-picker-probe.py EVIDENCE_DIR -- COMMAND [ARG ...]")
    evidence = Path(sys.argv[1]).resolve()
    evidence.mkdir(parents=True, exist_ok=True)
    fixture = Path("/tmp/juniper-picker-valid.txt")
    fixture.write_text("Juniper picker integration fixture.\n")
    log_path = evidence / "picker.log"
    dpy = display.Display()
    root = dpy.screen().root
    with log_path.open("w") as log:
        proc = subprocess.Popen(sys.argv[3:], stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            wait_for(
                lambda: "[juniper-startup] frontend ready" in log_path.read_text(),
                120,
                "frontend readiness",
                proc,
            )
            app = wait_for(lambda: next(iter(windows(root, "Juniper")), None), 15, "Juniper window", proc)
            geometry = app.get_geometry()
            if geometry.width < 1000 or geometry.height < 800:
                raise RuntimeError("Unexpected Juniper window geometry for the picker probe")
            click(dpy, app, 480, 535)  # fresh-profile onboarding: Skip
            wait_for_rendered_capture(app, evidence, "before-attachment")
            click(dpy, app, 420, 775)  # chat composer: Attach a file
            click(dpy, app, 420, 775)  # rapid repeat must not open another picker
            dialog = wait_for(lambda: next(iter(windows(root, "Open File")), None), 15, "native Open File dialog", proc)
            (evidence / "picker-before-cancel-windows.txt").write_text(describe_windows(root, dpy))
            time.sleep(0.5)
            count = len(windows(root, "Open File"))
            if count != 1:
                raise RuntimeError(f"Expected one native picker, found {count}")
            geometry = dialog.get_geometry()
            click(dpy, dialog, geometry.width - 140, geometry.height - 24)  # Cancel
            wait_for(lambda: not windows(root, "Open File"), 10, "picker cancellation", proc)
            time.sleep(0.5)
            click(dpy, app, 420, 775)
            dialog = wait_for(lambda: next(iter(windows(root, "Open File")), None), 15, "second native dialog", proc)
            (evidence / "picker-before-selection-windows.txt").write_text(describe_windows(root, dpy))
            capture(dialog, evidence / "picker-before-selection.xwd")
            choose_path(dpy, dialog, fixture, evidence)
            try:
                wait_for(lambda: not windows(root, "Open File"), 10, "valid attachment selection", proc)
            except RuntimeError:
                (evidence / "picker-selection-timeout-windows.txt").write_text(describe_windows(root, dpy))
                for index, visible in enumerate(windows(root, "Open File")):
                    capture(visible, evidence / f"picker-selection-timeout-{index}.xwd")
                raise
            time.sleep(0.5)
            wait_for_rendered_capture(app, evidence, "after-valid-selection")
            click(dpy, app, 80, 210)  # Settings: must still respond after cancellation
            wait_for_rendered_capture(app, evidence, "after-cancel-settings")
            scripts = Path(__file__).parent
            for name in ("before-attachment", "after-valid-selection", "after-cancel-settings"):
                with (evidence / f"{name}-render.json").open("w") as output:
                    subprocess.run(
                        [
                            "node",
                            str(scripts / "analyze-window-capture.mjs"),
                            str(evidence / f"{name}.xwd"),
                            str(evidence / f"{name}.png"),
                        ],
                        check=True,
                        stdout=output,
                    )
            with (evidence / "render-difference.json").open("w") as output:
                subprocess.run(
                    [
                        "node",
                        str(scripts / "compare-window-captures.mjs"),
                        str(evidence / "before-attachment.xwd"),
                        str(evidence / "after-cancel-settings.xwd"),
                    ],
                    check=True,
                    stdout=output,
                )
            with (evidence / "selection-difference.json").open("w") as output:
                subprocess.run(
                    [
                        "node",
                        str(scripts / "compare-window-captures.mjs"),
                        str(evidence / "before-attachment.xwd"),
                        str(evidence / "after-valid-selection.xwd"),
                        "--composer",
                    ],
                    check=True,
                    stdout=output,
                )
            if proc.poll() is not None:
                raise RuntimeError(f"Juniper exited after picker cancellation: {proc.returncode}")
            print("PASS: native picker opened, cancelled, staged a text file, and Juniper navigated afterward")
        finally:
            fixture.unlink(missing_ok=True)
            try:
                os.killpg(proc.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(proc.pid, signal.SIGKILL)
                proc.wait(timeout=10)


if __name__ == "__main__":
    main()
