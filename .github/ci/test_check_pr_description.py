# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

import contextlib
import io
import json
import subprocess
import unittest
from unittest import mock

from check_pr_description import main, validation_error


class ValidationTests(unittest.TestCase):
    def test_description_cases(self):
        cases = (
            (" \n\t", "template", "must not be empty"),
            ("Heading \n <!-- guidance --> text", "Heading text", "Fill in"),
            ("New text <!-- incomplete", "template", "Remove all"),
            ("New text <!-- complete -->", "template", "Remove all"),
            ("Completed description", "template", None),
        )
        for body, template, expected in cases:
            with self.subTest(body=body):
                error = validation_error(body, template)
                if expected is None:
                    self.assertIsNone(error)
                else:
                    self.assertIn(expected, error)

    def test_main_fetches_current_body_and_base_template(self):
        env = {"GITHUB_REPOSITORY": "owner/repo", "PR_NUMBER": "42", "BASE_SHA": "abc"}
        output = io.StringIO()
        with mock.patch.dict("os.environ", env, clear=True), mock.patch(
            "subprocess.check_output", side_effect=[json.dumps({"body": "Done"}), "Template"]
        ) as fetch, contextlib.redirect_stdout(output):
            self.assertEqual(main(), 0)
        self.assertEqual(fetch.call_args_list, [
            mock.call(["gh", "api", "repos/owner/repo/pulls/42"], text=True),
            mock.call([
                "gh", "api", "--header", "Accept: application/vnd.github.raw+json",
                "repos/owner/repo/contents/.github/PULL_REQUEST_TEMPLATE.md?ref=abc",
            ], text=True),
        ])
        self.assertIn("is valid", output.getvalue())

    def test_main_fails_closed_on_fetch_failure(self):
        env = {"GITHUB_REPOSITORY": "owner/repo", "PR_NUMBER": "42", "BASE_SHA": "abc"}
        with mock.patch.dict("os.environ", env, clear=True), mock.patch(
            "subprocess.check_output", side_effect=subprocess.CalledProcessError(1, "gh")
        ), contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(main(), 1)


if __name__ == "__main__":
    unittest.main()
