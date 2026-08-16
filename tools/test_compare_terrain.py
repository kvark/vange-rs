#!/usr/bin/env python3
"""Regression tests for the publication camera/reference math."""

import importlib.util
import math
import unittest
from pathlib import Path

import numpy as np


PATH = Path(__file__).with_name("compare-terrain.py")
SPEC = importlib.util.spec_from_file_location("compare_terrain", PATH)
COMPARE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(COMPARE)
COMPARE.np = np


class CameraReferenceTests(unittest.TestCase):
    def test_screen_right_matches_reflected_engine_basis(self):
        rays, forward = COMPARE.camera_rays(8, 6, 0.0, 0.0)
        self.assertLess(rays[3, 7, 0], 0.0)
        mean = rays.mean(axis=(0, 1))
        mean /= np.linalg.norm(mean)
        np.testing.assert_allclose(mean, forward, atol=1e-12)

    def test_far_plane_is_view_axis_distance(self):
        size = 16
        low = np.zeros((size, size), dtype=np.int16)
        layers = (low, np.zeros_like(low, dtype=bool), low, low)
        view = {
            "x": 8, "y": 8, "yaw": 0.0, "under": False,
        }
        sky, distance, rays, _ = COMPARE.ground_truth(
            layers, view, 8, 6, 150.0, 200.0, -90.0)
        self.assertFalse(sky.any())
        forward = rays.mean(axis=(0, 1))
        forward /= np.linalg.norm(forward)
        expected = 150.0 / (rays @ forward)
        np.testing.assert_allclose(distance, expected, atol=0.001)

        near, far = 1.0, 200.0
        view_depth = 150.0
        depth = (far - near * far / view_depth) / (far - near)
        decoded = COMPARE.ray_distance(
            np.full((6, 8), depth), rays, near, far)
        np.testing.assert_allclose(decoded, expected, atol=1e-9)

    def test_yaw_direction(self):
        _, forward = COMPARE.camera_rays(4, 4, 299.0, -30.0)
        yaw = math.radians(299.0)
        pitch = math.radians(-30.0)
        expected = np.array([
            math.sin(yaw) * math.cos(pitch),
            math.cos(yaw) * math.cos(pitch),
            math.sin(pitch),
        ])
        np.testing.assert_allclose(forward, expected, atol=1e-12)


if __name__ == "__main__":
    unittest.main()
