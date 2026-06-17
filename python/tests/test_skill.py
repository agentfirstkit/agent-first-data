"""Tests for the skill admin module, mirroring the Rust port's coverage."""

import os

import pytest

from agent_first_data import (
    SkillAction,
    SkillAgentSelection,
    SkillError,
    SkillOptions,
    SkillScope,
    SkillSpec,
    run_skill_admin,
)
from agent_first_data.skill import (
    SKILL_FILE_NAME,
    SkillAgent,
    _generated_by,
    _legacy_marker_block,
    _managed_marker_block,
    _marker,
    _resolve_targets,
    _validate_skill_frontmatter,
)

SKILL_SOURCE = "---\nname: agent-first-test\ndescription: test skill\n---\n\n# Body\n\nrules.\n"


def spec() -> SkillSpec:
    return SkillSpec(
        name="agent-first-test",
        source=SKILL_SOURCE,
        title="Agent-First Test",
        marker_slug="aftest",
    )


def legacy_managed_skill(body: str) -> str:
    return (
        "---\nname: agent-first-test\ndescription: test skill\n---\n"
        f"{_legacy_marker_block(spec())}\n\n{body}"
    )


def options(agent: SkillAgentSelection, skills_dir, force: bool = False) -> SkillOptions:
    return SkillOptions(agent=agent, scope=SkillScope.PERSONAL, skills_dir=str(skills_dir), force=force)


def test_validates_bundled_frontmatter():
    assert _validate_skill_frontmatter(SKILL_SOURCE) is None


def test_rejects_unquoted_colon_space():
    bad = "---\nname: x\ndescription: broken: yaml\n---\n"
    assert _validate_skill_frontmatter(bad) is not None


@pytest.mark.parametrize(
    "agent,expect",
    [
        (SkillAgentSelection.CODEX, "codex"),
        (SkillAgentSelection.CLAUDE_CODE, "claude-code"),
        (SkillAgentSelection.OPENCODE, "opencode"),
        (SkillAgentSelection.HERMES, "hermes"),
    ],
)
def test_install_status_uninstall(agent, expect, tmp_path):
    opts = options(agent, tmp_path)
    skill_path = tmp_path / "agent-first-test" / SKILL_FILE_NAME

    report = run_skill_admin(spec(), SkillAction.INSTALL, opts)
    assert skill_path.is_file()
    installed_text = skill_path.read_text(encoding="utf-8")
    assert _managed_marker_block(spec()) in installed_text
    assert "aftest-managed-skill-name: agent-first-test" in installed_text
    assert "aftest-managed-skill-source-hash-fnv1a64:" in installed_text
    # Structured report serializes to the protocol shape.
    assert report.to_dict()["code"] == "skill_install"

    report = run_skill_admin(spec(), SkillAction.STATUS, opts)
    assert report.installed_all is True
    assert report.valid_all is True
    assert report.current_all is True
    assert report.targets[0].agent.value == expect
    assert report.targets[0].current is True

    run_skill_admin(spec(), SkillAction.UNINSTALL, opts)
    assert not skill_path.exists()


def test_status_reports_stale_install_as_not_current(tmp_path):
    opts = options(SkillAgentSelection.OPENCODE, tmp_path)
    skill_dir = tmp_path / "agent-first-test"
    skill_dir.mkdir(parents=True)
    skill_path = skill_dir / SKILL_FILE_NAME
    stale = legacy_managed_skill("# Body\n\nOLD rules.\n")
    skill_path.write_text(stale, encoding="utf-8")

    report = run_skill_admin(spec(), SkillAction.STATUS, opts)
    assert report.current_all is False
    t = report.targets[0]
    assert t.installed is True
    assert t.valid is True
    assert t.managed is True
    assert t.current is False

    # Reinstall makes it current again.
    run_skill_admin(spec(), SkillAction.INSTALL, opts)
    refreshed = skill_path.read_text(encoding="utf-8")
    assert _managed_marker_block(spec()) in refreshed
    assert f"<!-- {_marker(spec())} -->" not in refreshed
    report = run_skill_admin(spec(), SkillAction.STATUS, opts)
    assert report.targets[0].current is True


def test_random_text_with_marker_words_is_not_managed(tmp_path):
    opts = options(SkillAgentSelection.OPENCODE, tmp_path)
    skill_dir = tmp_path / "agent-first-test"
    skill_dir.mkdir(parents=True)
    skill_path = skill_dir / SKILL_FILE_NAME
    random = (
        "---\nname: agent-first-test\ndescription: test skill\n---\n\n"
        f"This mentions {_generated_by(spec())} and {_marker(spec())} but is not a generated block.\n"
    )
    skill_path.write_text(random, encoding="utf-8")

    report = run_skill_admin(spec(), SkillAction.STATUS, opts)
    assert report.targets[0].managed is False
    with pytest.raises(SkillError):
        run_skill_admin(spec(), SkillAction.INSTALL, opts)


def test_install_and_uninstall_refuse_unmanaged(tmp_path):
    skill_dir = tmp_path / "agent-first-test"
    skill_dir.mkdir(parents=True)
    skill_path = skill_dir / SKILL_FILE_NAME
    skill_path.write_text("---\nname: custom\ndescription: custom\n---\n", encoding="utf-8")
    opts = options(SkillAgentSelection.CODEX, tmp_path)

    with pytest.raises(SkillError):
        run_skill_admin(spec(), SkillAction.INSTALL, opts)
    with pytest.raises(SkillError):
        run_skill_admin(spec(), SkillAction.UNINSTALL, opts)
    assert skill_path.exists()


