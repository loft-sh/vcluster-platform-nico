# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Validate the current pull request description against its base template."""

import json
import os
import re
import subprocess


def validation_error(body: str, template: str) -> str | None:
    if not "".join(body.split()):
        return "Pull request description must not be empty"

    def normalize(value: str) -> str:
        without_comments = re.sub(r"<!--.*?-->", "", value, flags=re.DOTALL)
        return "".join(without_comments.split())

    if normalize(body) == normalize(template):
        return "Fill in the pull request template before submitting"
    if "<!--" in body:
        return "Remove all <!-- ... --> comments from the pull request description"
    return None


def main() -> int:
    try:
        repository = os.environ["GITHUB_REPOSITORY"]
        number = os.environ["PR_NUMBER"]
        base_sha = os.environ["BASE_SHA"]
        pr_json = subprocess.check_output(
            ["gh", "api", f"repos/{repository}/pulls/{number}"], text=True
        )
        template = subprocess.check_output(
            [
                "gh", "api", "--header", "Accept: application/vnd.github.raw+json",
                f"repos/{repository}/contents/.github/PULL_REQUEST_TEMPLATE.md?ref={base_sha}",
            ],
            text=True,
        )
        body = json.loads(pr_json).get("body") or ""
        if not isinstance(body, str):
            raise ValueError("PR body is not a string")
        error = validation_error(body, template)
    except (KeyError, ValueError, subprocess.CalledProcessError, OSError) as exc:
        print(f"::error::PR description validation could not complete: {exc}")
        return 1

    if error:
        print(f"::error::{error}")
        return 1
    print("Pull request description is valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
