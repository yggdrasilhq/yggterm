"""Read-only legacy steering importer: plan, bind, and stage private candidates.

Consumes a legacy root-steer checkout (adapter yggsteer-steer-v1: a
targets.toml harness table plus the old 'steer' assembler script) and derives
proposal-only artifacts through the accepted private-source compiler
(ygg-steer-compiler.py). Planning reads one handle per source with pre/post
identity validation; candidate staging copies only the declared policy
Markdown parts plus small sealed metadata into a content-addressed
candidates/<id>/ directory of an already-initialized Ygg root.

This module never activates policy, stamps a target, syncs a host, deletes a
file, or claims the legacy repository is fully retired. Unclassified legacy
files are deletion blockers and stay in place. The adapter is versioned and
narrow: anything outside the recognized assembly grammar HOLDs instead of
pretending a general Python translation.
"""
from __future__ import annotations
import argparse
import ast
import ctypes
import datetime as dt
import errno
import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePath
import platform
import pwd
import shutil
import stat
import sys
import tomllib
import uuid
from contextlib import contextmanager

cspec = importlib.util.spec_from_file_location("ygg_import_compiler", Path(__file__).with_name("ygg-steer-compiler.py"))
compiler = importlib.util.module_from_spec(cspec)
cspec.loader.exec_module(compiler)
# One fs/Held identity across importer and compiler: reuse the compiler's load.
fs = compiler.fs

SCHEMA = 1
IMPORTER_VERSION = 2
ADAPTER_KIND = "yggsteer-steer-v1"
STEER_NAME = "steer"
TARGETS_NAME = "targets.toml"
LIMIT_PART_BYTES = 4 * 1024 * 1024      # same per-part bound as the compiler
LIMIT_STEER_BYTES = 512 * 1024
LIMIT_TOML_BYTES = 256 * 1024
LIMIT_JSON_BYTES = 256 * 1024
LIMIT_TREE_ENTRIES = 4096
LIMIT_TREE_DEPTH = 8
LIMIT_ASSEMBLY_APPENDS = 64
KNOWN_HARNESS_FIELDS = {"target", "core", "local"}


def sha(value):
    return hashlib.sha256(value).hexdigest()


def canonical(doc):
    return json.dumps(doc, sort_keys=True, separators=(",", ":")).encode()


def _owner_known(uid):
    try:
        return pwd.getpwuid(uid).pw_name
    except KeyError:
        raise fs.Held("source has an unknown owner; refusing unattributed input") from None


@contextmanager
def _input_directory(path):
    """Walk legacy input directories without following any component link.

    Unlike the private store walk, ownership/mode is not demanded here: a
    group-shared legacy checkout is legitimate unsealed proposed input.
    """
    path = Path(path).absolute()
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    fd = os.open(path.anchor, flags)
    try:
        for part in path.parts[1:]:
            child = os.open(part, flags, dir_fd=fd)
            os.close(fd)
            fd = child
        yield fd
    finally:
        os.close(fd)


def _slurp(stream, limit):
    return stream.read(limit + 1)


def _read_source(root, relative, limit):
    """One-handle read of a bounded regular legacy file with pre/post identity.

    Rejects symlink leaves/components, non-regular files, unknown owners,
    unbounded sizes, and bytes mutated while the handle was open.
    """
    rel = compiler._relative(relative)
    try:
        base = Path(root).resolve(strict=True)
    except OSError as exc:
        raise fs.Held(f"legacy root cannot be resolved ({type(exc).__name__})") from exc
    parts = rel.parts
    leaf_flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK
    dir_flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    try:
        with _input_directory(base) as root_fd:
            parent, chain = root_fd, []
            try:
                for part in parts[:-1]:
                    child = os.open(part, dir_flags, dir_fd=parent)
                    chain.append(child)
                    parent = child
                fd = os.open(parts[-1], leaf_flags, dir_fd=parent)
            finally:
                for handle in chain:
                    os.close(handle)
        with os.fdopen(fd, "rb") as stream:
            before = os.fstat(stream.fileno())
            if not stat.S_ISREG(before.st_mode):
                raise fs.Held("declared legacy source is not a regular file")
            _owner_known(before.st_uid)
            if before.st_size > limit:
                raise fs.Held("declared legacy source exceeds the bounded import input")
            raw = _slurp(stream, limit)
            after = os.fstat(stream.fileno())
    except fs.Held:
        raise
    except OSError as exc:
        raise fs.Held(f"legacy source is unreadable or linked ({type(exc).__name__})") from exc
    identity = ("st_dev", "st_ino", "st_size", "st_mtime_ns", "st_ctime_ns")
    if any(getattr(before, key) != getattr(after, key) for key in identity) or len(raw) > limit:
        raise fs.Held("legacy source changed or grew while it was being read")
    return raw, {"sha256": sha(raw), "bytes": len(raw), "uid": before.st_uid,
                 "owner": _owner_known(before.st_uid)}


def _scan_tree(root):
    """Bounded, link-free inventory of every legacy entry: rel path -> kind."""
    rows = {}

    def walk(dirpath, prefix, depth):
        if depth > LIMIT_TREE_DEPTH:
            raise fs.Held("legacy tree exceeds the bounded scan depth")
        try:
            with os.scandir(dirpath) as entries:
                listed = list(entries)
        except OSError as exc:
            raise fs.Held(f"legacy tree is not scannable ({type(exc).__name__})") from exc
        if len(rows) + len(listed) > LIMIT_TREE_ENTRIES:
            raise fs.Held("legacy tree exceeds the bounded scan size")
        for entry in listed:
            rel = prefix + entry.name
            if entry.is_symlink():
                rows[rel] = "symlink"
                continue
            if entry.is_dir(follow_symlinks=False):
                rows[rel] = "directory"
                walk(entry.path, rel + "/", depth + 1)
                continue
            try:
                mode = entry.stat(follow_symlinks=False).st_mode
            except OSError:
                rows[rel] = "other"
                continue
            if stat.S_ISREG(mode):
                rows[rel] = "file"
            elif stat.S_ISFIFO(mode):
                rows[rel] = "fifo"
            elif stat.S_ISSOCK(mode):
                rows[rel] = "socket"
            elif stat.S_ISBLK(mode) or stat.S_ISCHR(mode):
                rows[rel] = "device"
            else:
                rows[rel] = "other"

    walk(str(root), "", 0)
    return rows


def _constant_str(node):
    if isinstance(node, ast.Constant) and isinstance(node.value, str):
        return node.value
    return None


def _guard_tag(node, constants):
    literal = _constant_str(node)
    if literal is not None:
        return literal
    if isinstance(node, ast.Name) and node.id in constants:
        return constants[node.id]
    raise fs.Held("unsupported guard expression in the legacy assembly")


