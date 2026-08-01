"""Policy test: python/ must not use anything newer than the declared floor.

``requires-python`` in ``pyproject.toml`` is a promise made to every installer
that resolves this package, and it is the one promise CI structurally cannot
keep: the workflow pins a single interpreter, so a 3.11-only API compiles,
imports, and passes every other test while the package is undeclarably broken
on the versions it claims. (``from enum import StrEnum`` under a ``>=3.9``
floor was exactly that: an unconditional ``ImportError`` on both 3.9 and 3.10.)

The gate is therefore static, not a runtime probe -- it holds on whatever
interpreter happens to run the suite, and it stays honest under PEP 649, where
even annotations are no longer evaluated eagerly:

1. **Syntax** -- reparse each file with ``ast.parse(feature_version=floor)``,
   which rejects ``match``, ``except*``, PEP 695 type parameters, and friends.
2. **Stdlib API** -- match imports, module attribute accesses, call keywords,
   and distinctive method names against a version table.
3. **PEP 604 annotations** -- ``X | Y`` in a runtime-evaluated annotation needs
   3.10 unless the module opts into ``from __future__ import annotations``.

The version table is curated, not exhaustive: a static gate cannot know every
stdlib addition. Add an entry when python/ starts depending on a new API. What
the gate does guarantee is that the common ways of drifting past the floor fail
here rather than in a user's install.
"""

import ast
import re
import sys
from pathlib import Path

PYTHON_ROOT = Path(__file__).resolve().parents[1]
PYPROJECT = PYTHON_ROOT / "pyproject.toml"
PACKAGE_DIRS = ("agent_first_data", "tests")

REQUIRES_PYTHON = re.compile(
    r"""^\s*requires-python\s*=\s*["']>=\s*(\d+)\.(\d+)\s*["']""", re.MULTILINE
)

# Dotted stdlib paths that only exist from the given version. Matched against
# `import a.b`, `from a import b`, and `a.b` where `a` is an imported module.
MIN_VERSION_BY_SYMBOL = {
    # 3.10
    "contextlib.aclosing": (3, 10),
    "itertools.pairwise": (3, 10),
    "types.EllipsisType": (3, 10),
    "types.NoneType": (3, 10),
    "types.UnionType": (3, 10),
    "typing.Concatenate": (3, 10),
    "typing.ParamSpec": (3, 10),
    "typing.TypeAlias": (3, 10),
    "typing.TypeGuard": (3, 10),
    # 3.11
    "asyncio.Runner": (3, 11),
    "asyncio.TaskGroup": (3, 11),
    "asyncio.timeout": (3, 11),
    "contextlib.chdir": (3, 11),
    "datetime.UTC": (3, 11),
    "enum.EnumCheck": (3, 11),
    "enum.ReprEnum": (3, 11),
    "enum.StrEnum": (3, 11),
    "enum.global_enum": (3, 11),
    "enum.member": (3, 11),
    "enum.nonmember": (3, 11),
    "enum.verify": (3, 11),
    "hashlib.file_digest": (3, 11),
    "math.cbrt": (3, 11),
    "math.exp2": (3, 11),
    "operator.call": (3, 11),
    "re.NOFLAG": (3, 11),
    "tomllib": (3, 11),
    "typing.LiteralString": (3, 11),
    "typing.Never": (3, 11),
    "typing.NotRequired": (3, 11),
    "typing.Required": (3, 11),
    "typing.Self": (3, 11),
    "typing.TypeVarTuple": (3, 11),
    "typing.Unpack": (3, 11),
    "typing.assert_never": (3, 11),
    "typing.assert_type": (3, 11),
    "typing.dataclass_transform": (3, 11),
    "typing.reveal_type": (3, 11),
    "wsgiref.types": (3, 11),
    # 3.12
    "calendar.Month": (3, 12),
    "itertools.batched": (3, 12),
    "sys.monitoring": (3, 12),
    "types.get_original_bases": (3, 12),
    "typing.TypeAliasType": (3, 12),
    "typing.override": (3, 12),
    # 3.13
    "base64.z85encode": (3, 13),
    "copy.replace": (3, 13),
    "random.binomialvariate": (3, 13),
    "typing.NoDefault": (3, 13),
    "typing.ReadOnly": (3, 13),
    "typing.TypeIs": (3, 13),
    "warnings.deprecated": (3, 13),
    # 3.14
    "annotationlib": (3, 14),
    "compression.zstd": (3, 14),
    "concurrent.interpreters": (3, 14),
}

# Builtins added after 3.9.
MIN_VERSION_BY_BUILTIN = {
    "aiter": (3, 10),
    "anext": (3, 10),
    "EncodingWarning": (3, 10),
    "BaseExceptionGroup": (3, 11),
    "ExceptionGroup": (3, 11),
    "PythonFinalizationError": (3, 13),
}

