"""Exercise Window.apply without importing GTK or starting a renderer."""
import ast
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock


def apply_method():
    tree = ast.parse((Path(__file__).resolve().parents[1] / "wallforge" / "app.py").read_text())
    window = next(node for node in tree.body if isinstance(node, ast.ClassDef) and node.name == "Window")
    method = next(node for node in window.body if isinstance(node, ast.FunctionDef) and node.name == "apply")
    module = ast.fix_missing_locations(ast.Module(body=[method], type_ignores=[]))
    namespace = {"library": SimpleNamespace(Item=object), "threading": threading}
    exec(compile(module, "app.py", "exec"), namespace)
    return namespace["apply"], namespace


class ApplyTests(unittest.TestCase):
    def setUp(self):
        self.apply, ns = apply_method()
        self.pending = []
        self.notifications = []
        self.cfg = SimpleNamespace(data={"ultrawide_on_16_9": True, "engine": {"preload_next": False}},
                                   update_outputs=mock.Mock())
        self.it = SimpleNamespace(id="1", broken="", ultrawide=False, kind="video")
        self.window = SimpleNamespace(cfg=self.cfg, sel=self.it, _remember=mock.Mock(), _applied=mock.Mock())
        self.engine = SimpleNamespace(use_native=lambda *_: True, start=mock.Mock())
        ns.update(monitors=SimpleNamespace(outputs=lambda: [{"name": "DP-1", "ident": "A"},
                                                             {"name": "DP-2", "ident": "B"}]),
                  engine=self.engine, theme=SimpleNamespace(sync=mock.Mock()),
                  GLib=SimpleNamespace(idle_add=lambda *args: self.notifications.append(args)),
                  threading=SimpleNamespace(Thread=lambda target, daemon: SimpleNamespace(start=lambda: self.pending.append(target))))

    def test_initial_save_runs_in_worker_and_reports_errors(self):
        self.cfg.update_outputs.side_effect = OSError("save failed")
        self.apply(self.window, ["DP-1"], sync=False)
        self.cfg.update_outputs.assert_not_called()
        self.pending[0]()
        self.assertEqual(self.notifications[0][-1], "save failed")

    def test_later_start_failure_saves_successful_outputs(self):
        self.engine.start.side_effect = [None, RuntimeError("start failed")]
        self.apply(self.window, ["DP-1", "DP-2"], sync=False)
        self.pending[0]()
        self.assertEqual(self.cfg.update_outputs.call_args_list[-1].args[0], {"DP-1": {"id": "1", "ident": "A"}})
        self.assertEqual(self.notifications[0][-1], "start failed")