def _is_parts_append(node):
    return (isinstance(node, ast.Expr) and isinstance(node.value, ast.Call)
            and isinstance(node.value.func, ast.Attribute)
            and isinstance(node.value.func.value, ast.Name)
            and node.value.func.value.id == "parts" and node.value.func.attr == "append")


def _is_core_read(node):
    """(ROOT / t['core']).read_text().rstrip() — rstrip carries no cutset: a
    cutset would strip different bytes than the accepted compiler's core mode."""
    if not (isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
            and node.func.attr == "rstrip" and not node.args and not node.keywords):
        return False
    call = node.func.value
    return (isinstance(call, ast.Call) and isinstance(call.func, ast.Attribute)
            and call.func.attr == "read_text" and not call.args and not call.keywords
            and isinstance(call.func.value, ast.BinOp)
            and isinstance(call.func.value.op, ast.Div)
            and isinstance(call.func.value.left, ast.Name) and call.func.value.left.id == "ROOT"
            and isinstance(call.func.value.right, ast.Subscript)
            and isinstance(call.func.value.right.value, ast.Name)
            and call.func.value.right.value.id == "t"
            and _constant_str(call.func.value.right.slice) == "core")


def _is_section_read(node):
    """(ROOT / '<relative path>').read_text()"""
    return (isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
            and node.func.attr == "read_text" and not node.args and not node.keywords
            and isinstance(node.func.value, ast.BinOp)
            and isinstance(node.func.value.op, ast.Div)
            and isinstance(node.func.value.left, ast.Name) and node.func.value.left.id == "ROOT"
            and _constant_str(node.func.value.right) is not None)


def _is_local_read(node):
    """(ROOT / t['local']).read_text() — the harness record must come from the
    recognized config lookup variable t, not from an arbitrary object."""
    return (isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
            and node.func.attr == "read_text" and not node.args and not node.keywords
            and isinstance(node.func.value, ast.BinOp)
            and isinstance(node.func.value.op, ast.Div)
            and isinstance(node.func.value.left, ast.Name) and node.func.value.left.id == "ROOT"
            and isinstance(node.func.value.right, ast.Subscript)
            and isinstance(node.func.value.right.value, ast.Name)
            and node.func.value.right.value.id == "t"
            and _constant_str(node.func.value.right.slice) == "local")


def _is_docstring(node):
    return (isinstance(node, ast.Expr) and isinstance(node.value, ast.Constant)
            and isinstance(node.value.value, str))


def _plain_format(formatted):
    """No !conversion and no format spec: the formatted value inserts the
    operand itself, not a repr/str/format view of it."""
    if getattr(formatted, "conversion", -1) not in (-1, None) or formatted.format_spec is not None:
        raise fs.Held("legacy wrapper uses an unsupported format conversion or spec")


def _no_arg_call(node):
    return not node.args and not node.keywords


def _plain_parameters(fn, names, defaults=0):
    """Pin the complete recognized parameter binding: exactly these positional
    names in order with the stated number of literal-typed default slots, and
    no decorators, positional-only, keyword-only, vararg or kwarg surface.
    Anything else — a decorator that would rebind the function, an extra
    default, star-args — is outside the recognized grammar and HOLDs."""
    if fn.decorator_list:
        raise fs.Held("recognized legacy functions carry no decorators")
    args = fn.args
    if ([a.arg for a in args.posonlyargs] or [a.arg for a in args.args] != names
            or len(args.defaults) != defaults or args.vararg is not None
            or args.kwonlyargs or args.kwarg is not None):
        raise fs.Held("recognized legacy function parameters must be exactly "
                      f"({', '.join(names)}) with {defaults} default(s)")


def _match_guarded_fn(fn):
    """The wrapper must byte-match the accepted compiler's guarded chunk:
    f'<!-- BEGIN {name} -->\\n{body.rstrip()}\\n<!-- END {name} -->\\n' with
    plain format fields over the wrapper's own name/body parameters."""
    _plain_parameters(fn, ["name", "body"])
    body = [s for s in fn.body if not _is_docstring(s)]
    if len(body) != 1 or not isinstance(body[0], ast.Return) or not isinstance(body[0].value, ast.JoinedStr):
        raise fs.Held("legacy wrapper is not a single literal guard format string")
    values = body[0].value.values
    constants = [_constant_str(v) for v in values if isinstance(v, ast.Constant)]
    if constants != ["<!-- BEGIN ", " -->\n", "\n<!-- END ", " -->\n"]:
        raise fs.Held("legacy wrapper format does not match the accepted compiler guard")
    formatted = [v for v in values if isinstance(v, ast.FormattedValue)]
    if (len(formatted) != 3
            or not (isinstance(formatted[0].value, ast.Name) and formatted[0].value.id == "name")
            or not (isinstance(formatted[2].value, ast.Name) and formatted[2].value.id == "name")):
        raise fs.Held("legacy wrapper does not format the guard identity directly")
    for field in formatted:
        _plain_format(field)
    second = formatted[1].value
    # The wrapped body itself is inserted, rstripped with no arguments: a
    # different operand, a repr conversion or stripping with a cutset would
    # compile a different projection and must HOLD.
    if not (isinstance(second, ast.Call) and isinstance(second.func, ast.Attribute)
            and second.func.attr == "rstrip" and _no_arg_call(second)
            and isinstance(second.func.value, ast.Name) and second.func.value.id == "body"):
        raise fs.Held("legacy wrapper does not strip the wrapped body operand itself")
    return "<!-- BEGIN {name} -->\\n{body.rstrip()}\\n<!-- END {name} -->\\n"