# Keyword arguments added to a stdlib callable later than the callable itself.
# Keyed by (last segment of the callee, keyword) so an alias or a module-
# qualified spelling still matches.
MIN_VERSION_BY_CALL_KEYWORD = {
    ("dataclass", "kw_only"): (3, 10),
    ("dataclass", "match_args"): (3, 10),
    ("dataclass", "slots"): (3, 10),
    ("dataclass", "weakref_slot"): (3, 11),
    ("zip", "strict"): (3, 10),
}

# Method names distinctive enough that the receiver's type need not be proven.
MIN_VERSION_BY_METHOD = {
    "bit_count": (3, 10),  # int.bit_count
    "add_note": (3, 11),  # BaseException.add_note
    "full_match": (3, 13),  # PurePath.full_match
}


def _version_text(version):
    return f"{version[0]}.{version[1]}"


def declared_floor():
    """Return the ``requires-python`` lower bound from pyproject.toml.

    Parsed with a regex rather than ``tomllib``, which is itself 3.11-only --
    this file is scanned by the very gate it implements and has to satisfy it.
    """
    text = PYPROJECT.read_text(encoding="utf-8")
    match = REQUIRES_PYTHON.search(text)
    assert match, f"no `requires-python = \">=X.Y\"` found in {PYPROJECT}"
    return (int(match.group(1)), int(match.group(2)))


def _source_files():
    files = []
    for name in PACKAGE_DIRS:
        files.extend(sorted((PYTHON_ROOT / name).rglob("*.py")))
    return files


def _has_future_annotations(tree):
    return any(
        isinstance(node, ast.ImportFrom)
        and node.module == "__future__"
        and any(alias.name == "annotations" for alias in node.names)
        for node in tree.body
    )


def _evaluated_annotations(tree):
    """Annotation expressions Python evaluates eagerly without PEP 563.

    Signature annotations run at ``def`` time (including for a nested function,
    when its enclosing function runs). Module- and class-level ``x: T`` run when
    the module or class body executes. A function-*local* ``x: T`` never
    evaluates, so it is excluded rather than reported as a false positive.
    """
    found = []

    def visit(node, in_function):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            args = node.args
            slots = [
                *getattr(args, "posonlyargs", []),
                *args.args,
                *args.kwonlyargs,
                args.vararg,
                args.kwarg,
            ]
            found.extend(a.annotation for a in slots if a is not None and a.annotation is not None)
            if node.returns is not None:
                found.append(node.returns)
            for child in node.decorator_list:
                visit(child, in_function)
            for child in node.body:
                visit(child, True)
            return
        if isinstance(node, ast.ClassDef):
            for child in node.decorator_list:
                visit(child, in_function)
            # A class body executes wherever it appears, so its annotations are
            # evaluated even when the class is defined inside a function.
            for child in node.body:
                visit(child, False)
            return
        if isinstance(node, ast.AnnAssign) and not in_function:
            found.append(node.annotation)
        for child in ast.iter_child_nodes(node):
            visit(child, in_function)

    visit(tree, False)
    return found


def _callee_name(func):
    if isinstance(func, ast.Name):
        return func.id
    if isinstance(func, ast.Attribute):
        return func.attr
    return None


def scan_source(source, filename, floor):
    """Return the list of usages in ``source`` that need more than ``floor``."""
    violations = []

    def report(lineno, needs, what):
        if needs > floor:
            violations.append(
                f"{filename}:{lineno}: {what} needs Python {_version_text(needs)}"
            )

    try:
        ast.parse(source, filename=filename, feature_version=floor)
    except SyntaxError as exc:
        violations.append(
            f"{filename}:{exc.lineno}: syntax unavailable on Python "
            f"{_version_text(floor)}: {exc.msg}"
        )

    try:
        tree = ast.parse(source, filename=filename)
    except SyntaxError as exc:
        # Syntax the interpreter running the suite cannot parse either -- newer
        # than the floor by construction, since the suite never runs below it.
        # The feature_version pass above has already reported it; there is no
        # tree left to walk, so report and stop rather than raising out of the
        # gate with a bare SyntaxError.
        if not violations:
            violations.append(f"{filename}:{exc.lineno}: unparseable syntax: {exc.msg}")
        return violations

    # Local name -> dotted module path, so `import x as y` / `from a import b`
    # still resolve attribute accesses back to a table key.
    module_aliases = {}
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            for alias in node.names:
                if alias.name in MIN_VERSION_BY_SYMBOL:
                    report(node.lineno, MIN_VERSION_BY_SYMBOL[alias.name], alias.name)
                bound = alias.asname or alias.name.split(".")[0]
                module_aliases[bound] = alias.name if alias.asname else alias.name.split(".")[0]
        elif isinstance(node, ast.ImportFrom):
            if node.module == "__future__" or node.level:
                continue
            for alias in node.names:
                dotted = f"{node.module}.{alias.name}"
                if dotted in MIN_VERSION_BY_SYMBOL:
                    report(node.lineno, MIN_VERSION_BY_SYMBOL[dotted], dotted)
                module_aliases[alias.asname or alias.name] = dotted

    for node in ast.walk(tree):
        if isinstance(node, ast.Attribute):
            if isinstance(node.value, ast.Name) and node.value.id in module_aliases:
                dotted = f"{module_aliases[node.value.id]}.{node.attr}"
                if dotted in MIN_VERSION_BY_SYMBOL:
                    report(node.lineno, MIN_VERSION_BY_SYMBOL[dotted], dotted)
            if node.attr in MIN_VERSION_BY_METHOD:
                report(node.lineno, MIN_VERSION_BY_METHOD[node.attr], f".{node.attr}()")
        elif isinstance(node, ast.Name) and node.id in MIN_VERSION_BY_BUILTIN:
            report(node.lineno, MIN_VERSION_BY_BUILTIN[node.id], f"builtin {node.id}")
        elif isinstance(node, ast.Call):
            callee = _callee_name(node.func)
            for keyword in node.keywords:
                needs = MIN_VERSION_BY_CALL_KEYWORD.get((callee, keyword.arg))
                if needs is not None:
                    report(node.lineno, needs, f"{callee}({keyword.arg}=...)")

    if not _has_future_annotations(tree):
        for annotation in _evaluated_annotations(tree):
            for sub in ast.walk(annotation):
                if isinstance(sub, ast.BinOp) and isinstance(sub.op, ast.BitOr):
                    report(
                        sub.lineno,
                        (3, 10),
                        "PEP 604 `X | Y` in an eagerly evaluated annotation "
                        "(or add `from __future__ import annotations`)",
                    )

    return violations


