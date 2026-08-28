#!/usr/bin/env python3
"""Generate GitHub Release notes from commits since the previous tag."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


def command(*args: str) -> str:
    return subprocess.check_output(args, text=True, stderr=subprocess.STDOUT).strip()


def release_context() -> tuple[str, str, str]:
    sha = os.environ.get("GITHUB_SHA") or command("git", "rev-parse", "HEAD")
    tag = os.environ.get("GITHUB_REF_NAME", "")
    try:
        previous = command("git", "describe", "--tags", "--abbrev=0", f"{sha}^")
    except subprocess.CalledProcessError:
        previous = ""

    if previous:
        commits = command("git", "log", "--format=%h %s", f"{previous}..{sha}")
        diff = command("git", "diff", "--stat", "--patch", previous, sha)
    else:
        commits = command("git", "log", "--format=%h %s", "--max-count=30", sha)
        diff = command("git", "show", "--stat", "--patch", sha)

    return tag, commits, diff[-70000:]


def provider_endpoint() -> str:
    provider = os.environ["AI_PROVIDER_URL"].rstrip("/")
    if provider.endswith("/v1/messages"):
        return provider
    if provider.endswith("/v1"):
        return f"{provider}/messages"
    return f"{provider}/v1/messages"


def generate_notes(tag: str, commits: str, diff: str) -> str:
    payload = {
        "model": os.environ["AI_MODEL"],
        "max_tokens": 1200,
        "temperature": 0.2,
        "system": "You write accurate, polished GitHub release notes for an open-source desktop application.",
        "messages": [
            {
                "role": "user",
                "content": f"""Write the release description for {tag or 'this release'}.

Return only Markdown suitable for the body of a GitHub Release. Do not use a code fence.
Make it feel like a real open-source project release announcement:
- Start with one short, human introduction paragraph.
- Add concise sections such as ## Highlights, ## Notable changes, ## Bug fixes, or ## Notes.
- Use bullets where they improve scanning.
- Mention only changes supported by the commit list and diff.
- Do not invent features, bug fixes, compatibility, benchmarks, contributors, issue numbers, or links.
- Do not list downloadable assets; GitHub displays those separately.
- Prefer user-facing language and explain why a change matters.
- Omit empty sections.

Commits since the previous release:
{commits}

Diff:
{diff}
""",
            }
        ],
    }
    request = Request(
        provider_endpoint(),
        data=json.dumps(payload).encode("utf-8"),
        headers={
            "content-type": "application/json",
            "anthropic-version": "2023-06-01",
            "x-api-key": os.environ["AI_SECRET_KEY"],
        },
        method="POST",
    )

    try:
        with urlopen(request, timeout=120) as response:
            data = json.load(response)
    except HTTPError as error:
        body = error.read().decode("utf-8", errors="replace")[:1000]
        raise RuntimeError(f"AI provider returned HTTP {error.code}: {body}") from error
    except URLError as error:
        raise RuntimeError(f"Could not reach AI provider: {error.reason}") from error

    text = "\n".join(
        item.get("text", "")
        for item in data.get("content", [])
        if item.get("type") == "text"
    ).strip()
    if not text:
        raise RuntimeError("AI provider returned no release notes")
    return text.replace("```markdown", "").replace("```", "").strip()


def main() -> int:
    required = ("AI_MODEL", "AI_PROVIDER_URL", "AI_SECRET_KEY")
    missing = [name for name in required if not os.environ.get(name)]
    if missing:
        print(f"Missing required environment variables: {', '.join(missing)}", file=sys.stderr)
        return 2

    tag, commits, diff = release_context()
    print(generate_notes(tag, commits, diff))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