def _match_extract_foreign(fn):
    _plain_parameters(fn, ["text", "tag"], defaults=1)
    tag = _constant_str(fn.args.defaults[-1]) if fn.args.defaults else None
    if not tag:
        raise fs.Held("legacy foreign-owner tag is not a literal default")
    compiler._tag(tag)
    stmts = [s for s in fn.body if not _is_docstring(s)]
    if len(stmts) != 2:
        raise fs.Held("unsupported legacy foreign-owner extractor shape")
    bind = stmts[0]
    if not (isinstance(bind, ast.Assign) and len(bind.targets) == 1
            and isinstance(bind.targets[0], ast.Name) and bind.targets[0].id == "m"
            and isinstance(bind.value, ast.Call)
            and isinstance(bind.value.func, ast.Attribute) and bind.value.func.attr == "search"
            and isinstance(bind.value.func.value, ast.Name) and bind.value.func.value.id == "re"
            and len(bind.value.args) >= 2):
        raise fs.Held("unsupported legacy foreign-owner extractor match shape")
    pattern = bind.value.args[0]
    pattern_values = pattern.values if isinstance(pattern, ast.JoinedStr) else []
    pattern_constants = [_constant_str(v) for v in pattern_values if isinstance(v, ast.Constant)]
    pattern_formatted = [v for v in pattern_values if isinstance(v, ast.FormattedValue)]
    # The recognized grammar pins the exact tagged-region regex operand: the
    # region delimiters and lazy body are grammar, the tag interpolation is the
    # only free operand. The trailing optional newline is the same regex atom
    # written either as a literal newline or as the \n escape; any other region
    # expression compiles a different projection and HOLDs.
    trailing_ok = (pattern_constants[2:] == [" -->\n?"]
                   or pattern_constants[2:] == [" -->\\n?"])
    if (len(pattern_constants) != 3 or pattern_constants[0] != "<!-- BEGIN "
            or pattern_constants[1] != " -->.*?<!-- END " or not trailing_ok
            or len(pattern_formatted) != 2
            or not all(isinstance(v.value, ast.Name) and v.value.id == "tag"
                       for v in pattern_formatted)):
        raise fs.Held("unsupported legacy foreign-owner extractor pattern: the recognized "
                      "grammar pins the exact tagged BEGIN/END region regex over tag")
    for value in pattern_formatted:
        _plain_format(value)
    text_arg = bind.value.args[1]
    # The recognized call is re.search(pattern, text, re.S): the text operand is
    # the extractor's own parameter, flags are the positional DOTALL attribute,
    # and no keyword flags exist in the grammar.
    if not (len(bind.value.args) == 3 and isinstance(text_arg, ast.Name)
            and text_arg.id == "text"):
        raise fs.Held("unsupported legacy foreign-owner extractor text operand: "
                      "the recognized grammar searches the text parameter")
    if (bind.value.keywords
            or not (isinstance(bind.value.args[2], ast.Attribute)
                    and isinstance(bind.value.args[2].value, ast.Name)
                    and bind.value.args[2].value.id == "re"
                    and bind.value.args[2].attr == "S")):
        raise fs.Held("unsupported legacy foreign-owner extractor flags: the recognized "
                      "grammar pins re.search(pattern, text, re.S) with no keyword flags")
    outcome = stmts[1]
    if not (isinstance(outcome, ast.Return) and isinstance(outcome.value, ast.IfExp)
            and isinstance(outcome.value.test, ast.Name) and outcome.value.test.id == "m"
            and isinstance(outcome.value.orelse, ast.Constant) and outcome.value.orelse.value is None
            and isinstance(outcome.value.body, ast.Call)
            and isinstance(outcome.value.body.func, ast.Attribute)
            and outcome.value.body.func.attr == "group"
            and isinstance(outcome.value.body.func.value, ast.Name)
            and outcome.value.body.func.value.id == "m"
            and len(outcome.value.body.args) == 1
            and isinstance(outcome.value.body.args[0], ast.Constant)
            and outcome.value.body.args[0].value == 0
            and not outcome.value.body.keywords):
        raise fs.Held("unsupported legacy foreign-owner extractor discards or rewrites the region")
    return tag