def test_sources_stay_within_declared_requires_python() -> None:
    floor = declared_floor()
    files = _source_files()
    assert files, "no python source files found"

    violations = []
    for path in files:
        violations.extend(
            scan_source(
                path.read_text(encoding="utf-8"),
                str(path.relative_to(PYTHON_ROOT)),
                floor,
            )
        )

    assert not violations, (
        f"pyproject.toml declares requires-python >={_version_text(floor)}, but:\n"
        + "\n".join(violations)
        + "\n\nEither lower the implementation or raise requires-python."
    )


def test_suite_runs_at_or_above_the_declared_floor() -> None:
    # A suite running below its own declared floor would be testing an
    # interpreter the package does not claim to support.
    floor = declared_floor()
    assert sys.version_info[:2] >= floor, (
        f"pyproject.toml declares requires-python >={_version_text(floor)} but the "
        f"suite runs on {_version_text(sys.version_info[:2])}"
    )


def test_gate_flags_usage_newer_than_the_floor() -> None:
    floor = (3, 9)
    cases = {
        "match statement": "match value:\n    case 1:\n        pass\n",
        "except*": "try:\n    pass\nexcept* ValueError:\n    pass\n",
        "StrEnum from-import": "from enum import StrEnum\n",
        "StrEnum module attribute": "import enum\n\n\nclass L(enum.StrEnum):\n    A = 'a'\n",
        "aliased module attribute": "import enum as e\n\n\nX = e.ReprEnum\n",
        "tomllib": "import tomllib\n",
        "typing.Self": "from typing import Self\n",
        "datetime.UTC": "from datetime import UTC\n",
        "ExceptionGroup builtin": "def f():\n    raise ExceptionGroup('x', [ValueError()])\n",
        "add_note": "def f(exc):\n    exc.add_note('context')\n",
        "dataclass slots": (
            "from dataclasses import dataclass\n\n\n@dataclass(slots=True)\nclass C:\n    x: int\n"
        ),
        "zip strict": "def f(a, b):\n    return list(zip(a, b, strict=True))\n",
        "PEP 604 in a signature": "def f(a: int | None) -> str | None:\n    return None\n",
        "PEP 604 at module level": "X: int | None = None\n",
        "PEP 604 in a class body": "class C:\n    x: int | None = None\n",
    }
    for name, source in cases.items():
        assert scan_source(source, f"<{name}>", floor), f"gate missed: {name}"


def test_gate_accepts_floor_clean_sources() -> None:
    floor = (3, 9)
    clean = (
        "from enum import Enum\n"
        "\n"
        "\n"
        "class L(str, Enum):\n"
        "    A = 'a'\n"
        "\n"
        "\n"
        "def f(values: list, mapping: dict) -> str:\n"
        "    local: list = []\n"
        "    return ','.join(local) or str(values) + str(mapping)\n"
    )
    assert scan_source(clean, "<clean>", floor) == []

    # PEP 604 is fine once the module opts into lazy annotations.
    deferred = (
        "from __future__ import annotations\n"
        "\n"
        "\n"
        "def f(a: int | None) -> str | None:\n"
        "    return None\n"
    )
    assert scan_source(deferred, "<deferred>", floor) == []

    # A function-local annotation is never evaluated, so it is not a violation
    # even without the future import.
    local_only = "def f():\n    x: int | None = None\n    return x\n"
    assert scan_source(local_only, "<local>", floor) == []
