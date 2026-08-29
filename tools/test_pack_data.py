#!/usr/bin/env python3
"""Tests for tools/pack-data.py keep lists and iscreen-into-level packing."""

from __future__ import annotations

import importlib.util
import tempfile
import unittest
import zipfile
from pathlib import Path


PATH = Path(__file__).with_name("pack-data.py")
SPEC = importlib.util.spec_from_file_location("pack_data", PATH)
PACK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACK)


def write(path: Path, text: str = "x") -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


class KeepForWebTests(unittest.TestCase):
    def test_vehicle_metadata_is_kept(self):
        for name in (
            "car.prm",
            "item.prm",
            "price.prm",
            "device.lst",
            "vangers.prm",
            "passages.prm",
            "game.lst",
            "common.prm",
        ):
            self.assertTrue(PACK.keep_for_web(name), name)

    def test_whole_actint_text_tree_is_kept(self):
        for name in (
            "actint/actint.inc",
            "actint/a_str.inc",
            "actint/items.inc",
            "actint/item_prm.inc",
            "actint/mech_prm.inc",
            "actint/aci_iscr.inc",
            "actint/ml_data0.inc",
            "actint/escaves.inc",
        ):
            self.assertTrue(PACK.keep_for_web(name), name)

    def test_menu_art_and_videos_stay_out(self):
        for name in (
            "resource/video/intro.avi",
            "resource/music/track.ogg",
            "resource/actint/hd/big.bmp",
            "resource/actint/iscreen/matrix/m.bmo",
            "resource/iscreen/mainmenu.vmc",
            "vangers.exe",
        ):
            self.assertFalse(PACK.keep_for_web(name), name)

    def test_iscreen_ldata_is_not_common(self):
        self.assertTrue(
            PACK.is_level_iscreen("resource/iscreen/ldata/l0/escave.ini")
        )
        self.assertFalse(PACK.keep_for_web("resource/iscreen/ldata/l0/escave.ini"))
        self.assertFalse(PACK.is_level_iscreen("resource/iscreen/mainmenu.ini"))


class IscreenExtrasTests(unittest.TestCase):
    def test_fostral_gets_podish_and_incubator(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "resource/iscreen/ldata/l0/escave.ini")
            write(root / "resource/iscreen/ldata/l0/escave.vmc")
            write(root / "resource/iscreen/ldata/l0/bitmap/g0.bmp")
            write(root / "resource/iscreen/ldata/l1/escave.ini")
            write(root / "resource/iscreen/ldata/l2/escave.ini")
            names = [arc for _, arc in PACK.iscreen_extras(root, "Fostral")]
        self.assertEqual(
            names,
            [
                "resource/iscreen/ldata/l0/bitmap/g0.bmp",
                "resource/iscreen/ldata/l0/escave.ini",
                "resource/iscreen/ldata/l0/escave.vmc",
                "resource/iscreen/ldata/l1/escave.ini",
            ],
        )

    def test_world_without_caves_is_empty(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write(root / "resource/iscreen/ldata/l0/escave.ini")
            self.assertEqual(PACK.iscreen_extras(root, "khox"), [])

    def test_missing_ldata_is_empty_not_an_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(PACK.iscreen_extras(Path(tmp), "fostral"), [])


class PackRoundtripTests(unittest.TestCase):
    def test_level_zip_keeps_world_files_and_iscreen_keys(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "data"
            out = Path(tmp) / "out" / "fostral.zip"
            out.parent.mkdir()
            level = root / "thechain/fostral"
            write(level / "world.ini", "[Global]\n")
            write(level / "output.vmc", "vmc")
            write(root / "resource/iscreen/ldata/l0/escave.ini", "ini")
            write(root / "resource/iscreen/ldata/l0/escave.pal", "pal")
            write(root / "resource/iscreen/ldata/l1/escave.ini", "ini")
            PACK.pack_level(
                "fostral",
                level,
                out,
                verbose=False,
                extras=PACK.iscreen_extras(root, "fostral"),
            )
            with zipfile.ZipFile(out) as zf:
                names = set(zf.namelist())
        self.assertEqual(
            names,
            {
                "world.ini",
                "output.vmc",
                "resource/iscreen/ldata/l0/escave.ini",
                "resource/iscreen/ldata/l0/escave.pal",
                "resource/iscreen/ldata/l1/escave.ini",
            },
        )

    def test_common_zip_keeps_vehicle_metadata_and_skips_iscreen(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "data"
            out = Path(tmp) / "out" / "common.zip"
            out.parent.mkdir()
            level = root / "thechain/fostral"
            write(level / "world.ini")
            write(root / "car.prm")
            write(root / "item.prm")
            write(root / "actint/items.inc")
            write(root / "actint/mech_prm.inc")
            write(root / "resource/m3d/mechous/m1.prm")
            write(root / "resource/iscreen/ldata/l0/escave.ini")
            write(root / "resource/video/cut.avi", "big")
            PACK.pack_common(
                root,
                {level},
                out,
                full=False,
                verbose=False,
            )
            with zipfile.ZipFile(out) as zf:
                names = set(zf.namelist())
        self.assertEqual(
            names,
            {
                "car.prm",
                "item.prm",
                "actint/items.inc",
                "actint/mech_prm.inc",
                "resource/m3d/mechous/m1.prm",
            },
        )

    def test_full_common_still_leaves_iscreen_to_the_level_zip(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "data"
            out = Path(tmp) / "out" / "common.zip"
            out.parent.mkdir()
            level = root / "thechain/fostral"
            write(level / "world.ini")
            write(root / "resource/iscreen/ldata/l0/escave.ini")
            write(root / "resource/video/cut.avi")
            PACK.pack_common(root, {level}, out, full=True, verbose=False)
            with zipfile.ZipFile(out) as zf:
                names = set(zf.namelist())
        self.assertIn("resource/video/cut.avi", names)
        self.assertNotIn("resource/iscreen/ldata/l0/escave.ini", names)


if __name__ == "__main__":
    unittest.main()