def _match_assemble(fn, constants):
    _plain_parameters(fn, ["harness", "live_text"])
    stmts = [s for s in fn.body if not _is_docstring(s)]
    if len(stmts) < 6:
        raise fs.Held("unsupported legacy assembly: too few statements")
    index = 0

    def take():
        nonlocal index
        node = stmts[index]
        index += 1
        return node

    node = take()
    # The harness record must be looked up from CFG with the assembler's own
    # harness parameter, bound to the recognized variable t.
    if not (isinstance(node, ast.Assign) and isinstance(node.value, ast.Subscript)
            and isinstance(node.value.value, ast.Name) and node.value.value.id == "CFG"
            and isinstance(node.value.slice, ast.Name) and node.value.slice.id == "harness"
            and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name)
            and node.targets[0].id == "t"):
        raise fs.Held("unsupported legacy assembly: harness config lookup not recognized")
    node = take()
    # The core literal must open into the recognized parts accumulator.
    if not (isinstance(node, ast.Assign) and isinstance(node.value, ast.List)
            and len(node.value.elts) == 1 and _is_core_read(node.value.elts[0])
            and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name)
            and node.targets[0].id == "parts"):
        raise fs.Held("unsupported legacy assembly: core literal opening not recognized")
    node = take()
    sections = []
    while _is_parts_append(node):
        call = node.value
        guarded_call = call.args[0] if len(call.args) == 1 else None
        if (call.keywords or guarded_call is None
                or not (isinstance(guarded_call, ast.Call) and isinstance(guarded_call.func, ast.Name)
                        and guarded_call.func.id == "guarded") or len(guarded_call.args) != 2
                or guarded_call.keywords):
            raise fs.Held("unsupported legacy assembly: unrecognized managed append")
        tag = compiler._tag(_guard_tag(guarded_call.args[0], constants))
        if not _is_section_read(guarded_call.args[1]):
            raise fs.Held("unsupported legacy assembly: section read is not a literal ROOT join")
        sections.append((tag, _constant_str(guarded_call.args[1].func.value.right)))
        if len(sections) > LIMIT_ASSEMBLY_APPENDS:
            raise fs.Held("legacy assembly exceeds the bounded section count")
        if index >= len(stmts):
            raise fs.Held("unsupported legacy assembly: missing foreign-owner slot")
        node = take()
    if not (isinstance(node, ast.If) and isinstance(node.test, ast.Call)
            and isinstance(node.test.func, ast.Attribute) and node.test.func.attr == "get"
            and isinstance(node.test.func.value, ast.Name) and node.test.func.value.id == "t"
            and len(node.test.args) == 1 and _constant_str(node.test.args[0]) == "local"
            and len(node.body) == 1 and node.orelse == []
            and _is_parts_append(node.body[0])):
        raise fs.Held("unsupported legacy assembly: local harness branch not recognized")
    inner = node.body[0].value.args[0]
    if (not (isinstance(inner, ast.Call) and isinstance(inner.func, ast.Name)
             and inner.func.id == "guarded") or len(inner.args) != 2 or inner.keywords
            or not _is_local_read(inner.args[1])):
        raise fs.Held("unsupported legacy assembly: local harness wrap not recognized")
    if _guard_tag(inner.args[0], constants) != constants["HARNESS_GUARD"]:
        raise fs.Held("unsupported legacy assembly: local harness wrap does not use the harness guard constant")
    node = take()
    # The foreign slot binds the recognized stamp variable: the extraction runs
    # over the assembler's own live_text parameter, guarded by the same
    # parameter, and an absent region yields None.
    if not (isinstance(node, ast.Assign) and isinstance(node.value, ast.IfExp)
            and len(node.targets) == 1 and isinstance(node.targets[0], ast.Name)
            and node.targets[0].id == "stamp"
            and isinstance(node.value.test, ast.Name) and node.value.test.id == "live_text"
            and isinstance(node.value.body, ast.Call)
            and isinstance(node.value.body.func, ast.Name)
            and node.value.body.func.id == "extract_foreign"
            and not node.value.body.keywords and len(node.value.body.args) == 1
            and isinstance(node.value.body.args[0], ast.Name)
            and node.value.body.args[0].id == "live_text"
            and isinstance(node.value.orelse, ast.Constant) and node.value.orelse.value is None):
        raise fs.Held("unsupported legacy assembly: foreign-owner extraction not recognized")
    node = take()
    # The append fires only on the extracted stamp, and the else branch is the
    # permitted single warning: a print of one static string message to stderr.
    # Any other else body — a policy append, a file write, extra statements —
    # implements a different projection and HOLDs (it is also never executed:
    # the adapter reads structure only).
    if not (isinstance(node, ast.If) and isinstance(node.test, ast.Name)
            and node.test.id == "stamp"):
        raise fs.Held("unsupported legacy assembly: foreign-owner slot branch not recognized")
    if len(node.body) != 1 or not _is_parts_append(node.body[0]):
        raise fs.Held("unsupported legacy assembly: foreign-owner append not recognized")
    if len(node.orelse) != 1:
        raise fs.Held("unsupported legacy assembly: the foreign-owner else branch "
                      "is not a single warning statement")
    warning = node.orelse[0]
    message = warning.value.args[0] if (isinstance(warning, ast.Expr)
                                        and isinstance(warning.value, ast.Call)
                                        and warning.value.args) else None
    message_ok = isinstance(message, (ast.Constant, ast.JoinedStr)) and (
        not isinstance(message, ast.Constant) or isinstance(message.value, str))
    if not (isinstance(warning, ast.Expr) and isinstance(warning.value, ast.Call)
            and isinstance(warning.value.func, ast.Name) and warning.value.func.id == "print"
            and message_ok and len(warning.value.keywords) == 1
            and warning.value.keywords[0].arg == "file"
            and isinstance(warning.value.keywords[0].value, ast.Attribute)
            and isinstance(warning.value.keywords[0].value.value, ast.Name)
            and warning.value.keywords[0].value.value.id == "sys"
            and warning.value.keywords[0].value.attr == "stderr"):
        raise fs.Held("unsupported legacy assembly: foreign-owner else branch is not the "
                      "permitted print(message, file=sys.stderr) warning")
    appended = node.body[0].value.args[0]
    if not (len(node.body[0].value.args) == 1
            and isinstance(appended, ast.BinOp) and isinstance(appended.op, ast.Add)
            and isinstance(appended.left, ast.Call)
            and isinstance(appended.left.func, ast.Attribute)
            and appended.left.func.attr == "rstrip"
            and isinstance(appended.left.func.value, ast.Name)
            and appended.left.func.value.id == "stamp"
            and not appended.left.args and not appended.left.keywords
            and _constant_str(appended.right) == "\n"):
        raise fs.Held("unsupported legacy assembly: foreign-owner append does not "
                      "stamp the extracted region verbatim")
    node = take()
    if not (isinstance(node, ast.Return) and isinstance(node.value, ast.Call)
            and isinstance(node.value.func, ast.Attribute) and node.value.func.attr == "join"
            and _constant_str(node.value.func.value) == "\n\n"
            and isinstance(node.value.args[-1], ast.Name) and node.value.args[-1].id == "parts"):
        raise fs.Held("unsupported legacy assembly: join is not two newlines over parts")
    if index != len(stmts):
        raise fs.Held("unsupported legacy assembly: trailing statements are not recognized")
    return {"sections": sections}


def _contains_toml_load(node):
    if isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute):
        if node.func.attr in ("loads", "load"):
            for sub in ast.walk(node):
                if _constant_str(sub) == TARGETS_NAME:
                    return True
    return any(_contains_toml_load(child) for child in ast.iter_child_nodes(node))


def _extract_adapter(text):
    try:
        tree = ast.parse(text)
    except SyntaxError as exc:
        raise fs.Held(f"legacy steer is not parseable Python ({exc.msg})") from exc
    constants, has_root, has_cfg = {}, False, False
    wrapper, foreign_tag, assemble = None, None, None
    for node in tree.body:
        if isinstance(node, ast.Assign):
            for target in node.targets:
                if isinstance(target, ast.Name):
                    if target.id in ("DOOR_GUARD", "HARNESS_GUARD"):
                        value = _constant_str(node.value)
                        if value is None:
                            raise fs.Held(f"legacy {target.id} is not a literal guard constant")
                        constants[target.id] = compiler._tag(value)
                    elif target.id == "ROOT":
                        has_root = True
                    elif target.id == "CFG":
                        has_cfg = _contains_toml_load(node.value)
        elif isinstance(node, ast.FunctionDef):
            if node.name == "guarded":
                wrapper = _match_guarded_fn(node)
            elif node.name == "extract_foreign":
                foreign_tag = _match_extract_foreign(node)
            elif node.name == "assemble":
                assemble = node
    if not has_root or not has_cfg:
        raise fs.Held("legacy steer does not bind ROOT and a targets.toml CFG loader")
    if set(constants) != {"DOOR_GUARD", "HARNESS_GUARD"}:
        raise fs.Held("legacy steer lacks the expected guard constants")
    if wrapper is None:
        raise fs.Held("legacy steer has no recognizable guarded wrapper")
    if foreign_tag is None:
        raise fs.Held("legacy steer has no recognizable foreign-owner extractor")
    if assemble is None:
        raise fs.Held("legacy steer has no assemble function")
    matched = _match_assemble(assemble, constants)
    return {"kind": ADAPTER_KIND, "guard_constants": constants,
            "sections": matched["sections"], "foreign_tag": foreign_tag, "wrapper": wrapper}


def _canonical_target(raw):
    if not isinstance(raw, str) or not raw.strip():
        raise fs.Held("legacy target is not a path string")
    expanded = PurePath(os.path.expanduser(raw))
    if not expanded.is_absolute():
        raise fs.Held("legacy target must be home-anchored (~)")
    try:
        relative = expanded.relative_to(PurePath(os.path.expanduser("~")))
    except ValueError:
        raise fs.Held("legacy target outside the home directory cannot be canonicalized") from None
    return compiler._relative(relative.as_posix()).as_posix()


