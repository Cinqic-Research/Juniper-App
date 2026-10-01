#!/usr/bin/env python3
"""Exercise the packaged Linux attachment picker under an isolated X11 display.

Usage: linux-picker-probe.py EVIDENCE_DIR -- COMMAND [ARG ...]
The caller supplies DISPLAY, an empty XDG profile, and the executable to test.
"""

import os
import shutil
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


def scroll_down(dpy, window, x, y, steps):
    geometry = window.get_geometry()
    screen_x = geometry.x + x
    screen_y = geometry.y + y
    for _ in range(steps):
        xtest.fake_input(dpy, X.MotionNotify, x=screen_x, y=screen_y)
        xtest.fake_input(dpy, X.ButtonPress, 5)
        xtest.fake_input(dpy, X.ButtonRelease, 5)
        dpy.sync()
        time.sleep(0.1)


def capture(window, path):
    subprocess.run(["xwd", "-silent", "-id", hex(window.id), "-out", str(path)], check=True)


def wait_for_rendered_capture(window, evidence, name, timeout=20, require_stable=False):
    scripts = Path(__file__).parent
    capture_path = evidence / f"{name}.xwd"
    png_path = evidence / f"{name}.png"
    analyzer = scripts / "analyze-window-capture.mjs"
    comparer = scripts / "compare-window-captures.mjs"
    previous_path = evidence / f"{name}-stability-previous.xwd"
    observations = []
    deadline = time.monotonic() + timeout
    rendered_since = None
    stable_samples = 0
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
            if not require_stable:
                (evidence / f"{name}-render-readiness.txt").write_text(
                    "\n".join(observations) + "\n"
                )
                return capture_path
            now = time.monotonic()
            rendered_since = rendered_since or now
            if now - rendered_since >= 1.5 and previous_path.exists():
                comparison = subprocess.run(
                    ["node", str(comparer), str(previous_path), str(capture_path), "--stable"],
                    capture_output=True,
                    text=True,
                    check=False,
                )
                observations.append(
                    f"stable_exit={comparison.returncode} "
                    f"{comparison.stdout.strip() or comparison.stderr.strip()}"
                )
                stable_samples = stable_samples + 1 if comparison.returncode == 0 else 0
                if stable_samples >= 3:
                    previous_path.unlink(missing_ok=True)
                    (evidence / f"{name}-render-readiness.txt").write_text(
                        "\n".join(observations) + "\n"
                    )
                    return capture_path
            else:
                stable_samples = 0
            shutil.copyfile(capture_path, previous_path)
        elif result.returncode == 3:
            rendered_since = None
            stable_samples = 0
        else:
            break
        time.sleep(0.25)

    previous_path.unlink(missing_ok=True)
    (evidence / f"{name}-render-readiness.txt").write_text(
        "\n".join(observations) + "\n"
    )
    raise RuntimeError(
        f"Juniper window did not reach the required rendered state within {timeout}s at {name}; "
        f"last analysis: {observations[-1] if observations else 'no capture'}"
    )


def wait_for_rendered_change(window, evidence, name, baseline, mode=None, timeout=20):
    scripts = Path(__file__).parent
    capture_path = evidence / f"{name}.xwd"
    png_path = evidence / f"{name}.png"
    analyzer = scripts / "analyze-window-capture.mjs"
    comparer = scripts / "compare-window-captures.mjs"
    observations = []
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        capture(window, capture_path)
        rendered = subprocess.run(
            ["node", str(analyzer), str(capture_path), str(png_path)],
            capture_output=True,
            text=True,
            check=False,
        )
        detail = rendered.stdout.strip() or rendered.stderr.strip()
        observations.append(f"render_exit={rendered.returncode} {detail}")
        if rendered.returncode == 0:
            command = ["node", str(comparer), str(baseline), str(capture_path)]
            if mode:
                command.append(mode)
            comparison = subprocess.run(
                command,
                capture_output=True,
                text=True,
                check=False,
            )
            compare_detail = comparison.stdout.strip() or comparison.stderr.strip()
            observations.append(f"change_exit={comparison.returncode} {compare_detail}")
            if comparison.returncode == 0:
                (evidence / f"{name}-difference.json").write_text(
                    comparison.stdout.strip() + "\n"
                )
                (evidence / f"{name}-readiness.txt").write_text(
                    "\n".join(observations) + "\n"
                )
                return capture_path
            if comparison.returncode != 3:
                break
        elif rendered.returncode != 3:
            break
        time.sleep(0.25)

    (evidence / f"{name}-readiness.txt").write_text("\n".join(observations) + "\n")
    raise RuntimeError(
        f"Juniper did not show the expected UI change within {timeout}s at {name}; "
        f"last observation: {observations[-1] if observations else 'no capture'}"
    )


