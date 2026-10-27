"""Deterministic validation of report math, missing evidence and phase boundaries."""
import importlib.util
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('ui_performance', Path(__file__).with_name('ui-performance.py'))
performance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(performance)


class PerformanceEvidenceTests(unittest.TestCase):
    def test_empty_metrics_are_missing_instead_of_zero(self):
        self.assertEqual(performance.quantiles([]), dict(p50=None, p95=None, max=None, samples=0))
        self.assertEqual(performance.input_delta([], [])['samples'], 0)
        self.assertIsNone(performance.input_delta([], [])['p95'])

    def test_phase_histogram_excludes_previous_inputs(self):
        result = performance.input_delta([(1000000, 20)], [(1000000, 20), (18000000, 19), (40000000, 1)])
        self.assertEqual(result, dict(p50=18, p95=18, max=40, samples=20))

    def test_histogram_reset_refuses_a_comparison(self):
        with self.assertRaises(ValueError):
            performance.input_delta([(1000000, 20)], [(1000000, 1)])
        with self.assertRaises(ValueError):
            performance.input_delta([(1000000, 20)], [(2000000, 1)])

    def test_descendants_exclude_unrelated_tool_hosts(self):
        table = {10: (1, 'app'), 11: (10, 'audio helper'), 12: (11, 'ffmpeg'), 13: (1, 'collector')}
        self.assertEqual(performance.descendants(table, 10), {10, 11, 12})

    def test_quantiles_use_samples_and_preserve_outliers(self):
        result = performance.quantiles([None, float('nan'), *range(1, 101)])
        self.assertEqual(result, dict(p50=50, p95=95, max=100, samples=100))


if __name__ == '__main__':
    unittest.main()