def plan_import(legacy_root):
    """Read-only classification of a legacy root-steer checkout. Writes nothing."""
    try:
        root = Path(legacy_root).resolve(strict=True)
    except OSError as exc:
        raise fs.Held(f"legacy root cannot be resolved ({type(exc).__name__})") from exc
    if not root.is_dir():
        raise fs.Held("legacy root is not a directory")
    tree = _scan_tree(root)
    toml_raw, toml_meta = _read_source(root, TARGETS_NAME, LIMIT_TOML_BYTES)
    try:
        doc = tomllib.loads(toml_raw.decode("utf-8"))
    except (UnicodeDecodeError, ValueError) as exc:
        raise fs.Held(f"legacy {TARGETS_NAME} is not valid TOML ({type(exc).__name__})") from exc
    steer_raw, steer_meta = _read_source(root, STEER_NAME, LIMIT_STEER_BYTES)
    adapter = _extract_adapter(steer_raw.decode("utf-8"))

    unknown, hold_items, harnesses = [], [], {}
    placement = None
    for key, value in doc.items():
        if key == "hosts":
            if not isinstance(value, dict):
                hold_items.append({"scope": "placement", "detail": "hosts table is not a mapping"})
            else:
                placement = {"observed_keys": sorted(value),
                             "authority": "observation-only private placement data; "
                                          "not authority, not compiled or staged"}
            continue
        if not isinstance(value, dict):
            unknown.append({"scope": "top-level", "field": key})
            continue
        fields = set(value) - KNOWN_HARNESS_FIELDS
        if fields:
            unknown.append({"scope": "harness", "harness": key, "fields": sorted(fields)})
        harnesses[key] = dict(value)
    for name, entry in harnesses.items():
        record = {"target_raw": entry.get("target")}
        try:
            record["target_relative"] = _canonical_target(entry.get("target"))
        except fs.Held as exc:
            record["target_relative"] = None
            hold_items.append({"scope": "harness", "harness": name, "detail": str(exc)})
        record["core"] = entry.get("core")
        record["local"] = entry.get("local") or None
        harnesses[name] = record

    declared = {TARGETS_NAME, STEER_NAME}
    for entry in harnesses.values():
        for key in ("core", "local"):
            if isinstance(entry.get(key), str) and entry[key]:
                declared.add(entry[key])
    declared.update(path for _, path in adapter["sections"])
    missing = sorted(p for p in declared if tree.get(p) != "file")
    if missing:
        raise fs.Held(f"declared legacy sources are absent or not plain files: {missing[:8]}")

    sources = {TARGETS_NAME: toml_meta, STEER_NAME: steer_meta}
    for rel in sorted(declared - {TARGETS_NAME, STEER_NAME}):
        _, sources[rel] = _read_source(root, rel, LIMIT_PART_BYTES)
    sources = {rel: {"sha256": meta["sha256"], "bytes": meta["bytes"], "owner": meta["owner"]}
               for rel, meta in sorted(sources.items())}

    unclassified, git_history = [], []
    declared_prefixes = tuple(name + "/" for name in declared)
    for rel, kind in sorted(tree.items()):
        if rel in declared or rel.startswith(declared_prefixes):
            continue
        if rel == ".git" or rel.startswith(".git/"):
            git_history.append({"path": rel, "kind": kind,
                                "disposition": "parent history transaction; archived; "
                                               "importer claims no retirement"})
            continue
        unclassified.append({"path": rel, "kind": kind})

    return {"schema": SCHEMA, "importer_version": IMPORTER_VERSION,
            "adapter": {"kind": adapter["kind"], "steer_sha256": steer_meta["sha256"]},
            "targets_toml_sha256": toml_meta["sha256"],
            "guard_constants": adapter["guard_constants"],
            "wrapper": adapter["wrapper"],
            "sections": [{"guard": tag, "source": path} for tag, path in adapter["sections"]],
            "foreign": {"tag": adapter["foreign_tag"], "required": False,
                        "note": "the legacy tool warned and left the slot empty when absent; "
                                "required=True remains available to the caller and HOLDs through the compiler"},
            "harnesses": harnesses, "sources": sources,
            "legacy_root_resolved": str(root),
            "placement": placement, "unknown": unknown, "hold_items": hold_items,
            "unclassified": unclassified,
            "blockers": [row["path"] for row in unclassified],
            "delete_eligible": [],
            "git_history": git_history, "sealed": False,
            "notes": ["plan is proposal-only metadata; no policy activation, target stamp, "
                      "host sync, deletion or complete-retirement claim is made"]}


def build_manifest(plan, harnesses=None):
    """Derive the accepted compiler manifest from a plan. Refuses to claim
    harnesses that carry unknown fields or hold items."""
    if plan.get("schema") != SCHEMA or plan.get("adapter", {}).get("kind") != ADAPTER_KIND:
        raise fs.Held("plan is not a recognized importer revision")
    hold_scopes = {row.get("harness") for row in plan.get("hold_items", []) if row.get("scope") == "harness"}
    unknown_scopes = {row.get("harness") for row in plan.get("unknown", []) if row.get("scope") == "harness"}
    if any(row.get("scope") != "harness" for row in plan.get("hold_items", [])):
        raise fs.Held("plan carries non-harness hold items; nothing may be staged")
    selected = sorted(plan["harnesses"]) if harnesses is None else sorted(harnesses)
    targets = {}
    for name in selected:
        entry = plan["harnesses"].get(name)
        if entry is None:
            raise fs.Held(f"unknown legacy harness {name}")
        if name in hold_scopes or name in unknown_scopes:
            raise fs.Held(f"harness {name} carries unknown fields or hold items; refusing to compile")
        if not entry.get("target_relative"):
            raise fs.Held(f"harness {name} has no canonical target relative path")
        core = entry.get("core")
        if not isinstance(core, str) or core not in plan["sources"]:
            raise fs.Held(f"harness {name} core is not a planned source")
        parts = [{"source": core, "sha256": plan["sources"][core]["sha256"], "mode": "literal"}]
        for row in plan["sections"]:
            source = row["source"]
            if source not in plan["sources"]:
                raise fs.Held(f"section source is not a planned source: {source}")
            parts.append({"source": source, "sha256": plan["sources"][source]["sha256"],
                          "mode": "guarded", "guard": row["guard"]})
        local = entry.get("local")
        if local:
            if local not in plan["sources"]:
                raise fs.Held(f"harness {name} local is not a planned source")
            parts.append({"source": local, "sha256": plan["sources"][local]["sha256"],
                          "mode": "guarded", "guard": plan["guard_constants"]["HARNESS_GUARD"]})
        targets[name] = {"target": entry["target_relative"], "parts": parts,
                         "preserve": [{"tag": plan["foreign"]["tag"], "required": False}]}
    return {"schema": 1, "compiler_version": 1, "targets": targets}


def compile_preview(legacy_root, manifest, harness, live=b""):
    """Proposal-only projection through the accepted compiler. Writes nothing."""
    return compiler.compile_projection(Path(legacy_root), manifest, harness, live)


