"""Tests for the skill admin module, mirroring the Rust port's coverage."""

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
    ],
)
def test_install_status_uninstall(agent, expect, tmp_path):
    opts = options(agent, tmp_path)
    skill_path = tmp_path / "agent-first-test" / SKILL_FILE_NAME

    report = run_skill_admin(spec(), SkillAction.INSTALL, opts)
    assert skill_path.is_file()
    assert _marker(spec()) in skill_path.read_text(encoding="utf-8")
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
    stale = (
        "---\nname: agent-first-test\ndescription: test skill\n---\n"
        f"<!-- {_generated_by(spec())} -->\n<!-- {_marker(spec())} -->\n\n# Body\n\nOLD rules.\n"
    )
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
    report = run_skill_admin(spec(), SkillAction.STATUS, opts)
    assert report.targets[0].current is True


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


def test_all_personal_resolves_three_targets():
    opts = SkillOptions(agent=SkillAgentSelection.ALL, scope=SkillScope.PERSONAL)
    targets = _resolve_targets(spec(), opts)
    assert [t.agent for t in targets] == [SkillAgent.CODEX, SkillAgent.CLAUDE_CODE, SkillAgent.OPENCODE]


def test_all_project_skips_codex():
    opts = SkillOptions(agent=SkillAgentSelection.ALL, scope=SkillScope.PROJECT)
    targets = _resolve_targets(spec(), opts)
    assert [t.agent for t in targets] == [SkillAgent.CLAUDE_CODE, SkillAgent.OPENCODE]


def test_codex_project_scope_rejected():
    opts = SkillOptions(agent=SkillAgentSelection.CODEX, scope=SkillScope.PROJECT)
    with pytest.raises(SkillError):
        _resolve_targets(spec(), opts)


def test_skills_dir_requires_single_agent():
    opts = SkillOptions(agent=SkillAgentSelection.ALL, scope=SkillScope.PERSONAL, skills_dir="/tmp/x")
    with pytest.raises(SkillError):
        _resolve_targets(spec(), opts)
