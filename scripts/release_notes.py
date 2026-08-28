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
    tag = os.environ.get("RELEASE_TAG") or os.environ.get("GITHUB_REF_NAME", "")
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


def normalize_notes(text: str) -> str:
    """Keep the release body in a scannable heading-and-bullets format."""
    lines = []
    for line in text.replace("```markdown", "").replace("```", "").splitlines():
        stripped = line.strip()
        if not stripped:
            if lines and lines[-1] != "":
                lines.append("")
            continue
        if stripped.startswith("#") or stripped.startswith(("- ", "* ", "+ ")):
            lines.append(stripped)
        else:
            lines.append(f"- {stripped}")

    while lines and lines[-1] == "":
        lines.pop()
    if not any(line.startswith("## ") for line in lines):
        lines.insert(0, "## Что изменилось")
    return "\n".join(lines)


def generate_notes(tag: str, commits: str, diff: str) -> str:
    payload = {
        "model": os.environ["AI_MODEL"],
        "max_tokens": 1200,
        "temperature": 0.2,
        "system": "You write concise, accurate GitHub release notes in Russian for an open-source desktop application. Your output must be a useful change list for users, not a commit report.",
        "messages": [
            {
                "role": "user",
                "content": f"""Prepare the GitHub Release description for {tag or 'this release'}.

Return only Markdown suitable for the body of a GitHub Release. Do not use a code fence.
The result must be a human-readable list of concrete changes:
- Start immediately with a section heading: ## Что изменилось
- Use 3–8 concise bullet points. Each bullet describes one real, specific change and why it matters to a user.
- Group bullets under short headings such as ## Что нового, ## Исправления, and ## Технические изменения when useful.
- Do not write a generic introduction paragraph, a daily summary, or phrases like "сегодня", "эта версия в первую очередь", or "по коммитам".
- Do not repeat the release number in every bullet.
- Mention only changes directly supported by the commit list or diff. Never invent features, bug fixes, compatibility, benchmarks, contributors, issue numbers, or links.
- Prefer user-facing wording. Mention CI, packaging, or internal refactoring only when it changes what users can download or use.
- Do not list downloadable assets; GitHub displays those separately.
- Omit empty sections. Every non-heading line in the result must be a bullet point.

Use this shape (adapt the headings and content, do not copy the example):
## Что нового
- Коротко и конкретно описано изменение и его польза.

## Исправления
- Конкретно описано исправление.

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
    return normalize_notes(text)


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