def _descriptor(plan, manifest):
    return {"schema": SCHEMA, "candidate_schema": 1,
            "adapter_kind": plan["adapter"]["kind"],
            "steer_sha256": plan["adapter"]["steer_sha256"],
            "targets_toml_sha256": plan["targets_toml_sha256"],
            "manifest": manifest}


def candidate_identity(plan, manifest):
    """Location-independent candidate ID: schema, source bytes, relative paths."""
    return sha(canonical(_descriptor(plan, manifest)))


def _fsync_directory(path):
    fd = os.open(str(path), os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def _fsync_tree(root):
    for base, dirs, _files in os.walk(root, topdown=False):
        for name in dirs:
            _fsync_directory(Path(base) / name)
    _fsync_directory(root)


_RENAME_NOREPLACE = 1  # Linux renameat2 flag: fail with EEXIST instead of replacing
_RENAMEAT2_SYSCALL = {"x86_64": 316, "aarch64": 276, "armv7l": 382, "i686": 353, "ppc64le": 357, "s390x": 347}
_libc = None


def _rename_no_replace(src_dir_fd, src_name, dst_dir_fd, dst_name):
    """Linux renameat2(RENAME_NOREPLACE): rename only when the destination is absent.

    Returns True on rename, False when the destination exists (EEXIST/ENOTEMPTY).
    Raises fs.Held on platforms without a typed renameat2 route — never falls
    back to replacing os.rename, and the claim is Linux-scoped, not portable.
    """
    global _libc
    if sys.platform != "linux":
        raise fs.Held("atomic no-replace publication needs Linux renameat2; "
                      f"platform {sys.platform!r} is unsupported for candidate staging")
    src, dst = os.fsencode(src_name), os.fsencode(dst_name)
    if _libc is None:
        try:
            _libc = ctypes.CDLL(None, use_errno=True)
        except OSError as exc:
            raise fs.Held(f"renameat2 unavailable ({type(exc).__name__})") from exc
    if hasattr(_libc, "renameat2"):
        _libc.renameat2.restype = ctypes.c_int
        _libc.renameat2.argtypes = [ctypes.c_int, ctypes.c_char_p,
                                    ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
        result = _libc.renameat2(src_dir_fd, src, dst_dir_fd, dst, _RENAME_NOREPLACE)
    else:
        number = _RENAMEAT2_SYSCALL.get(platform.machine())
        if number is None:
            raise fs.Held(f"renameat2 has no typed syscall route for machine "
                          f"{platform.machine()!r}; staging HOLDs rather than replacing")
        _libc.syscall.restype = ctypes.c_long
        result = _libc.syscall(ctypes.c_long(number), ctypes.c_int(src_dir_fd), src,
                               ctypes.c_int(dst_dir_fd), dst, ctypes.c_uint(_RENAME_NOREPLACE))
    if result == 0:
        return True
    code = ctypes.get_errno()
    if code in (errno.ENOTEMPTY, errno.EEXIST):
        return False
    if code in (errno.ENOSYS, errno.EINVAL, errno.EOPNOTSUPP):
        raise fs.Held("Linux renameat2(RENAME_NOREPLACE) is not supported by this kernel "
                      f"(errno {code}); candidate staging HOLDs instead of replacing")
    raise OSError(code, os.strerror(code), os.fsdecode(dst))


class _Staging:
    """Private temp directory that publishes atomically or not at all."""

    def __init__(self, parent_fd, candidates_path, name):
        self.name = name
        self.fsync_error = None
        os.mkdir(name, 0o700, dir_fd=parent_fd)
        self.path = Path(candidates_path) / name

    def put(self, relative, data):
        target = self.path / relative
        with fs._directory(target.parent, create=True) as parent:
            fd = os.open(target.name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                         0o600, dir_fd=parent)
            try:
                os.fchmod(fd, 0o600)
                offset = 0
                while offset < len(data):
                    offset += os.write(fd, data[offset:])
                os.fsync(fd)
            finally:
                os.close(fd)

    def publish(self, parent_fd, candidate_id):
        """Rename into place with no-replace semantics.

        Returns False when the destination is occupied (nothing written there).
        After a successful rename the candidate is published; a parent-directory
        fsync failure cannot undo it, so the error is recorded on the instance
        and reported truthfully as a durability uncertainty instead of being
        raised as a false failure claim.
        """
        _fsync_tree(self.path)
        if not _rename_no_replace(parent_fd, self.name, parent_fd, candidate_id):
            return False
        try:
            os.fsync(parent_fd)
        except OSError as exc:
            self.fsync_error = exc
        return True

    def discard(self):
        shutil.rmtree(self.path, ignore_errors=True)


def _validate_existing(candidate_dir, candidate_id):
    """Replay: a complete candidate must match its sealed marker AND the
    candidate identity must be recomputed from the staged bytes, never trusted
    from the caller's in-memory plan or from the marker alone.

    The mandatory file inventory and every source digest derive from the
    descriptor-bound manifest — the self-declared marker cannot add, drop or
    rehash a declared source. The on-disk tree must be exactly the sealed
    inventory plus the marker: no unlisted foreign policy file, no missing or
    changed payload, no symlink standing in for a staged file."""
    marker_raw, _ = _read_source(candidate_dir, "complete.json", LIMIT_JSON_BYTES)
    try:
        marker = json.loads(marker_raw.decode("utf-8"))
    except (UnicodeDecodeError, ValueError) as exc:
        raise fs.Held(f"existing candidate marker is unreadable ({type(exc).__name__})") from exc
    if (not isinstance(marker, dict) or marker.get("schema") != 1
            or marker.get("candidate_id") != candidate_id or marker.get("sealed") is not True):
        raise fs.Held("existing candidate identity disagrees with its directory")
    files = marker.get("files")
    if not isinstance(files, dict) or not files:
        raise fs.Held("existing candidate has no sealed file list")
    manifest_raw, _ = _read_source(candidate_dir, "manifest.json", LIMIT_JSON_BYTES)
    if sha(manifest_raw) != marker.get("manifest_sha256"):
        raise fs.Held("existing candidate manifest differs from its sealed digest")
    descriptor_raw, _ = _read_source(candidate_dir, "descriptor.json", LIMIT_JSON_BYTES)
    if sha(descriptor_raw) != marker.get("descriptor_sha256"):
        raise fs.Held("existing candidate descriptor differs from its sealed digest")
    # Content identity recomputed from the staged descriptor: the directory
    # name must be the hash of the descriptor bytes actually on disk.
    if sha(descriptor_raw) != candidate_id:
        raise fs.Held("existing candidate descriptor does not hash to the candidate "
                      "identity; staged content id was rewritten")
    try:
        descriptor = json.loads(descriptor_raw.decode("utf-8"))
    except (UnicodeDecodeError, ValueError) as exc:
        raise fs.Held(f"existing candidate descriptor is unreadable ({type(exc).__name__})") from exc
    if not isinstance(descriptor, dict) or descriptor.get("schema") != SCHEMA:
        raise fs.Held("existing candidate descriptor is not a recognized importer revision")
    # Manifest binding: the staged manifest bytes must be exactly the manifest
    # bound inside the staged descriptor.
    if canonical(descriptor.get("manifest")) != manifest_raw:
        raise fs.Held("existing candidate manifest is not the descriptor's bound manifest")
    # Mandatory inventory from the descriptor-bound manifest: every declared
    # policy source plus the sealed metadata. A marker that omits, adds or
    # rehashes any of them does not describe this candidate.
    bound_manifest = descriptor.get("manifest")
    declared_sources = sorted({part["source"]
                               for target in bound_manifest.get("targets", {}).values()
                               for part in target.get("parts", [])})
    required = {"manifest.json", "descriptor.json", "provenance.json"} | {
        "sources/" + rel for rel in declared_sources}
    if set(files) != required:
        raise fs.Held(f"existing candidate sealed inventory disagrees with the descriptor-bound "
                      f"manifest (missing from marker: {sorted(required - set(files))[:8]}; "
                      f"unlisted in marker: {sorted(set(files) - required)[:8]})")
    expected = {rel: "file" for rel in sorted(required | {"complete.json"})}
    for rel in sorted(required):
        for ancestor in list(PurePath(rel).parents)[:-1]:
            expected[ancestor.as_posix()] = "directory"
    try:
        actual = _scan_tree(candidate_dir)
    except fs.Held as exc:
        raise fs.Held(f"existing candidate tree is not scannable: {exc}") from exc
    if actual != expected:
        raise fs.Held(f"existing candidate directory does not match its sealed inventory "
                      f"(extra entries: {sorted(set(actual) - set(expected))[:8]}; "
                      f"absent entries: {sorted(set(expected) - set(actual))[:8]}; "
                      f"wrong kinds: {sorted(rel for rel in set(actual) & set(expected)
                                             if actual[rel] != expected[rel])[:8]})")
    # Byte validation of every sealed file against its marker digest, then the
    # descriptor binding for the declared sources: an independently rehashed
    # marker entry cannot legitimize changed payload bytes.
    for relative, digest in sorted(files.items()):
        try:
            raw, _meta = _read_source(candidate_dir, relative, LIMIT_PART_BYTES)
        except fs.Held as exc:
            raise fs.Held(f"existing candidate file is unreadable: {relative} ({exc})") from exc
        if sha(raw) != digest:
            raise fs.Held(f"existing candidate file differs from its sealed digest: {relative}")
    for target in bound_manifest.get("targets", {}).values():
        for part in target.get("parts", []):
            relative = "sources/" + part["source"]
            if files.get(relative) != part["sha256"]:
                raise fs.Held(f"existing candidate marker digest was rewritten for a "
                              f"descriptor-bound source: {relative}")
    return {"manifest_sha256": sha(manifest_raw), "descriptor_sha256": sha(descriptor_raw),
            "descriptor": descriptor}


