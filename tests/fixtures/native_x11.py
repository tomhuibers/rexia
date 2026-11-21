"""Native focus and tab tests. Run with python3 SCRIPT /path/to/rexia CASE.

Needs Xvfb, libX11, libXtst and a revision-18 Rex on PATH. Uses a private X server
and a disposable Rex server. Set REXIA_TEST_X11_LOG_DIR to retain logs.
"""

import ctypes as C
import json
import os
import re
import socket
import subprocess as sp
import sys
import tempfile
import threading
import time
from contextlib import ExitStack
from pathlib import Path


class WindowAttributes(C.Structure):
    _fields_ = (
        [
            (name, C.c_int)
            for name in ("x", "y", "width", "height", "border_width", "depth")
        ]
        + [("visual", C.c_void_p), ("root", C.c_ulong)]
        + [
            (name, C.c_int)
            for name in ("class_", "bit_gravity", "win_gravity", "backing_store")
        ]
        + [
            ("backing_planes", C.c_ulong),
            ("backing_pixel", C.c_ulong),
            ("save_under", C.c_int),
            ("colormap", C.c_ulong),
            ("map_installed", C.c_int),
            ("map_state", C.c_int),
            ("all_event_masks", C.c_long),
            ("your_event_mask", C.c_long),
            ("do_not_propagate_mask", C.c_long),
            ("override_redirect", C.c_int),
            ("screen", C.c_void_p),
        ]
    )


def library(name):
    return C.CDLL(os.environ.get(f"REXIA_TEST_{name.upper()}", f"lib{name}.so.6"))


def wait(description, read, timeout=8):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if value := read():
            return value
        time.sleep(0.01)
    raise AssertionError(f"timed out waiting for {description}")


