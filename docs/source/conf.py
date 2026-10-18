"""Sphinx configuration for the claimdag project site (Shibuya theme)."""

from __future__ import annotations

import html
import re
from pathlib import Path

_DOCS = Path(__file__).resolve().parent
_ROOT = _DOCS.parent.parent
_ORG = _DOCS.parent / "orgmode"

project = "claimdag"
copyright = "2026, Rohit Goswami"
author = "Rohit Goswami"
release = "0.2.0"
version = "0.2"

extensions = [
    "sphinx.ext.mathjax",
    "sphinx_copybutton",
    "sphinx_design",
]

templates_path = ["_templates"]
exclude_patterns: list[str] = []

html_theme = "shibuya"
html_static_path = ["_static"]
html_favicon = "_static/favicon.svg"
html_logo = "_static/mark.svg"
html_title = "claimdag"
html_css_files = ["custom.css"]

# View Source reads the copy Sphinx writes under _sources/. Without the
# site URL, Shibuya points it at a raw GitHub path that adds .txt and
# does not exist.
html_baseurl = "https://leidarljos.github.io/claimdag/"
html_copy_source = True
html_show_sourcelink = True

html_context = {
    "source_type": "github",
    "source_user": "leidarljos",
    "source_repo": "claimdag",
    "source_version": "main",
    "source_docs_path": "/docs/source/",
}

_OG_IMAGE = "https://leidarljos.github.io/claimdag/_static/og.png"

html_theme_options = {
    "accent_color": "gold",
    "color_mode": "auto",
    "dark_code": True,
    "open_in_chatgpt": False,
    "open_in_claude": False,
    "github_url": "https://github.com/leidarljos/claimdag",
    "og_image_url": _OG_IMAGE,
    "nav_links": [
        {"title": "Get started", "url": "getting-started"},
        {"title": "How-to", "url": "howto"},
        {"title": "Reference", "url": "reference"},
        {"title": "Explanation", "url": "explanation"},
        {"title": "Search", "url": "search"},
    ],
}

# Offline builds must not reach for an inventory.
intersphinx_mapping: dict = {}

copybutton_prompt_text = r"\$ "
copybutton_prompt_is_regexp = True

# Pages Sphinx builds with no org source.
_EXTRA_DESC = {
    "search": "Search the claimdag documentation.",
    "genindex": "Index of the claimdag documentation.",
}

_TITLE = re.compile(r"^#\+title:\s*(.+?)\s*$", re.MULTILINE)
_DESC = re.compile(r"^#\+description:\s*(.+?)\s*$", re.MULTILINE)
_H1_OPEN = re.compile(r"<h1(\s[^>]*)?>")
_H1_BLOCK = re.compile(r"<h1(\s[^>]*)?>.*?</h1>", re.DOTALL)
_BLANK = re.compile(r"<a\b[^>]*\btarget=\"_blank\"[^>]*>", re.IGNORECASE | re.DOTALL)


def _org_pages() -> dict[str, dict[str, str]]:
    pages: dict[str, dict[str, str]] = {}
    missing: list[str] = []
    for path in sorted(_ORG.glob("*.org")):
        text = path.read_text(encoding="utf-8")
        title = _TITLE.search(text)
        desc = _DESC.search(text)
        if not title or not desc:
            missing.append(path.name)
            continue
        pages[path.stem] = {
            "title": title.group(1).strip(),
            "description": desc.group(1).strip(),
        }
    if missing:
        raise RuntimeError(
            "org pages need #+title and #+description: " + ", ".join(missing)
        )
    return pages


_PAGES = _org_pages()


def _heading_text(block: str) -> str:
    text = re.sub(r"<[^>]+>", "", block)
    text = re.sub(r"\s+", " ", text).strip()
    if text.endswith("#"):
        text = text[:-1].strip()
    return text


def _one_h1(body: str, title: str) -> str:
    """Keep a single h1, the page title, without drawing a second heading."""
    blocks = list(_H1_BLOCK.finditer(body))
    if len(blocks) == 1 and _heading_text(blocks[0].group(0)) == title:
        return body
    demoted = _H1_OPEN.sub(
        lambda match: '<h2 class="ll-section"' + (match.group(1) or "") + ">",
        body,
    )
    demoted = demoted.replace("</h1>", "</h2>")
    return f'<h1 class="ll-sr">{html.escape(title)}</h1>\n{demoted}'


def _html_page_context(app, pagename, templatename, context, doctree) -> None:
    meta = context.get("meta")
    if not isinstance(meta, dict):
        meta = {}
        context["meta"] = meta
    page = _PAGES.get(pagename)
    if page:
        # share_name is the document title. context["title"] stays the first
        # section so the breadcrumb, which prints it, does not move.
        context["share_name"] = page["title"]
        meta["description"] = page["description"]
        context["body"] = _one_h1(context.get("body") or "", page["title"])
    elif pagename in _EXTRA_DESC:
        meta["description"] = _EXTRA_DESC[pagename]
    meta["cover"] = _OG_IMAGE


def _noopener(tag: str) -> str:
    if re.search(r"\bnoopener\b", tag):
        return tag
    rel = re.search(r'\brel="([^"]*)"', tag)
    if not rel:
        return tag[:-1] + ' rel="noopener noreferrer">'
    value = rel.group(1)
    extra = [part for part in ("noopener", "noreferrer") if part not in value.split()]
    if not extra:
        return tag
    return tag[: rel.start(1)] + value + " " + " ".join(extra) + tag[rel.end(1) :]


def _blank_targets(app, exception) -> None:
    if exception:
        return
    out = Path(app.outdir)
    for path in out.rglob("*.html"):
        text = path.read_text(encoding="utf-8")
        new = _BLANK.sub(lambda match: _noopener(match.group(0)), text)
        if new != text:
            path.write_text(new, encoding="utf-8")


def setup(app) -> None:
    app.connect("html-page-context", _html_page_context)
    app.connect("build-finished", _blank_targets)