def _retry_parent_fsync(candidates_path):
    """Recovery contract for a published candidate: retry the candidates-parent
    fsync on every replay and report the outcome truthfully.

    Returns (durable, error). A failure is never laundered: the rename already
    committed, so the candidate is published and visible while its rename
    durability stays unconfirmed until a replay sync succeeds."""
    try:
        _fsync_directory(candidates_path)
        return True, None
    except OSError as exc:
        return False, exc


def _policy_source_drift_gate(plan, manifest):
    """Re-read every identity-bound selected policy source and compare it with
    the manifest digest before anything is staged or replayed. This gate runs
    before the existing-candidate early return, so replay can never accept a
    stale plan against drifted legacy sources."""
    bound = {}
    for target in manifest["targets"].values():
        for part in target["parts"]:
            bound.setdefault(part["source"], part["sha256"])
    for relative, digest in sorted(bound.items()):
        raw, _meta = _read_source(plan["legacy_root_resolved"], relative, LIMIT_PART_BYTES)
        if sha(raw) != digest:
            raise fs.Held(f"legacy source changed since the plan was made: {relative}")


def stage_candidate(plan, manifest, *, home=None, env=None, root=None):
    """Stage sealed sources into <initialized root>/steer/candidates/<id>/.

    Idempotent on replay, never overwrites a different candidate, and leaves
    no complete marker behind a partial copy. Requires an initialized root via
    the shared resolver; no genesis and no foreign-root bypass.
    """
    if plan.get("legacy_root_resolved") is None:
        raise fs.Held("plan does not record a resolved legacy root")
    selected = fs.resolve(home=home, env=env, explicit=root)
    if not selected.get("initialized"):
        raise fs.Held("initialize the root identity before staging legacy candidates")
    # Metadata drift gate: every assembler/TOML dependency bound by the plan is
    # re-read before anything is staged or replayed; the staged descriptor must
    # never claim an old source revision against changed legacy metadata.
    for relative, bound, limit in ((STEER_NAME, plan["adapter"]["steer_sha256"], LIMIT_STEER_BYTES),
                                   (TARGETS_NAME, plan["targets_toml_sha256"], LIMIT_TOML_BYTES)):
        raw, _meta = _read_source(plan["legacy_root_resolved"], relative, limit)
        if sha(raw) != bound:
            raise fs.Held(f"legacy {relative} changed since the plan was made; replan before staging")
    # Policy-source drift gate: the same staleness refusal for every selected
    # Markdown source, enforced before the existing-candidate early return below.
    _policy_source_drift_gate(plan, manifest)
    candidates_path = Path(selected["physical_path"]) / "steer" / "candidates"
    candidate_id = candidate_identity(plan, manifest)
    manifest_bytes = canonical(manifest)
    with fs._directory(candidates_path, create=True) as candidates_fd:
        existing = candidates_path / candidate_id
        if existing.is_dir() and not existing.is_symlink():
            stored = _validate_existing(existing, candidate_id)
            durable, sync_error = _retry_parent_fsync(candidates_path)
            result = {"candidate_id": candidate_id, "path": str(existing.resolve()),
                      "published": False, "validated": True, "sealed": True,
                      "durable": durable,
                      "manifest_sha256": stored["manifest_sha256"]}
            if sync_error is not None:
                result["fsync_error"] = (f"errno {sync_error.errno}: the candidates-parent fsync "
                                         "still fails during replay; the candidate is published "
                                         "and validated, but its rename durability remains "
                                         "unconfirmed (recovery contract)")
            result["status"] = ("VISIBLE_DURABILITY_UNKNOWN" if not durable
                                else "DURABLE_CANDIDATE")
            return result
        temp_name = ".staging-" + uuid.uuid4().hex
        staging = _Staging(candidates_fd, candidates_path, temp_name)
        try:
            staging.put("manifest.json", manifest_bytes)
            descriptor_bytes = canonical(_descriptor(plan, manifest))
            staging.put("descriptor.json", descriptor_bytes)
            provenance_bytes = canonical({
                "schema": SCHEMA, "importer_version": IMPORTER_VERSION,
                "observed_at": dt.datetime.now(dt.timezone.utc).isoformat(),
                "legacy_root": plan["legacy_root_resolved"],
                "ygg_root_physical": selected["physical_path"],
                "note": "provenance paths are deliberately outside the candidate identity"})
            staging.put("provenance.json", provenance_bytes)
            sealed_files = {"manifest.json": sha(manifest_bytes),
                            "descriptor.json": sha(descriptor_bytes),
                            "provenance.json": sha(provenance_bytes)}
            staged_sources = set()
            for name, target in sorted(manifest["targets"].items()):
                for part in target["parts"]:
                    relative = part["source"]
                    if relative in staged_sources:
                        continue
                    raw, _meta = _read_source(plan["legacy_root_resolved"], relative, LIMIT_PART_BYTES)
                    if sha(raw) != part["sha256"]:
                        raise fs.Held(f"legacy source changed since the plan was made: {relative}")
                    staging.put("sources/" + relative, raw)
                    sealed_files["sources/" + relative] = part["sha256"]
                    staged_sources.add(relative)
            staging.put("complete.json", canonical({
                "schema": 1, "candidate_id": candidate_id, "sealed": True,
                "manifest_sha256": sha(manifest_bytes),
                "descriptor_sha256": sha(descriptor_bytes),
                "files": sealed_files}))
            published = staging.publish(candidates_fd, candidate_id)
            fsync_error = staging.fsync_error
            if not published:
                staging.discard()
        except BaseException:
            staging.discard()
            raise
    final = candidates_path / candidate_id
    stored = None
    durable, sync_error = True, None
    if not published:
        try:
            stored = _validate_existing(final, candidate_id)
        except fs.Held as exc:
            raise fs.Held("candidate destination is occupied by an unrecognized or "
                          f"incomplete directory (preserved in place): {exc}") from exc
        durable, sync_error = _retry_parent_fsync(candidates_path)
    result = {"candidate_id": candidate_id, "path": str(final.resolve()),
              "published": bool(published), "validated": not published, "sealed": True,
              "durable": bool(durable),
              "manifest_sha256": sha(manifest_bytes) if published
              else stored["manifest_sha256"]}
    if not published and sync_error is not None:
        result["fsync_error"] = (f"errno {sync_error.errno}: the candidates-parent fsync "
                                 "still fails during replay; the candidate is published "
                                 "and validated, but its rename durability remains "
                                 "unconfirmed (recovery contract)")
    if published and fsync_error is not None:
        result["durable"] = False
        result["fsync_error"] = (f"errno {fsync_error.errno}: the parent-directory fsync failed "
                                 "after the rename; the candidate is published and visible, but "
                                 "rename durability is unconfirmed. Re-running stage replays and "
                                 "validates the existing candidate (recovery contract).")
    result["status"] = ("VISIBLE_DURABILITY_UNKNOWN" if result.get("durable") is False
                        else "DURABLE_CANDIDATE")
    return result