def choose_path(dpy, dialog, path, evidence, location_capture="picker-location-entry.xwd"):
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
    capture(dialog, evidence / location_capture)
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
    gguf_fixture = Path("/tmp/juniper-picker-valid.gguf")
    gguf_fixture.write_bytes(b"GGUFfixture\n")
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
            baseline = wait_for_rendered_capture(
                app, evidence, "before-attachment", require_stable=True
            )
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
            selected = wait_for_rendered_change(
                app,
                evidence,
                "after-valid-selection",
                baseline,
                "--composer",
            )
            wait_for_rendered_capture(
                app, evidence, "after-valid-selection-stable", require_stable=True
            )
            click(dpy, app, 80, 210)  # Settings: must still respond after cancellation
            wait_for_rendered_change(
                app,
                evidence,
                "after-cancel-settings",
                selected,
            )
            settings = evidence / "after-cancel-settings.xwd"
            click(dpy, app, 390, 286)  # Settings sections: Models & runtime
            wait_for_rendered_change(
                app,
                evidence,
                "models-runtime-settings",
                settings,
            )
            scroll_down(dpy, app, 1000, 750, 12)
            wait_for_rendered_capture(
                app, evidence, "models-runtime-settings-scrolled", require_stable=True
            )
            gguf_baseline = evidence / "models-runtime-settings-scrolled.xwd"
            click(dpy, app, 680, 540)  # Import a GGUF file: Choose .gguf file
            gguf_dialog = wait_for(
                lambda: next(iter(windows(root, "Open File")), None),
                15,
                "native GGUF picker dialog",
                proc,
            )
            (evidence / "gguf-picker-before-cancel-windows.txt").write_text(
                describe_windows(root, dpy)
            )
            if len(windows(root, "Open File")) != 1:
                raise RuntimeError("Expected exactly one native GGUF picker dialog")
            geometry = gguf_dialog.get_geometry()
            click(dpy, gguf_dialog, geometry.width - 140, geometry.height - 24)
            wait_for(lambda: not windows(root, "Open File"), 10, "GGUF picker cancellation", proc)
            click(dpy, app, 680, 540)
            gguf_dialog = wait_for(
                lambda: next(iter(windows(root, "Open File")), None),
                15,
                "second native GGUF picker dialog",
                proc,
            )
            (evidence / "gguf-picker-before-selection-windows.txt").write_text(
                describe_windows(root, dpy)
            )
            choose_path(
                dpy,
                gguf_dialog,
                gguf_fixture,
                evidence,
                "gguf-picker-location-entry.xwd",
            )
            wait_for(
                lambda: not windows(root, "Open File"),
                10,
                "valid GGUF selection",
                proc,
            )
            wait_for_rendered_change(
                app,
                evidence,
                "after-gguf-selection",
                gguf_baseline,
                "--gguf",
            )
            scripts = Path(__file__).parent
            for name in (
                "before-attachment",
                "after-valid-selection",
                "after-valid-selection-stable",
                "after-cancel-settings",
                "models-runtime-settings",
                "models-runtime-settings-scrolled",
                "after-gguf-selection",
            ):
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
            if proc.poll() is not None:
                raise RuntimeError(f"Juniper exited after picker cancellation: {proc.returncode}")
            print(
                "PASS: attachment picker opened, cancelled, and staged text; "
                "Settings responded; GGUF picker opened, cancelled, and accepted "
                "a header-valid fixture (model import not invoked)"
            )
        finally:
            fixture.unlink(missing_ok=True)
            gguf_fixture.unlink(missing_ok=True)
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