def test_invalid_spec_slugs_rejected_before_path_resolution(tmp_path):
    for name in ["", "../x", "x/y", ".hidden", "bad_name", "Bad"]:
        bad = SkillSpec(name=name, source=SKILL_SOURCE, title="Bad", marker_slug="aftest")
        with pytest.raises(SkillError):
            run_skill_admin(bad, SkillAction.STATUS, options(SkillAgentSelection.CODEX, tmp_path))

    bad_marker = SkillSpec(
        name="agent-first-test",
        source=SKILL_SOURCE,
        title="Bad",
        marker_slug="../aftest",
    )
    with pytest.raises(SkillError):
        run_skill_admin(bad_marker, SkillAction.STATUS, options(SkillAgentSelection.CODEX, tmp_path))


def test_frontmatter_name_must_match_spec_name(tmp_path):
    bad = SkillSpec(
        name="agent-first-test",
        source="---\nname: other-skill\ndescription: test skill\n---\n",
        title="Bad",
        marker_slug="aftest",
    )
    with pytest.raises(SkillError):
        run_skill_admin(bad, SkillAction.INSTALL, options(SkillAgentSelection.CODEX, tmp_path))


def test_symlink_target_rejected_by_default_and_force_does_not_follow(tmp_path):
    if not hasattr(os, "symlink"):
        pytest.skip("symlink unsupported")
    opts = options(SkillAgentSelection.CODEX, tmp_path)
    force_opts = options(SkillAgentSelection.CODEX, tmp_path, force=True)
    skill_dir = tmp_path / "agent-first-test"
    skill_dir.mkdir(parents=True)
    skill_path = skill_dir / SKILL_FILE_NAME
    external = tmp_path / "external.md"
    external.write_text("external", encoding="utf-8")
    try:
        os.symlink(external, skill_path)
    except OSError as exc:
        pytest.skip(f"symlink unsupported: {exc}")

    with pytest.raises(SkillError):
        run_skill_admin(spec(), SkillAction.INSTALL, opts)
    assert external.read_text(encoding="utf-8") == "external"
    with pytest.raises(SkillError):
        run_skill_admin(spec(), SkillAction.UNINSTALL, opts)
    assert skill_path.is_symlink()

    run_skill_admin(spec(), SkillAction.INSTALL, force_opts)
    assert external.read_text(encoding="utf-8") == "external"
    assert skill_path.is_file()
    assert not skill_path.is_symlink()


def test_force_uninstall_removes_symlink_without_following(tmp_path):
    if not hasattr(os, "symlink"):
        pytest.skip("symlink unsupported")
    force_opts = options(SkillAgentSelection.CODEX, tmp_path, force=True)
    skill_dir = tmp_path / "agent-first-test"
    skill_dir.mkdir(parents=True)
    skill_path = skill_dir / SKILL_FILE_NAME
    external = tmp_path / "external.md"
    external.write_text("external", encoding="utf-8")
    try:
        os.symlink(external, skill_path)
    except OSError as exc:
        pytest.skip(f"symlink unsupported: {exc}")

    run_skill_admin(spec(), SkillAction.UNINSTALL, force_opts)
    assert not skill_path.exists()
    assert external.read_text(encoding="utf-8") == "external"


def test_all_personal_resolves_four_targets():
    opts = SkillOptions(agent=SkillAgentSelection.ALL, scope=SkillScope.PERSONAL)
    targets = _resolve_targets(spec(), opts)
    assert [t.agent for t in targets] == [
        SkillAgent.CODEX,
        SkillAgent.CLAUDE_CODE,
        SkillAgent.OPENCODE,
        SkillAgent.HERMES,
    ]


def test_all_workspace_resolves_four_targets():
    opts = SkillOptions(agent=SkillAgentSelection.ALL, scope=SkillScope.WORKSPACE)
    targets = _resolve_targets(spec(), opts)
    assert [t.agent for t in targets] == [
        SkillAgent.CODEX,
        SkillAgent.CLAUDE_CODE,
        SkillAgent.OPENCODE,
        SkillAgent.HERMES,
    ]
    assert [t.scope for t in targets] == [
        SkillScope.WORKSPACE,
        SkillScope.WORKSPACE,
        SkillScope.WORKSPACE,
        SkillScope.WORKSPACE,
    ]


def test_codex_workspace_scope_uses_codex_skills_dir():
    opts = SkillOptions(agent=SkillAgentSelection.CODEX, scope=SkillScope.WORKSPACE)
    targets = _resolve_targets(spec(), opts)
    assert [t.agent for t in targets] == [SkillAgent.CODEX]
    assert targets[0].scope is SkillScope.WORKSPACE
    assert str(targets[0].skills_dir).endswith(".codex/skills")


def test_hermes_workspace_scope_uses_hermes_skills_dir():
    opts = SkillOptions(agent=SkillAgentSelection.HERMES, scope=SkillScope.WORKSPACE)
    targets = _resolve_targets(spec(), opts)
    assert [t.agent for t in targets] == [SkillAgent.HERMES]
    assert targets[0].scope is SkillScope.WORKSPACE
    assert str(targets[0].skills_dir).endswith(".hermes/skills")


def test_skills_dir_requires_single_agent():
    opts = SkillOptions(agent=SkillAgentSelection.ALL, scope=SkillScope.PERSONAL, skills_dir="/tmp/x")
    with pytest.raises(SkillError):
        _resolve_targets(spec(), opts)
