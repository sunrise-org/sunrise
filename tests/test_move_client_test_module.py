import tempfile
import unittest
from pathlib import Path

from tools import move_client_test_module


class MoveClientTestModuleTests(unittest.TestCase):
    def make_repo(self, root):
        source = root / "crates/sunrise-client/src/pieces/moved.rs"
        source.parent.mkdir(parents=True)
        source.write_text("#[cfg(test)]\nmod tests { fn sample() {} }\n", encoding="utf-8")

        declaration = root / "crates/sunrise-client/src/server.rs"
        declaration.parent.mkdir(parents=True, exist_ok=True)
        declaration.write_text(
            '#[path = "pieces/moved.rs"]\npub(crate) mod moved;\nmod unchanged;\n',
            encoding="utf-8",
        )

        integration_main = root / "crates/sunrise-client/tests/it/main.rs"
        integration_main.parent.mkdir(parents=True)
        integration_main.write_text("mod existing;\n", encoding="utf-8")

        ui_wrapper = root / "crates/sunrise-client/tests/it/ui.rs"
        ui_wrapper.write_text("pub(crate) mod sibling;\n", encoding="utf-8")
        deep_wrapper = root / "crates/sunrise-client/tests/it/ui/deep.rs"
        deep_wrapper.parent.mkdir(parents=True)
        deep_wrapper.write_text("mod keep;\n", encoding="utf-8")

        policy_dir = root / "tools/ci/ignored-tests"
        policy_dir.mkdir(parents=True)
        first_policy = policy_dir / "daily-sunrise-client-a.txt"
        first_policy.write_text(
            "# policy comment\n"
            "sunrise-client ui::deep::moved::tests::first lib daily resources # preserve me\n"
            "sunrise-client ui::deep::moved::tests::second lib daily child-process\n"
            "sunrise-client ui::deep::moved_extra::tests::near_match lib daily resources\n"
            "sunrise-client ui::deep::moved::tests::verify lib verify resources\n",
            encoding="utf-8",
        )
        second_policy = policy_dir / "daily-sunrise-client-b.txt"
        second_policy.write_text(
            "sunrise-client ui::deep::moved::tests::third lib daily resources\n",
            encoding="utf-8",
        )
        return {
            "source": source,
            "declaration": declaration,
            "integration_main": integration_main,
            "policy_dir": policy_dir,
            "first_policy": first_policy,
            "second_policy": second_policy,
            "destination": root / "crates/sunrise-client/tests/it/ui/deep/moved.rs",
            "ui_wrapper": ui_wrapper,
            "deep_wrapper": deep_wrapper,
        }

    @staticmethod
    def run_extraction(root, paths):
        return move_client_test_module.extract(
            root,
            str(paths["source"]),
            str(paths["declaration"]),
            "ui::deep::moved",
            str(paths["integration_main"]),
            str(paths["policy_dir"]),
        )

    def test_moves_module_and_rewrites_only_its_daily_rows(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            paths = self.make_repo(root)
            source_text = paths["source"].read_text(encoding="utf-8")

            changed_rows = self.run_extraction(root, paths)

            self.assertEqual(changed_rows, 3)
            self.assertFalse(paths["source"].exists())
            self.assertEqual(paths["destination"].read_text(encoding="utf-8"), source_text)
            self.assertEqual(
                paths["declaration"].read_text(encoding="utf-8"),
                "mod unchanged;\n",
            )
            self.assertEqual(
                paths["integration_main"].read_text(encoding="utf-8"),
                "mod existing;\nmod ui;\n",
            )
            self.assertEqual(
                paths["ui_wrapper"].read_text(encoding="utf-8"),
                "pub(crate) mod sibling;\npub(crate) mod deep;\n",
            )
            self.assertEqual(
                paths["deep_wrapper"].read_text(encoding="utf-8"),
                "mod keep;\npub(crate) mod moved;\n",
            )
            self.assertEqual(
                paths["first_policy"].read_text(encoding="utf-8"),
                "# policy comment\n"
                "sunrise-client ui::deep::moved::tests::first test:it daily resources # preserve me\n"
                "sunrise-client ui::deep::moved::tests::second test:it daily child-process\n"
                "sunrise-client ui::deep::moved_extra::tests::near_match lib daily resources\n"
                "sunrise-client ui::deep::moved::tests::verify lib verify resources\n",
            )
            self.assertEqual(
                paths["second_policy"].read_text(encoding="utf-8"),
                "sunrise-client ui::deep::moved::tests::third test:it daily resources\n",
            )

    def test_missing_policy_rows_fail_before_any_file_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            paths = self.make_repo(root)
            paths["first_policy"].write_text(
                "sunrise-client ui::deep::other::tests::unrelated lib daily resources\n",
                encoding="utf-8",
            )
            paths["second_policy"].write_text(
                "sunrise-client ui::deep::other::tests::also_unrelated lib daily resources\n",
                encoding="utf-8",
            )
            before = {key: path.read_bytes() for key, path in paths.items() if path.is_file()}

            with self.assertRaises(move_client_test_module.ExtractionError):
                self.run_extraction(root, paths)

            for key, contents in before.items():
                self.assertEqual(paths[key].read_bytes(), contents, key)
            self.assertFalse(paths["destination"].exists())

    def test_preceding_outer_attributes_fail_before_any_file_changes(self):
        for cfg_attribute in (
            "#[cfg(test)]",
            "#[cfg(\n    test\n)]",
            '#[cfg(all(test, feature = "client"))]',
            '#[cfg(\n    all(\n        test,\n        feature = "client"\n    )\n)]',
        ):
            with self.subTest(cfg_attribute=cfg_attribute), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                paths = self.make_repo(root)
                paths["declaration"].write_text(
                    f'{cfg_attribute}\n'
                    '#[path = "pieces/moved.rs"]\n'
                    'pub(crate) mod moved;\n'
                    'mod unchanged;\n',
                    encoding="utf-8",
                )
                before = {
                    path.relative_to(root): path.read_bytes()
                    for path in root.rglob("*")
                    if path.is_file()
                }

                with self.assertRaisesRegex(
                    move_client_test_module.ExtractionError,
                    r"preceding unsupported outer attribute",
                ):
                    self.run_extraction(root, paths)

                after = {
                    path.relative_to(root): path.read_bytes()
                    for path in root.rglob("*")
                    if path.is_file()
                }
                self.assertEqual(after, before)
                self.assertFalse(paths["destination"].exists())

    def test_destination_collision_and_mismatched_declaration_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            paths = self.make_repo(root)
            paths["destination"].parent.mkdir(parents=True)
            paths["destination"].write_text("pre-existing\n", encoding="utf-8")
            before_source = paths["source"].read_bytes()
            before_declaration = paths["declaration"].read_bytes()
            with self.assertRaises(move_client_test_module.ExtractionError):
                self.run_extraction(root, paths)
            self.assertEqual(paths["source"].read_bytes(), before_source)
            self.assertEqual(paths["declaration"].read_bytes(), before_declaration)
            self.assertEqual(paths["destination"].read_text(encoding="utf-8"), "pre-existing\n")

        mismatches = (
            '#[path = "pieces/moved.rs"]\npub(crate) mod other_name;\n',
            '#[path = "pieces/moved.rs"]\npub mod moved;\n',
        )
        for declaration in mismatches:
            with self.subTest(declaration=declaration), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                paths = self.make_repo(root)
                paths["declaration"].write_text(declaration, encoding="utf-8")
                before_source = paths["source"].read_bytes()
                before_declaration = paths["declaration"].read_bytes()
                with self.assertRaises(move_client_test_module.ExtractionError):
                    self.run_extraction(root, paths)
                self.assertEqual(paths["source"].read_bytes(), before_source)
                self.assertEqual(paths["declaration"].read_bytes(), before_declaration)
                self.assertFalse(paths["destination"].exists())

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            paths = self.make_repo(root)
            paths["deep_wrapper"].write_text(
                '#[path = "elsewhere.rs"] pub(crate) mod moved;\n',
                encoding="utf-8",
            )
            with self.assertRaises(move_client_test_module.ExtractionError):
                self.run_extraction(root, paths)
            self.assertTrue(paths["source"].exists())
            self.assertFalse(paths["destination"].exists())


if __name__ == "__main__":
    unittest.main()