class X11:
    def __init__(self, display):
        xlib, xtst = library("X11"), library("Xtst")
        ptr, num, integer = C.c_void_p, C.c_ulong, C.c_int

        def bind(name, result, *args, lib=xlib):
            function = getattr(lib, name)
            function.restype, function.argtypes = result, args
            return function

        self.display = bind("XOpenDisplay", ptr, C.c_char_p)(display.encode())
        assert self.display
        self.root = bind("XDefaultRootWindow", num, ptr)(self.display)
        self.query = bind(
            "XQueryTree",
            integer,
            ptr,
            num,
            C.POINTER(num),
            C.POINTER(num),
            C.POINTER(C.POINTER(num)),
            C.POINTER(C.c_uint),
        )
        self.name = bind("XFetchName", integer, ptr, num, C.POINTER(C.c_char_p))
        self.attributes = bind(
            "XGetWindowAttributes", integer, ptr, num, C.POINTER(WindowAttributes)
        )
        self.free = bind("XFree", integer, ptr)
        self.create = bind(
            "XCreateSimpleWindow",
            num,
            ptr,
            num,
            integer,
            integer,
            C.c_uint,
            C.c_uint,
            C.c_uint,
            num,
            num,
        )
        self.map = bind("XMapWindow", integer, ptr, num)
        self.focus = bind("XSetInputFocus", integer, ptr, num, integer, num)
        self.sync = bind("XSync", integer, ptr, integer)
        self.grab = bind(
            "XGrabButton",
            integer,
            ptr,
            C.c_uint,
            C.c_uint,
            num,
            integer,
            C.c_uint,
            integer,
            integer,
            num,
            num,
        )
        self.ungrab = bind("XUngrabButton", integer, ptr, C.c_uint, C.c_uint, num)
        self.check = bind("XCheckTypedWindowEvent", integer, ptr, num, integer, ptr)
        self.allow = bind("XAllowEvents", integer, ptr, integer, num)
        self.motion = bind(
            "XTestFakeMotionEvent",
            integer,
            ptr,
            integer,
            integer,
            integer,
            num,
            lib=xtst,
        )
        self.button = bind(
            "XTestFakeButtonEvent", integer, ptr, C.c_uint, integer, num, lib=xtst
        )
        self.keycode = bind("XKeysymToKeycode", C.c_ubyte, ptr, num)
        self.key_event = bind(
            "XTestFakeKeyEvent", integer, ptr, C.c_uint, integer, num, lib=xtst
        )
        self.image = bind(
            "XGetImage",
            ptr,
            ptr,
            num,
            integer,
            integer,
            C.c_uint,
            C.c_uint,
            num,
            integer,
        )
        self.pixel = bind("XGetPixel", num, ptr, integer, integer)
        self.destroy_image = bind("XDestroyImage", integer, ptr)

    def window(self):
        root, parent = C.c_ulong(), C.c_ulong()
        children, count = C.POINTER(C.c_ulong)(), C.c_uint()
        self.query(
            self.display,
            self.root,
            C.byref(root),
            C.byref(parent),
            C.byref(children),
            C.byref(count),
        )
        try:
            for index in range(count.value):
                window = children[index]
                name = C.c_char_p()
                if self.name(self.display, window, C.byref(name)):
                    try:
                        attrs = WindowAttributes()
                        if (
                            name.value == b"rexia"
                            and self.attributes(self.display, window, C.byref(attrs))
                            and attrs.map_state == 2
                        ):
                            return window
                    finally:
                        self.free(name)
        finally:
            if children:
                self.free(children)

    def click(self, x, y, activate=None):
        self.motion(self.display, -1, x, y, 0)
        self.sync(self.display, 0)
        if activate is not None:
            # The click-to-focus window manager grabs the press, sets focus, then
            # replays the original button event. Pointer mode is GrabModeSync.
            self.grab(self.display, 1, 1 << 15, activate, 0, 1 << 2, 0, 1, 0, 0)
        self.button(self.display, 1, 1, 0)
        self.sync(self.display, 0)
        if activate is not None:
            event = (C.c_long * 24)()
            wait(
                "the grabbed ButtonPress",
                lambda: self.check(self.display, activate, 4, event),
            )
            self.focus(self.display, activate, 2, 0)
            self.allow(self.display, 2, 0)  # ReplayPointer
            self.ungrab(self.display, 1, 1 << 15, activate)
            self.sync(self.display, 0)
        self.button(self.display, 1, 0, 0)
        self.sync(self.display, 0)

    def drag(self, start, end, y):
        self.motion(self.display, -1, start, y, 0)
        self.button(self.display, 1, 1, 0)
        self.sync(self.display, 0)
        for step in range(1, 11):
            self.motion(self.display, -1, start + (end - start) * step // 10, y, 0)
            self.sync(self.display, 0)
            time.sleep(0.03)
        self.button(self.display, 1, 0, 0)
        self.sync(self.display, 0)

    def row(self, window, width, y):
        frame = self.image(self.display, window, 0, y, width, 1, 0xFFFFFFFF, 2)
        assert frame
        try:
            return [self.pixel(frame, x, 0) & 0xFFFFFF for x in range(width)]
        finally:
            self.destroy_image(frame)

    def snapshot(self, window, path, width, height):
        frame = self.image(self.display, window, 0, 0, width, height, 0xFFFFFFFF, 2)
        assert frame
        try:
            pixels = bytearray()
            for y in range(height):
                for x in range(width):
                    pixels.extend(
                        (self.pixel(frame, x, y) & 0xFFFFFF).to_bytes(3, "big")
                    )
            path.write_bytes(f"P6\n{width} {height}\n255\n".encode() + pixels)
        finally:
            self.destroy_image(frame)

    def sample(self, window):
        # Capture the tab strip and terminal together in one X request.
        frame = self.image(self.display, window, 0, 0, 1260, 500, 0xFFFFFFFF, 2)
        assert frame
        try:
            strip = tuple(self.pixel(frame, x, 10) for x in range(60, 1140, 8))
            body = {
                self.pixel(frame, x, y) & 0xFFFFFF
                for x in range(40, 1000, 40)
                for y in range(100, 450, 24)
            }
            return strip, body
        finally:
            self.destroy_image(frame)

    def key(self, letter, modifiers=()):
        named = {"Escape": 0xFF1B, "Return": 0xFF0D}
        code = self.keycode(
            self.display, named[letter] if letter in named else ord(letter)
        )
        modifier_codes = [self.keycode(self.display, key) for key in modifiers]
        for modifier in modifier_codes:
            self.key_event(self.display, modifier, 1, 0)
        self.key_event(self.display, code, 1, 0)
        self.key_event(self.display, code, 0, 0)
        for modifier in reversed(modifier_codes):
            self.key_event(self.display, modifier, 0, 0)
        self.sync(self.display, 0)


class DataGate:
    def __init__(self, address, destination, retry):
        self.hold = threading.Event()
        self.waiting = threading.Event()
        self.release = threading.Event()
        self.lock = threading.Lock()
        self.opened = 0
        self.attempts = 0
        self.retry = retry
        listener = socket.socket(socket.AF_UNIX)
        listener.bind(str(address))
        listener.listen()

        def accept():
            with listener:
                while True:
                    client, _ = listener.accept()
                    threading.Thread(
                        target=self.relay, args=(client, destination), daemon=True
                    ).start()

        threading.Thread(target=accept, daemon=True).start()

    def relay(self, client, destination):
        def copy(source, target):
            try:
                while data := source.recv(65536):
                    target.sendall(data)
                target.shutdown(socket.SHUT_WR)
            except OSError:
                pass

        with client, socket.socket(socket.AF_UNIX) as server:
            header = b""
            while b"\r\n\r\n" not in header:
                chunk = client.recv(4096)
                if not chunk:
                    return
                header += chunk
            is_data = header.startswith(b"GET /api/data ")
            if is_data and self.hold.is_set():
                with self.lock:
                    self.attempts += 1
                    attempt = self.attempts
                if self.retry and attempt == 1:
                    client.sendall(
                        b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n"
                    )
                    return
                delayed = 3 if self.retry else 1
                if attempt == delayed:
                    self.waiting.set()
                    if not self.release.wait(10):
                        return
            server.connect(str(destination))
            server.sendall(header)
            outgoing = threading.Thread(target=copy, args=(client, server), daemon=True)
            outgoing.start()
            if is_data:
                response = server.recv(65536)
                if not response:
                    return
                client.sendall(response)
                with self.lock:
                    self.opened += 1
            copy(server, client)


def verify_last_session(binary, directory, display, x, launch, rex, server, picker=None):
    """Real Rexia/XTest selection, with a private server and test-only keymap."""
    colors = (0xD92143, 0x16CB52, 0xB552E0)
    ids = []
    for label, rgb in zip(("native-last-A", "native-last-B", "native-last-C"), colors):
        r, g, b = (rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255
        paint = f"import os,time; os.write(1,b'\\x1b[48;2;{r};{g};{b}m\\x1b[2J\\x1b[H'); time.sleep(120)"
        args = ["new", label, "--shell=none"]
        if picker == "t" and label.endswith("-C"):
            args += ["--cwd", str(directory / label)]
        ids.append(rex(*args, "--", sys.executable, "-c", paint)["session_id"])
    settings = directory / "settings.toml"
    settings.write_text('[interface]\nsidebar = false\npane_headers = "hidden"\n')
    env = dict(os.environ, DISPLAY=display, XDG_CONFIG_HOME=str(directory / "config"))
    env.pop("WAYLAND_DISPLAY", None)
    launch("rexia", [binary, "--config", str(settings), "--server", f"unix://{server}",
                     "--session", ids[0]], env=env, stdout=sp.DEVNULL)
    window = wait("Rexia native window", x.window)
    x.focus(x.display, window, 2, 0)
    x.sync(x.display, 0)

    def shown(index):
        return colors[index] in x.sample(window)[1]

    def expect(index, label):
        wait(label, lambda: shown(index), timeout=20)
        # Repeated frames must show the destination rather than an overlay/old pane.
        for _ in range(3):
            assert shown(index), f"{label}: destination not stable"
            time.sleep(0.03)

    def choose(label):
        x.key("k", modifiers=(0xFFE3, 0xFFE1))  # native Ctrl+Shift+K switcher
        for letter in label:
            x.key(letter)
        x.key("Return")

    def last():
        x.key("a", modifiers=(0xFFE3,))
        x.key("l")

    expect(0, "initial A")
    choose("native-last-B")
    expect(1, "native picker B")
    last(); expect(0, "B -> A by prefix+l")
    last(); expect(1, "A -> B by prefix+l")
    def open_picker(source, key):
        before = len(rex("block", "ls", "-s", source)["blocks"])
        x.key("a", modifiers=(0xFFE3,))
        x.key(key)
        wait("real fzf picker block", lambda: len(rex("block", "ls", "-s", source)["blocks"]) > before)
        time.sleep(0.25)
        return before

    def fzf(source, key, query):
        open_picker(source, key)
        for letter in query:
            x.key(letter)
        x.key("Return")

    if picker in ("f", "t"):
        fzf(ids[1], picker, "native-last-C")
    else:
        choose("native-last-C")
    expect(2, "picker existing C")
    last(); expect(1, "C -> B by prefix+l")
    last(); expect(2, "B -> C by prefix+l")
    if picker in ("f", "t"):
        before = open_picker(ids[2], picker)
        x.key("Escape")
        wait("cancelled picker removed", lambda: len(rex("block", "ls", "-s", ids[2])["blocks"]) == before, timeout=20)
        expect(2, "picker cancellation keeps C")
        last(); expect(1, "cancel did not alter last B")
        last(); expect(2, "cancelled picker still toggles C")
    if picker == "family":
        def family(key):
            x.key("a", modifiers=(0xFFE3,))
            x.key(key)

        family("k")
        wait("k created agent session", lambda: any(s["label"] == "native-last-C-agents" for s in rex("ls")["sessions"]), timeout=20)
        wait("k selected agent client-side", lambda: not shown(2), timeout=20)
        last(); expect(2, "new k destination -> C by prefix+l")
        last(); wait("C -> existing agent by prefix+l", lambda: not shown(2), timeout=20)
        family("s")
        wait("s created servers session", lambda: any(s["label"] == "native-last-C-servers" for s in rex("ls")["sessions"]), timeout=20)
        family("j"); expect(2, "j reused root C")
        last(); wait("j -> prior server by prefix+l", lambda: not shown(2), timeout=20)
    if picker == "t":
        fzf(ids[2], "t", "native-last-D")
        wait("project picker created D", lambda: any(s["label"] == "native-last-D" for s in rex("ls")["sessions"]), timeout=20)
        wait("new D replaces C", lambda: not shown(2) and len(rex("block", "ls", "-s", ids[2])["blocks"]) == 1, timeout=20)
        time.sleep(0.3)
        last(); expect(2, "new D -> C by prefix+l")
        last(); wait("C -> new D by prefix+l", lambda: not shown(2), timeout=20)
    print(f"Native X11: A/B toggle, {picker or 'native'} navigation to existing C, B/C toggle via prefix+l", flush=True)


def run(binary, case, directory):
    with ExitStack() as stack:

        def launch(name, args, **kwargs):
            log = stack.enter_context((directory / f"{name}.log").open("wb"))
            process = sp.Popen(args, stderr=log, **kwargs)
            stack.callback(process.wait, timeout=5)
            stack.callback(process.terminate)
            return process

        xvfb = launch(
            "xvfb",
            ["Xvfb", "-displayfd", "1", "-screen", "0", "1440x900x24"],
            stdout=sp.PIPE,
            text=True,
        )
        display = f":{xvfb.stdout.readline().strip()}"
        x = X11(display)
        server = directory / "rex.sock"
        keymap = directory / "init.lua"
        if case in ("last-f", "last-t", "last-family"):
            package = os.environ["REXIA_TEST_REXCONFIG"]
            keymap.write_text(f'dofile({json.dumps(package + "/share/rexconfig/init.lua")})\n')
        else:
            keymap.write_text('rex.bind("ctrl+a>l", "session.last")\n')
        server_env = dict(os.environ)
        if case == "last-t":
            server_env["_ZO_DATA_DIR"] = str(directory / "zoxide")
        launch(
            "rex",
            [
                "rex",
                "server",
                "run",
                "--listen",
                f"unix://{server}",
                "--state-dir",
                str(directory / "state"),
                "--restore=false",
                "--config",
                str(keymap) if case in ("last-session", "last-f", "last-t", "last-family") else "/dev/null",
            ],
            stdout=sp.DEVNULL,
            env=server_env,
        )
        wait("Rex listening", server.exists)

        def rex(*args, as_json=True):
            args = list(args)
            if as_json:
                args.insert(args.index("--") if "--" in args else len(args), "--json")
            result = sp.check_output(
                ["rex", "--autostart=false", "-S", f"unix://{server}", *args]
            )
            return json.loads(result) if as_json else result

        if case in ("last-session", "last-f", "last-t", "last-family"):
            if case == "last-t":
                zoxide = os.environ["REXIA_TEST_ZOXIDE"]
                zoenv = dict(os.environ, _ZO_DATA_DIR=str(directory / "zoxide"))
                for name in ("native-last-C", "native-last-D"):
                    project = directory / name
                    project.mkdir()
                    sp.check_call([zoxide, "add", str(project)], env=zoenv)
            verify_last_session(binary, directory, display, x, launch, rex, server,
                                picker=case[-1] if case in ("last-f", "last-t") else
                                "family" if case == "last-family" else None)
            return

        if case == "floating-resize":
            verify_floating_resize(binary, directory, display, x, launch, rex, server)
            return

        old_color, cold_color, split_color = 0xD92143, 0x16CB52, 0x1379DE
        other_color = 0xB552E0
        closing = case in ("close", "close-select")
        cross_session = case == "cross-session"
        destination_label = "coldsplit"
        clicks = directory / "clicks"
        clicks.write_bytes(b"")
        probe = (
            "import os,sys,tty; tty.setraw(0); "
            "os.write(1, b'\\x1b[48;2;217;33;67m\\x1b[2J\\x1b[H\\x1b[?1000h\\x1b[?1006h'); "
            "log=open(sys.argv[1],'ab',buffering=0)\n"
            "while data:=os.read(0,1024): log.write(data)\n"
        )
        created = rex(
            "new",
            "native-first-frame",
            "--shell=none",
            "--",
            sys.executable,
            "-c",
            probe,
            str(clicks),
        )
        session = created["session_id"]
        old_window = created["initial_windows"][0]["window_id"]
        if cross_session:
            color = b"\x1b[48;2;22;203;82m\x1b[2J\x1b[H"
            script = f"import os,time; os.write(1,{color!r}); time.sleep(60)"
            destination = rex(
                "new",
                destination_label,
                "--shell=none",
                "--",
                sys.executable,
                "-c",
                script,
            )
            split_session = destination["session_id"]
            cold = destination["initial_windows"][0]
        elif case != "focus":
            split_session = session
            tabs = []
            for index in range(5):
                rgb = "181;82;224" if index == 0 else "22;203;82"
                color = f"\x1b[48;2;{rgb}m\x1b[2J\x1b[H".encode()
                script = f"import os,time; os.write(1,{color!r}); time.sleep(60)"
                tabs.append(
                    rex(
                        "window",
                        "new",
                        "-s",
                        session,
                        "--focus=false",
                        "--shell=none",
                        f"tab-{index}",
                        "--",
                        sys.executable,
                        "-c",
                        script,
                    )
                )
            cold = tabs[-1]
            # Put the old tab in the middle. The two nearest tabs on each side
            # prewarm, leaving just the last tab's two streams unopened.
            rex(
                "api",
                "call",
                "-s",
                session,
                "session.move_window",
                json.dumps(
                    {
                        "window_id": old_window,
                        "before_window_id": tabs[2]["window_id"],
                    }
                ),
                as_json=False,
            )
        if case != "focus":
            color = b"\x1b[48;2;19;121;222m\x1b[2J\x1b[H"
            script = f"import os,time; os.write(1,{color!r}); time.sleep(60)"
            rex(
                "split",
                "-s",
                split_session,
                "-b",
                cold["block_ids"][0],
                "--split=right",
                "--focus=false",
                "--shell=none",
                "--",
                sys.executable,
                "-c",
                script,
            )
            gate = DataGate(directory / "gate.sock", server, retry=case == "retry")
        config = directory / "settings.toml"
        config.write_text(
            '[interface]\nsidebar = false\npane_headers = "hidden"\ntabs_fill = true\n'
        )
        env = dict(
            os.environ, DISPLAY=display, XDG_CONFIG_HOME=str(directory / "config")
        )
        env.pop("WAYLAND_DISPLAY", None)
        address = server if case == "focus" else directory / "gate.sock"
        launch(
            "rexia",
            [
                binary,
                "--config",
                str(config),
                "--server",
                f"unix://{address}",
                "--session",
                session,
            ],
            env=env,
            stdout=sp.DEVNULL,
        )
        window = wait("the Rexia X11 window", x.window)
        other = x.create(x.display, x.root, 1, 1, 50, 50, 0, 0, 0)
        x.map(x.display, other)
        x.focus(x.display, window, 2, 0)
        x.sync(x.display, 0)
        initial = wait(
            "the initial terminal frame",
            lambda: sample if old_color in (sample := x.sample(window))[1] else None,
        )
        if case == "focus":
            # Only the window deactivates. Its terminal still owns internal focus.
            x.focus(x.display, other, 2, 0)
            x.sync(x.display, 0)
            x.click(450, 260, activate=window)
            x.key("a")
            x.click(450, 260)
            x.key("b")
            wait(
                "the keyboard marker after both clicks",
                lambda: b"b" in clicks.read_bytes(),
            )
            activation, ordinary = clicks.read_bytes().split(b"a", 1)
            press = rb"\x1b\[<0;[0-9]+;[0-9]+M"
            counts = tuple(
                len(re.findall(press, events)) for events in (activation, ordinary)
            )
            print(
                f"X11 terminal presses: {counts[0]} activation, {counts[1]} ordinary",
                flush=True,
            )
            assert counts == (0, 1), clicks.read_bytes()
            assert activation == b"", "activation also suppressed the button release"
            # A focus change without a held button must not eat the next real click.
            x.focus(x.display, other, 2, 0)
            x.focus(x.display, window, 2, 0)
            x.sync(x.display, 0)
            x.click(450, 260)
            x.key("c")
            wait(
                "the marker after programmatic focus",
                lambda: b"c" in clicks.read_bytes(),
            )
            after_focus = clicks.read_bytes().split(b"b", 1)[1]
            assert len(re.findall(press, after_focus)) == 1, after_focus
            print("X11: programmatic activation did not eat the next click", flush=True)
        elif cross_session:
            wait(
                "the original terminal accepting keyboard input",
                lambda: send_marker(x, clicks),
            )
            assert gate.opened == 1, "destination streams were not cold"
            gate.hold.set()
            x.key("k", modifiers=(0xFFE3, 0xFFE1))  # Ctrl_L, Shift_L
            for letter in destination_label:
                x.key(letter)
            x.key("Return")
            wait(
                "one destination stream held and the other opened",
                lambda: gate.waiting.is_set() and gate.opened == 2,
            )
            wait(
                "the loading area replacing the original terminal",
                lambda: old_color not in x.sample(window)[1],
            )
            loading = assert_loading(x, window, {old_color, cold_color, split_color})
            verify_first_layout(x, window, gate, loading, {cold_color, split_color})
            print(
                "Cross-session jump: loading card held for 1.25s; first observed destination layout contained both terminals",
                flush=True,
            )
        else:
            wait("the old tab and four prewarmed neighbors", lambda: gate.opened == 5)
            gate.hold.set()
            if closing:
                rex(
                    "api",
                    "call",
                    "-s",
                    session,
                    "session.move_window",
                    json.dumps(
                        {
                            "window_id": cold["window_id"],
                            "before_window_id": tabs[2]["window_id"],
                        }
                    ),
                    as_json=False,
                )
                rex("window", "close", "-s", session, old_window, as_json=False)
                assert (
                    rex("session", "inspect", session)["active_window_id"]
                    == cold["window_id"]
                )
                verify_closed_tab(
                    x,
                    window,
                    gate,
                    old_color,
                    cold_color,
                    split_color,
                    other_color,
                    case,
                    lambda: rex("session", "inspect", session)["active_window_id"],
                    tabs[0]["window_id"],
                )
                return
            rex("focus", "-s", session, cold["window_id"], as_json=False)
            wait(
                "one cold stream held and the other opened",
                lambda: gate.waiting.is_set() and gate.opened == 6,
            )
            delay = 0.12 if case == "tabs" else 1.25
            start = time.monotonic()
            while time.monotonic() - start < delay:
                strip, colors = x.sample(window)
                assert (
                    strip == initial[0]
                    and old_color in colors
                    and not {cold_color, split_color} & colors
                ), (
                    f"cold layout drew with one stream still held ({time.monotonic() - start:.3f}s)"
                )
                time.sleep(0.01)

            def new_layout():
                sample = x.sample(window)
                return sample if sample[0] != initial[0] else None

            if case == "cancel":
                x.key("Escape")
                wait(
                    "Escape returning focus to the old tab",
                    lambda: (
                        rex("session", "inspect", session)["active_window_id"]
                        == old_window
                    ),
                )
                gate.release.set()
                wait(
                    "the old terminal getting keyboard focus back",
                    lambda: send_marker(x, clicks),
                )
                wait("the abandoned stream still attaching", lambda: gate.opened == 7)
                until = time.monotonic() + 0.2
                while time.monotonic() < until:
                    assert not new_layout(), "late data revived the canceled switch"
                    time.sleep(0.01)
                print(
                    "Escape canceled the switch, restored terminal input, and late data did not switch tabs",
                    flush=True,
                )
            else:
                gate.release.set()
                first = wait("the new selected tab in a native frame", new_layout)
                assert {cold_color, split_color} <= first[1] and old_color not in first[
                    1
                ], "first new tab frame did not contain both cold terminals"
                print(
                    f"Cold tab ({case}): held one stream for {delay:.2f}s; first observed new layout contained both terminals",
                    flush=True,
                )


def verify_floating_resize(binary, directory, display, x, launch, rex, server):
    created = rex("new", "floating-resize", "--shell=none", "--", "cat")
    session = created["session_id"]
    left, right = 0xD92143, 0x1379DE

    def pane(label, color):
        rgb = ";".join(str(color >> shift & 255) for shift in (16, 8, 0))
        fill = f"\x1b[48;2;{rgb}m\x1b[2J\x1b[H\x1b[?25l".encode()
        script = (
            "import os, signal, time\n"
            f"def paint(*_): os.write(1, {fill!r})\n"
            "signal.signal(signal.SIGWINCH, paint)\npaint()\ntime.sleep(60)\n"
        )
        return {
            "block": {
                "label": label,
                "flavor": "com.superlogical.terminal.shell",
                "options": {"shell": "none", "command": [sys.executable, "-c", script]},
            }
        }

    rex(
        "api",
        "call",
        "-s",
        session,
        "session.new_layer",
        json.dumps(
            {
                "session_id": session,
                "bounds": [0.2, 0.2, 0.6, 0.6],
                "focus": True,
                "layout": {
                    "split": {
                        "direction": "horizontal",
                        "ratio": 0.5,
                        "before": pane("Left", left),
                        "after": pane("Right", right),
                    }
                },
            }
        ),
        as_json=False,
    )
    config = directory / "settings.toml"
    config.write_text(
        '[interface]\nsidebar = false\nanimation = "off"\npane_style = "comfortable"\n'
    )
    env = dict(os.environ, DISPLAY=display, XDG_CONFIG_HOME=str(directory / "config"))
    env.pop("WAYLAND_DISPLAY", None)
    launch(
        "rexia",
        [
            binary,
            "--config",
            str(config),
            "--server",
            f"unix://{server}",
            "--session",
            session,
        ],
        env=env,
        stdout=sp.DEVNULL,
    )
    window = wait("the Rexia X11 window", x.window)
    attrs = WindowAttributes()
    assert x.attributes(x.display, window, C.byref(attrs))
    x.focus(x.display, window, 2, 0)
    x.sync(x.display, 0)

    def spans():
        row = x.row(window, attrs.width, attrs.height // 2)
        if left not in row or right not in row:
            return None
        return [
            (row.index(color), len(row) - row[::-1].index(color))
            for color in (left, right)
        ]

    def tab():
        return rex("session", "inspect", session)["windows"][0]

    def split():
        layer = next(layer for layer in tab()["layers"] if layer["kind"] != "tiled")
        assert layer["bounds"] == [0.2, 0.2, 0.6, 0.6], "the drag moved the whole layer"
        return layer["layout"]["split"]

    def divider(stretches):
        # The terminal origin is consistently inset; its right edge is rounded to whole cells.
        return attrs.x + stretches[1][0] - inset

    initial = wait("both floating terminals", spans)
    inset = initial[0][0] - round(attrs.width * 0.2)
    y = attrs.y + attrs.height // 2
    edge = attrs.x + initial[-1][1]
    x.snapshot(window, directory / "before-drag.ppm", attrs.width, attrs.height // 2)
    print(
        f"X11 drag from ({divider(initial)}, {y}) in window at ({attrs.x}, {attrs.y})",
        flush=True,
    )
    x.drag(divider(initial), edge - 35, y)
    first_ratio = split()["ratio"]
    print(f"X11 floating resize: saved ratio={first_ratio:.4f}", flush=True)
    assert 0.60 < first_ratio < 0.80, (
        "floating divider did not move to the header width floor"
    )
    first = wait(
        "both resized terminal frames",
        lambda: (
            current
            if (current := spans()) and current[0][1] > initial[0][1] + 80
            else None
        ),
    )
    right_width = round(attrs.width * 0.8) + attrs.x - divider(first)
    print(f"X11 right pane stopped at {right_width}px (with gutters)", flush=True)
    assert 210 <= right_width <= 218
    x.snapshot(window, directory / "right-limit.ppm", attrs.width, attrs.height // 2)

    x.drag(divider(first), edge - 5, y)
    assert abs(split()["ratio"] - first_ratio) < 0.001, (
        "dragging farther shrank the header"
    )
    assert divider(spans()) == divider(first), "the divider drifted past the floor"

    x.drag(divider(first), attrs.x + initial[0][0] + 5, y)
    last_ratio = split()["ratio"]
    assert 0.20 < last_ratio < 0.40
    last = wait(
        "both terminal frames after reversing the drag",
        lambda: (
            current
            if (current := spans()) and current[1][0] < initial[1][0] - 80
            else None
        ),
    )
    left_width = divider(last) - attrs.x - round(attrs.width * 0.2)
    assert 210 <= left_width <= 218
    print(
        f"X11 left pane stopped at {left_width}px (with gutters), saved ratio={last_ratio:.4f}",
        flush=True,
    )
    x.snapshot(window, directory / "left-limit.ppm", attrs.width, attrs.height // 2)

    left_block = split()["before"]["block_id"]
    for direction, amount, expected in (
        ("right", 0.08, last_ratio + 0.08),
        ("left", 0.5, last_ratio),
    ):
        rex(
            "do",
            "-s",
            session,
            "-b",
            left_block,
            "pane.resize",
            f"direction={direction}",
            f"amount={amount}",
        )
        wait(
            "Resize Pane's saved ratio",
            lambda: abs(split()["ratio"] - expected) < 0.001,
        )
    print("Resize Pane moved away from the floor and stopped at it again", flush=True)
    wait(
        "the divider back at the left floor",
        lambda: (current := spans()) and divider(current) == divider(last),
    )

    # Zoom is the middle of the five inline buttons, 80 px left of the split line.
    x.click(divider(last) - 80, attrs.y + 217)
    wait(
        "the narrow header's inline Zoom button",
        lambda: tab().get("zoomed_block_id") == left_block,
    )
    print("X11: narrow pane's inline Zoom button still works", flush=True)


def send_marker(x, log):
    x.key("d")
    return b"d" in log.read_bytes()


def assert_loading(x, window, terminal_colors):
    start = time.monotonic()
    while time.monotonic() - start < 1.25:
        loading = x.sample(window)
        _, colors = loading
        assert not terminal_colors & colors, (
            "a terminal appeared while the other stream was held"
        )
        assert len(colors) > 1, (
            "the waiting area was blank instead of showing a loading card"
        )
        time.sleep(0.01)
    return loading


def verify_first_layout(x, window, gate, loading, terminal_colors):
    gate.release.set()

    def new_layout():
        sample = x.sample(window)
        return sample if sample != loading else None

    first = wait("the destination selected in a native frame", new_layout)
    assert terminal_colors <= first[1], (
        "first observed destination layout lacked a terminal"
    )


def verify_closed_tab(
    x, window, gate, old, cold, split, other, case, active_tab, other_window
):
    wait(
        "one successor stream held and one opened",
        lambda: gate.waiting.is_set() and gate.opened == 6,
    )
    wait("the closed terminal disappearing", lambda: old not in x.sample(window)[1])
    loading = assert_loading(x, window, {old, cold, split, other})
    if case == "close-select":
        x.key("1", modifiers=(0xFFE9,))  # Alt_L
        wait(
            "Alt+1 selecting the first tab on Rex", lambda: active_tab() == other_window
        )
        wait(
            "another tab selected while the successor is still held",
            lambda: other in x.sample(window)[1],
        )
        gate.release.set()
        wait("the abandoned successor stream opening", lambda: gate.opened == 7)
        until = time.monotonic() + 0.2
        while time.monotonic() < until:
            _, colors = x.sample(window)
            assert other in colors and not {cold, split} & colors, (
                "late data reopened the successor"
            )
            time.sleep(0.01)
        print(
            "Closed tab: Alt+1 selected another tab; late data did not reopen the successor",
            flush=True,
        )
        return

    verify_first_layout(x, window, gate, loading, {cold, split})
    print(
        "Closed tab: separate loading state for 1.25s; first observed successor layout contained both terminals",
        flush=True,
    )


if __name__ == "__main__":
    case = sys.argv[2]
    assert case in (
        "focus",
        "tabs",
        "slow",
        "retry",
        "cancel",
        "close",
        "close-select",
        "cross-session",
        "last-session",
        "last-f",
        "last-t",
        "last-family",
        "floating-resize",
    )
    parent = os.environ.get("REXIA_TEST_X11_LOG_DIR")
    if parent:
        directory = Path(tempfile.mkdtemp(prefix=f"x11-{case}-", dir=parent))
        print(f"Evidence: {directory}", flush=True)
        run(sys.argv[1], case, directory)
    else:
        with tempfile.TemporaryDirectory(prefix=f"rexia-x11-{case}-") as directory:
            run(sys.argv[1], case, Path(directory))
