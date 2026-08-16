#!/usr/bin/env python3
"""Regression tests for supplement/result composition."""

import json
import pathlib
import subprocess
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "tools" / "upgrade-paper-results.py"


def method(label):
    return {"label": label, "terrain": label, "args": [], "warmup_frames": 3}


def fixture(purpose, labels, rows, supplement=False):
    value = {
        "protocol_version": 3,
        "purpose": "publication-supplement" if supplement else purpose,
        "label": "GPU",
        "source": {"revision": "abc", "dirty": False},
        "level": "fostral",
        "device": {"adapter": "GPU", "backend": "Vulkan"},
        "width": 10,
        "height": 8,
        "far": 600.0,
        "frames": 40,
        "shadows": "on",
        "lighting": "unbaked diffuse",
        "scenes": [{"name": "one"}],
        "methods": [method(label) for label in labels],
        "rows": [{"method": label, "view": "one", "pitch": 0,
                  "marker": marker} for label, marker in rows],
        "edit_protocol": {"fixture": 1} if supplement else None,
        "edit_rows": [{"method": "Painted"}] if supplement else [],
    }
    if supplement:
        value["publication_methods"] = [
            method(label) for label in
            ("RayTraced", "RayVoxel", "Sliced", "Scattered", "Painted", "Mesh q=0.5")
        ]
    return value


class UpgradeTests(unittest.TestCase):
    def run_upgrade(self, base, supplement):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = pathlib.Path(directory.name)
        paths = [root / name for name in ("base.json", "supplement.json", "out.json")]
        paths[0].write_text(json.dumps(base))
        paths[1].write_text(json.dumps(supplement))
        result = subprocess.run(
            [sys.executable, SCRIPT, paths[0], paths[1], "--out", paths[2]],
            text=True, capture_output=True,
        )
        return result, paths[2]

    def test_reuses_only_unchanged_rows(self):
        base_labels = ["RayTraced", "RayVoxel", "Sliced", "Scattered", "Painted",
                       "Mesh q=0.0", "Mesh q=0.75"]
        base = fixture("publication", base_labels,
                       [(label, "old") for label in base_labels])
        changed = ["RayTraced", "RayVoxel", "Mesh q=0.5"]
        supplement = fixture("publication", changed,
                             [(label, "new") for label in changed], True)
        result, output = self.run_upgrade(base, supplement)
        self.assertEqual(result.returncode, 0, result.stderr)
        upgraded = json.loads(output.read_text())
        markers = {row["method"]: row["marker"] for row in upgraded["rows"]}
        self.assertEqual(markers, {
            "RayTraced": "new", "RayVoxel": "new", "Sliced": "old",
            "Scattered": "old", "Painted": "old", "Mesh q=0.5": "new",
        })
        self.assertFalse(upgraded["accuracy_valid"])
        self.assertEqual(upgraded["edit_rows"], [{"method": "Painted"}])

    def test_rejects_different_device(self):
        labels = ["Sliced", "Scattered", "Painted"]
        base = fixture("publication", labels, [(label, "old") for label in labels])
        changed = ["RayTraced", "RayVoxel", "Mesh q=0.5"]
        supplement = fixture("publication", changed,
                             [(label, "new") for label in changed], True)
        supplement["device"]["adapter"] = "another GPU"
        result, _ = self.run_upgrade(base, supplement)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("device", result.stderr)


if __name__ == "__main__":
    unittest.main()
