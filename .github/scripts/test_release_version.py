from pathlib import Path
import tempfile
import unittest

from release_version import ROOT, choose_version, parse_version, stamp, version_code


class ReleaseVersionTest(unittest.TestCase):
    def test_starts_at_015_ignoring_old_main_builds(self):
        self.assertEqual(choose_version(["v0.1.0", "main-45"], "0.1.5"), "0.1.5")
        self.assertEqual(choose_version([], "0.1.5"), "0.1.5")
        self.assertEqual(choose_version(["v0.1.0"], "0.1.5", ["v0.1.0"]), "0.1.5")

    def test_increments_numerically_and_reuses_a_tag_on_retry(self):
        tags = ["v0.1.5", "v0.1.9", "v0.1.10", "v1.0.0-rc.1", "main-999"]
        self.assertEqual(choose_version(tags, "0.1.5"), "0.1.11")
        self.assertEqual(choose_version(tags, "0.1.5", ["v0.1.5"]), "0.1.5")
        self.assertEqual(choose_version(tags + ["v0.2.0"], "0.1.5"), "0.2.1")
        self.assertEqual(choose_version(tags, "0.2.0"), "0.2.0")

    def test_explicit_versions_are_validated(self):
        self.assertEqual(choose_version([], "0.1.5", requested="v0.1.6"), "0.1.6")
        for value in ["main-45", "0.01.5", "0.1.5\nsha=bad", "0.1.5; echo bad", "0.1.5-rc.1"]:
            with self.subTest(value=value), self.assertRaises(ValueError):
                parse_version(value)
        self.assertLess(version_code("0.1.5"), version_code("0.1.6"))
        self.assertLess(version_code("0.1.999"), version_code("0.2.0"))
        with self.assertRaises(ValueError):
            version_code("0.1.1000")

    def test_stamps_both_apps_and_the_lockfile(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            names = ["linux/Cargo.toml", "linux/Cargo.lock", "android/app/build.gradle.kts"]
            for name in names:
                target = root / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text((ROOT / name).read_text())
            stamp("0.1.6", root)
            self.assertIn('version = "0.1.6"', (root / names[0]).read_text())
            self.assertIn('name = "airbridge"\nversion = "0.1.6"', (root / names[1]).read_text())
            android = (root / names[2]).read_text()
            self.assertIn('versionName = "0.1.6"', android)
            self.assertIn('versionCode = 1006', android)


if __name__ == "__main__":
    unittest.main()
