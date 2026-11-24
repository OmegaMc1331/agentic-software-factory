import copy
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import v2_baseline as baseline


class InventoryTests(unittest.TestCase):
    def test_fixture_hash_is_stable_across_git_checkout_line_endings(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "fixture.sql"
            with patch.object(baseline, "FIXTURE", path):
                path.write_bytes(b"BEGIN;\nCOMMIT;\n")
                unix_hash = baseline.fixture_digest()
                path.write_bytes(b"BEGIN;\r\nCOMMIT;\r\n")
                self.assertEqual(baseline.fixture_digest(), unix_hash)

    def test_multiline_chained_routes_and_terminal_are_inventoried(self):
        routes = baseline.route_inventory('''
            Router::new()
                .route("/api/runs", get(list_runs).post(create_run))
                .route(
                    "/api/sessions/:id/terminal",
                    get(interactive_terminal),
                )
                .route("/api/config", get(config).put(save))
                .fallback_service(assets())
        ''')
        self.assertEqual(routes, [
            {"path": "/api/config", "methods": ["GET", "PUT"]},
            {"path": "/api/runs", "methods": ["GET", "POST"]},
            {"path": "/api/sessions/:id/terminal", "methods": ["GET"]},
        ])

    def test_empty_or_incomplete_routing_fails(self):
        for source in ("Router::new()", '.route("/broken", get(handler)', '.route("/unknown", custom(handler))'):
            with self.subTest(source=source), self.assertRaises(ValueError):
                baseline.route_inventory(source)

    def test_compatible_additions_do_not_rewrite_the_baseline(self):
        reference = json.loads(baseline.INVENTORY.read_text(encoding="utf-8"))
        current = copy.deepcopy(reference)
        current["routes"].append({"path": "/api/v2/projects", "methods": ["GET", "POST"]})
        current["migration_constants"].append({"version": 19, "name": "V19_SCHEMA", "source_sha256": "new"})
        self.assertEqual(baseline.compatibility_errors(reference, current), [])

    def test_removed_route_rewritten_migration_and_fixture_are_rejected(self):
        reference = json.loads(baseline.INVENTORY.read_text(encoding="utf-8"))
        current = copy.deepcopy(reference)
        current["routes"] = [r for r in current["routes"] if r["path"] != "/api/runs"]
        current["migration_constants"][0]["source_sha256"] = "rewritten"
        current["fixture_sha256"] = "changed"
        errors = baseline.compatibility_errors(reference, current)
        self.assertEqual(len(errors), 3)
        self.assertTrue(any("/api/runs" in error for error in errors))

    def test_frozen_fixture_has_all_tables_populated_and_valid(self):
        inventory = baseline.database_inventory(baseline.FIXTURE.read_text(encoding="utf-8"))
        self.assertEqual(inventory["migrations"], list(range(1, 19)))
        self.assertEqual(len(inventory["tables"]), 16)
        self.assertTrue(all(table["fixture_rows"] > 0 for table in inventory["tables"]))

    def test_current_routes_and_migrations_preserve_v1(self):
        reference = json.loads(baseline.INVENTORY.read_text(encoding="utf-8"))
        self.assertEqual(baseline.compatibility_errors(reference, baseline.current_inventory()), [])


class MeasurementTests(unittest.TestCase):
    def test_nearest_rank_p95_and_median_preserve_raw_measurements(self):
        samples = list(range(1, 21))
        result = baseline.summarize(samples)
        self.assertEqual(result["median_ms"], 10.5)
        self.assertEqual(result["p95_ms"], 19)
        self.assertEqual(result["samples_ms"], samples)

    def test_invalid_measurements_are_not_reported_as_success(self):
        for samples in ([], [-1], [float("inf")], [float("nan")]):
            with self.subTest(samples=samples), self.assertRaises(ValueError):
                baseline.summarize(samples)


if __name__ == "__main__":
    unittest.main()