def cli(argv=None):
    parser = argparse.ArgumentParser(prog="ygg steer import-legacy (unwired)",
                                     description="Read-only legacy steering importer: plan, manifest, preview, stage")
    parser.add_argument("--root", help="exact Ygg root override (initialized)")
    parser.add_argument("--legacy", required=True, help="legacy root-steer checkout path")
    sub = parser.add_subparsers(dest="verb", required=True)
    plan_p = sub.add_parser("plan")
    plan_p.add_argument("--json", action="store_true")
    manifest_p = sub.add_parser("manifest")
    manifest_p.add_argument("--harness", action="append", default=[])
    manifest_p.add_argument("--json", action="store_true")
    preview = sub.add_parser("preview")
    preview.add_argument("--harness", required=True)
    preview.add_argument("--live", help="optional live target snapshot file")
    preview.add_argument("--json", action="store_true")
    stage_p = sub.add_parser("stage")
    stage_p.add_argument("--harness", action="append", default=[])
    stage_p.add_argument("--home", help="explicit HOME for the root resolver")
    stage_p.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)
    try:
        plan = plan_import(args.legacy)
        if args.verb == "plan":
            result = plan
        else:
            manifest = build_manifest(plan, args.harness or None)
            if args.verb == "manifest":
                result = manifest
            elif args.verb == "preview":
                live = Path(args.live).read_bytes() if args.live else b""
                result = compile_preview(args.legacy, manifest, args.harness, live)
            else:
                result = stage_candidate(plan, manifest, home=args.home, env=None, root=args.root)
        payload = dict(result)
        if isinstance(payload.get("output"), bytes):
            payload["output"] = payload["output"].decode("utf-8", "replace")
        if args.json or args.verb == "manifest":
            sys.stdout.write(json.dumps(payload, indent=2, sort_keys=True, default=repr) + "\n")
        elif args.verb == "stage":
            print(f"candidate {result['candidate_id'][:12]} "
                  f"{'published' if result['published'] else 'already sealed (replay validated)'} "
                  f"at {result['path']}")
            if result.get("durable") is False:
                print(f"  WARNING durability unconfirmed: {result['fsync_error']}", file=sys.stderr)
            print(f"  blockers preserved in place: {len(plan['unclassified'])} unclassified files; "
                  "no deletion eligibility claimed")
        elif args.verb == "preview":
            print(result["output"].decode("utf-8", "replace"), end="")
            print(f"[proposal-only harness={result['harness']} output_digest={result['output_digest']}]",
                  file=sys.stderr)
        else:
            print(f"legacy plan: {len(plan['harnesses'])} harnesses, {len(plan['sections'])} sections, "
                  f"{len(plan['blockers'])} unclassified blockers preserved in place, "
                  f"{len(plan['unknown'])} unknown items, {len(plan['hold_items'])} hold items")
            print(f"  foreign-owner slot: {plan['foreign']['tag']} (required=False)")
        return 75 if args.verb == "stage" and result.get("durable") is False else 0
    except (fs.Held, OSError, ValueError) as exc:
        print(f"ygg steer import-legacy: HOLD — {exc}", file=sys.stderr)
        return 75


if __name__ == "__main__":
    raise SystemExit(cli())
