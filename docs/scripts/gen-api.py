#!/usr/bin/env python3
"""Write docs/orgmode/api.org from the public claimdag sources.

The page lists the items a downstream crate can name: the re-exports in
src/lib.rs, the public module lock, and the public fields, variants and
inherent methods of those types. Doc comments are copied from the source.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SRC = ROOT / "crates" / "claimdag" / "src"
OUT = ROOT / "docs" / "orgmode" / "api.org"

# Generated schema. It is a private module and not part of the crate API.
SKIP_FILES = {"claimdag_capnp.rs"}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    text = render()
    if args.check:
        current = OUT.read_text(encoding="utf-8") if OUT.is_file() else ""
        if current != text:
            print("gen-api: api.org drifted from the crate sources", file=sys.stderr)
            return 1
        print("gen-api: api.org matches the crate sources")
        return 0
    OUT.write_text(text, encoding="utf-8")
    print(f"gen-api: wrote {OUT.relative_to(ROOT)}")
    return 0


def render() -> str:
    files = {
        path.name: path.read_text(encoding="utf-8")
        for path in sorted(SRC.glob("*.rs"))
        if path.name not in SKIP_FILES
    }
    parsed = {name: parse_file(text) for name, text in files.items()}
    exports, lock_mod = read_exports(parsed["lib.rs"])
    by_rust: dict[tuple[str, str], Item] = {}
    for filename, parsed_file in parsed.items():
        module = "crate" if filename == "lib.rs" else filename[: -len(".rs")]
        for item in parsed_file.items:
            if item.kind in {"fn", "struct", "enum", "const", "type"} and item.parent is None:
                by_rust[(module, item.name)] = item
    lines: list[str] = [
        "#+title: API reference",
        "#+description: Public items of the claimdag crate, generated from the Rust sources.",
        "#+options: toc:nil num:nil",
        "",
        "The items below are the public surface of the =claimdag= library, read from",
        "the crate sources. =cargo doc --no-deps -p claimdag= writes the same items",
        "as HTML. =docs/build.sh= and the docs-site workflow copy that HTML to",
        "=rustdoc/claimdag/index.html= next to this page.",
        "",
        "#+begin_export html",
        '<p><a href="rustdoc/claimdag/index.html">claimdag rustdoc</a></p>',
        "#+end_export",
        "",
        "The command line is the [[file:reference.org][reference]]. The panes and the",
        "server are the [[file:howto.org][how-to guides]].",
        "",
    ]
    lib_doc = parsed["lib.rs"].module_doc
    if lib_doc:
        lines.append("* Crate")
        lines.append("")
        lines.extend(doc_to_org(lib_doc))
        lines.append("")
    lines.append("* Re-exports")
    lines.append("")
    lines.append("Each name is what a downstream crate writes after =claimdag::=.")
    lines.append("")
    for public_name, (module, rust_name) in exports:
        item = by_rust.get((module, rust_name))
        if item is None:
            lines.append(f"** {public_name}")
            lines.append("")
            lines.append(f"Re-exported from ={module}::{rust_name}=. The source item was not found.")
            lines.append("")
            continue
        lines.extend(emit_item(public_name, item, parsed, module))
    if lock_mod:
        lines.append("* Module =lock=")
        lines.append("")
        lock_doc = parsed["lock.rs"].module_doc
        if lock_doc:
            lines.extend(doc_to_org(lock_doc))
            lines.append("")
        for item in parsed["lock.rs"].items:
            if item.parent is None and item.kind in {"fn", "struct", "enum", "const", "type"}:
                if not item.public:
                    continue
                lines.extend(emit_item(f"lock::{item.name}", item, parsed, "lock"))
    text = "\n".join(lines).rstrip() + "\n"
    return text


def emit_item(title: str, item: Item, parsed: dict[str, File], module: str) -> list[str]:
    out = [f"** {title}", ""]
    if title != item.name:
        out.append(f"Source name: ={module}::{item.name}=.")
        out.append("")
    out.append("#+begin_src rust")
    out.append(item.signature.rstrip())
    out.append("#+end_src")
    out.append("")
    if item.doc:
        out.extend(doc_to_org(item.doc))
        out.append("")
    children = [
        child
        for file in parsed.values()
        for child in file.items
        if child.parent == item.name and child.public and child.kind in {"method", "variant", "field"}
    ]
    # Fields and variants are recorded on the file that defined the type.
    # Methods too. Filter to the defining module by identity of the parent item.
    children = [child for child in children if child.origin == item.origin]
    variants = [child for child in children if child.kind == "variant"]
    fields = [child for child in children if child.kind == "field"]
    methods = [child for child in children if child.kind == "method"]
    if variants:
        out.append("Variants:")
        out.append("")
        for child in variants:
            if child.doc:
                out.append(f"- ={child.name}=: {one_line(child.doc)}")
                continue
            sig = " ".join(child.signature.split())
            if sig != child.name:
                out.append(f"- ={child.name}=: ={sig}=")
            else:
                out.append(f"- ={child.name}=")
        out.append("")
    if fields:
        out.append("Fields:")
        out.append("")
        for child in fields:
            sig = child.signature.strip()
            note = one_line(child.doc)
            if note:
                out.append(f"- ={sig}=: {note}")
            else:
                out.append(f"- ={sig}=")
        out.append("")
    for child in methods:
        out.append(f"*** {title}::{child.name}")
        out.append("")
        out.append("#+begin_src rust")
        out.append(child.signature.rstrip())
        out.append("#+end_src")
        out.append("")
        if child.doc:
            out.extend(doc_to_org(child.doc))
            out.append("")
    return out


def one_line(doc: str) -> str:
    text = " ".join(line.strip() for line in doc.splitlines() if line.strip())
    text = convert_inline(text)
    text = re.sub(r"\s+", " ", text).strip()
    return text


class Item:
    def __init__(
        self,
        kind: str,
        name: str,
        signature: str,
        doc: str,
        public: bool,
        parent: str | None,
        origin: str,
    ) -> None:
        self.kind = kind
        self.name = name
        self.signature = signature
        self.doc = doc
        self.public = public
        self.parent = parent
        self.origin = origin


class File:
    def __init__(self, module_doc: str, items: list[Item]) -> None:
        self.module_doc = module_doc
        self.items = items


def parse_file(text: str) -> File:
    lines = text.splitlines()
    module_doc: list[str] = []
    i = 0
    while i < len(lines) and (lines[i].startswith("//!") or lines[i].strip() == ""):
        if lines[i].startswith("//!"):
            module_doc.append(strip_doc(lines[i], "//!"))
        i += 1
    items: list[Item] = []
    # (brace depth of the body, owner, "type" or "impl")
    bodies: list[tuple[int, str | None, str]] = []
    brace = 0
    pending_doc: list[str] = []
    pending_attr: list[str] = []
    origin = "file"
    while i < len(lines):
        raw = lines[i]
        stripped = raw.strip()
        if stripped.startswith("///"):
            pending_doc.append(strip_doc(raw, "///"))
            i += 1
            continue
        if stripped.startswith("//"):
            i += 1
            continue
        if stripped == "":
            pending_doc = []
            pending_attr = []
            i += 1
            continue
        if stripped.startswith("#["):
            if stripped == "#[cfg(test)]":
                pending_doc = []
                pending_attr = []
            else:
                pending_attr.append(stripped)
            i += 1
            continue
        if stripped.startswith("impl"):
            owner = inherent_impl(stripped)
            if "{" in stripped:
                bodies.append((brace + stripped.count("{"), owner, "impl"))
            brace += stripped.count("{") - stripped.count("}")
            pending_doc = []
            pending_attr = []
            i += 1
            continue
        type_owner = next((name for _depth, name, kind in reversed(bodies) if kind == "type" and name), None)
        impl_owner = next((name for _depth, name, kind in reversed(bodies) if kind == "impl" and name), None)
        if stripped.startswith("}"):
            pending_doc = []
            pending_attr = []
            brace += raw.count("{") - raw.count("}")
            i += 1
        elif starts_signature(stripped):
            header, consumed, _ended = take_header(lines, i)
            item = make_item(header, pending_doc, pending_attr, impl_owner, origin)
            if item is not None:
                items.append(item)
                if item.kind in {"struct", "enum"} and any("{" in part for part in lines[i : i + consumed]):
                    bodies.append((brace + 1, item.name, "type"))
            pending_doc = []
            pending_attr = []
            for part in lines[i : i + consumed]:
                brace += part.count("{") - part.count("}")
            i += consumed
        elif type_owner and not impl_owner:
            header, consumed = take_member(lines, i)
            item = make_member(header, pending_doc, type_owner, origin)
            if item is not None:
                items.append(item)
            pending_doc = []
            pending_attr = []
            for part in lines[i : i + consumed]:
                brace += part.count("{") - part.count("}")
            i += consumed
        else:
            pending_doc = []
            pending_attr = []
            brace += raw.count("{") - raw.count("}")
            i += 1
        while bodies and brace < bodies[-1][0]:
            bodies.pop()
    return File("\n".join(module_doc).strip(), items)


def inherent_impl(stripped: str) -> str | None:
    if " for " in stripped:
        return None
    match = re.match(r"impl(?:<[^>]*>)?\s+([A-Za-z0-9_]+)\b", stripped)
    if not match:
        return None
    return match.group(1)


def starts_signature(stripped: str) -> bool:
    return bool(
        re.match(
            r"(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|fn|const|type)\b",
            stripped,
        )
    )


def take_member(lines: list[str], start: int) -> tuple[str, int]:
    """One enum variant or struct field, ending at a comma outside brackets."""
    chunk: list[str] = []
    depth = 0
    for offset, line in enumerate(lines[start:], start=1):
        chunk.append(line)
        for char in line:
            if char in "({[":
                depth += 1
            elif char in ")}]":
                depth = max(0, depth - 1)
            elif char == "," and depth == 0:
                return "\n".join(chunk), offset
        if offset > 30:
            break
    return "\n".join(chunk), len(chunk)


def make_member(header: str, doc: list[str], parent: str, origin: str) -> Item | None:
    text = header.strip().rstrip(",").strip()
    if not text or text.startswith("}"):
        return None
    field = re.match(
        r"^(?P<vis>pub(?:\([^)]*\))?)?\s*(?P<name>[A-Za-z0-9_]+)\s*:\s*(?P<ty>.+)$",
        text,
    )
    if field:
        vis = field.group("vis") or ""
        return Item(
            "field",
            field.group("name"),
            text,
            "\n".join(doc).strip(),
            vis == "pub",
            parent,
            origin,
        )
    variant = re.match(r"^(?P<name>[A-Za-z0-9_]+)\b", text)
    if variant and not text.startswith("pub ") and " fn " not in f" {text}":
        return Item(
            "variant",
            variant.group("name"),
            text,
            "\n".join(doc).strip(),
            True,
            parent,
            origin,
        )
    return None


def strip_doc(line: str, marker: str) -> str:
    rest = line.strip()[len(marker) :]
    if rest.startswith(" "):
        rest = rest[1:]
    return rest


def take_header(lines: list[str], start: int) -> tuple[str, int, bool]:
    """The signature lines, how many source lines they occupied, and whether `{` was seen."""
    chunk: list[str] = []
    i = start
    while i < len(lines):
        chunk.append(lines[i])
        joined = "\n".join(chunk)
        if "{" in joined or ";" in joined:
            # Cut at the first brace or semicolon that ends the signature.
            sig = []
            seen = False
            for part in chunk:
                if seen:
                    break
                if "{" in part or ";" in part:
                    cut = part
                    brace_at = cut.find("{")
                    semi_at = cut.find(";")
                    if brace_at == -1:
                        idx = semi_at
                    elif semi_at == -1:
                        idx = brace_at
                    else:
                        idx = min(brace_at, semi_at)
                    sig.append(cut[:idx].rstrip())
                    seen = True
                else:
                    sig.append(part)
            return "\n".join(sig).strip(), len(chunk), True
        i += 1
        if i - start > 40:
            break
    return "\n".join(chunk).strip(), len(chunk), False


def header_is_item(header: str) -> bool:
    first = header.lstrip()
    return bool(
        re.match(
            r"(?:pub(?:\([^)]*\))?\s+)?(?:struct|enum|fn|const|type|mod)\b",
            first,
        )
        or re.match(r"(?:pub(?:\([^)]*\))?\s+)?[A-Za-z0-9_]+\s*[,:(]", first)
        or re.match(r"(?:pub(?:\([^)]*\))?\s+)?[A-Za-z0-9_]+\s*$", first)
    )


_ITEM = re.compile(
    r"^(?P<vis>pub(?:\([^)]*\))?|\s*)\s*(?P<kind>struct|enum|fn|const|type|mod)\s+(?P<name>[A-Za-z0-9_]+)",
    re.M,
)


def make_item(
    header: str,
    doc: list[str],
    attrs: list[str],
    parent: str | None,
    origin: str,
) -> Item | None:
    text = header.strip()
    # Drop a where-clause leftover that is only whitespace.
    match = _ITEM.search(text)
    if match and match.start() == 0 or (match and text.startswith(match.group(0).split()[0] if False else "")):
        pass
    match = re.match(
        r"^(?P<vis>pub(?:\([^)]*\))?)?\s*(?P<kind>struct|enum|fn|const|type|mod)\s+(?P<name>[A-Za-z0-9_]+)",
        text,
    )
    if match:
        vis = match.group("vis") or ""
        public = vis == "pub"
        kind = match.group("kind")
        name = match.group("name")
        if parent and kind == "fn":
            kind = "method"
        if kind == "mod":
            return None
        signature = text
        if attrs:
            signature = "\n".join(attrs + [text])
        return Item(kind, name, signature, "\n".join(doc).strip(), public, parent, origin)
    # Enum variant or struct field inside a type body. The caller only passes
    # these when we are inside an impl or a type. Variants look like `Todo,`
    # or `Unreadable(PathBuf, String)`. Fields look like `pub id: WorkId`.
    field = re.match(
        r"^(?P<vis>pub(?:\([^)]*\))?)?\s*(?P<name>[A-Za-z0-9_]+)\s*:\s*(?P<ty>.+)$",
        text,
    )
    if field and parent:
        vis = field.group("vis") or ""
        return Item(
            "field",
            field.group("name"),
            text,
            "\n".join(doc).strip(),
            vis == "pub",
            parent,
            origin,
        )
    variant = re.match(r"^(?P<name>[A-Za-z0-9_]+)\s*(\([^)]*\)|\{[^}]*\})?\s*$", text)
    if variant and parent and not text.startswith("pub") and "fn " not in text:
        return Item(
            "variant",
            variant.group("name"),
            text,
            "\n".join(doc).strip(),
            True,
            parent,
            origin,
        )
    return None


def read_exports(lib: File) -> tuple[list[tuple[str, tuple[str, str]]], bool]:
    """Ordered (public name, (module, rust name)) pairs, and whether `lock` is public."""
    # Re-parse lib.rs text from the signatures we stored is lossy. Read the file.
    text = (SRC / "lib.rs").read_text(encoding="utf-8")
    # Drop comments so a commented pub use cannot leak in.
    text = re.sub(r"//.*?$", "", text, flags=re.M)
    exports: list[tuple[str, tuple[str, str]]] = []
    for match in re.finditer(
        r"pub\s+use\s+(?:crate::)?([A-Za-z0-9_]+)::\{([^}]+)\}",
        text,
    ):
        module = match.group(1)
        for part in match.group(2).split(","):
            part = part.strip()
            if not part:
                continue
            if " as " in part:
                rust_name, public = [piece.strip() for piece in part.split(" as ", 1)]
            else:
                rust_name = public = part
            exports.append((public, (module, rust_name)))
    for match in re.finditer(
        r"pub\s+use\s+(?:crate::)?([A-Za-z0-9_]+)::([A-Za-z0-9_]+)(?:\s+as\s+([A-Za-z0-9_]+))?\s*;",
        text,
    ):
        module, rust_name, alias = match.group(1), match.group(2), match.group(3)
        exports.append((alias or rust_name, (module, rust_name)))
    lock_mod = bool(re.search(r"pub\s+mod\s+lock\s*;", text))
    return exports, lock_mod


_LINK = re.compile(r"\[`([^`]+)`\](?:\([^)]*\))?")
_MD_LINK = re.compile(r"\[([^\]]+)\]\(([^)]+)\)")
_CODE = re.compile(r"`([^`]+)`")


def convert_inline(text: str) -> str:
    text = _LINK.sub(lambda m: f"={m.group(1)}=", text)
    text = _MD_LINK.sub(lambda m: f"{m.group(1)} ({m.group(2)})", text)

    def code(match: re.Match[str]) -> str:
        body = match.group(1)
        if "=" in body and "~" not in body:
            return f"~{body}~"
        if "=" in body:
            return body
        return f"={body}="

    return _CODE.sub(code, text)


def doc_to_org(doc: str) -> list[str]:
    out: list[str] = []
    para: list[str] = []
    in_code = False
    code_buf: list[str] = []
    code_lang = "text"

    def flush() -> None:
        if not para:
            return
        text = convert_inline(" ".join(part.strip() for part in para))
        text = re.sub(r"\s+", " ", text).strip()
        if re.match(r"^\[\d+\]", text) or re.match(r"^\d+\.\s", text):
            text = "\\ensp " + text
        out.append(text)
        out.append("")
        para.clear()

    for line in doc.splitlines():
        stripped = line.strip()
        if stripped.startswith("```"):
            flush()
            if not in_code:
                in_code = True
                code_lang = stripped[3:].strip() or "text"
                code_buf = []
            else:
                in_code = False
                out.append(f"#+begin_src {code_lang}")
                out.extend(code_buf)
                out.append("#+end_src")
                out.append("")
            continue
        if in_code:
            code_buf.append(line)
            continue
        if stripped.startswith("#"):
            flush()
            heading = stripped.lstrip("#").strip()
            out.append(f"*{convert_inline(heading)}*")
            out.append("")
            continue
        if stripped == "":
            flush()
            continue
        numbered = re.match(r"^(\d+)\.\s+(.*)$", stripped)
        bullet = re.match(r"^[-*]\s+(.*)$", stripped)
        if numbered or bullet:
            flush()
            body = numbered.group(2) if numbered else bullet.group(1)
            prefix = f"{numbered.group(1)}. " if numbered else ""
            # The hyphen keeps a numbered step off column zero, where Org
            # would treat it as a list and check_org would reject it.
            out.append("- " + prefix + convert_inline(body))
            continue
        if line.startswith((" ", "\t")) and out and out[-1].startswith("- "):
            out[-1] += " " + convert_inline(stripped)
            continue
        para.append(stripped)
    flush()
    if in_code:
        out.append(f"#+begin_src {code_lang}")
        out.extend(code_buf)
        out.append("#+end_src")
        out.append("")
    return out


if __name__ == "__main__":
    sys.exit(main())
