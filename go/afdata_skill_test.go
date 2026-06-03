package afdata

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

const skillSource = "---\nname: agent-first-test\ndescription: test skill\n---\n\n# Body\n\nrules.\n"

func testSpec() SkillSpec {
	return SkillSpec{
		Name:       "agent-first-test",
		Source:     skillSource,
		Title:      "Agent-First Test",
		MarkerSlug: "aftest",
	}
}

func testOptions(agent SkillAgentSelection, dir string, force bool) SkillOptions {
	return SkillOptions{Agent: agent, Scope: SkillScopePersonal, SkillsDir: dir, Force: force}
}

func TestSkillValidatesBundledFrontmatter(t *testing.T) {
	if err := skillValidateFrontmatter(skillSource); err != nil {
		t.Fatalf("bundled front matter should be valid: %v", err)
	}
}

func TestSkillRejectsUnquotedColonSpace(t *testing.T) {
	bad := "---\nname: x\ndescription: broken: yaml\n---\n"
	if err := skillValidateFrontmatter(bad); err == nil {
		t.Fatal("expected unquoted ': ' to be rejected")
	}
}

func installStatusUninstallFor(t *testing.T, agent SkillAgentSelection, expect string) {
	t.Helper()
	dir := t.TempDir()
	opts := testOptions(agent, dir, false)
	skillPath := filepath.Join(dir, "agent-first-test", skillFileName)

	if _, err := RunSkillAdmin(testSpec(), SkillActionInstall, opts); err != nil {
		t.Fatalf("install: %v", err)
	}
	if !skillIsFile(skillPath) {
		t.Fatalf("expected skill file at %s", skillPath)
	}
	data, _ := os.ReadFile(skillPath)
	if want := skillMarker(testSpec()); !strings.Contains(string(data), want) {
		t.Fatalf("installed skill missing marker %q", want)
	}

	report, err := RunSkillAdmin(testSpec(), SkillActionStatus, opts)
	if err != nil {
		t.Fatalf("status: %v", err)
	}
	status, ok := report.(*SkillStatusReport)
	if !ok {
		t.Fatalf("expected *SkillStatusReport, got %T", report)
	}
	if !status.InstalledAll || !status.ValidAll || !status.CurrentAll {
		t.Fatalf("status rollups not all true: %+v", status)
	}
	if status.Targets[0].Agent != expect {
		t.Fatalf("agent = %v, want %q", status.Targets[0].Agent, expect)
	}
	if !status.Targets[0].Current {
		t.Fatalf("expected current=true, got %v", status.Targets[0].Current)
	}

	if _, err := RunSkillAdmin(testSpec(), SkillActionUninstall, opts); err != nil {
		t.Fatalf("uninstall: %v", err)
	}
	if skillPathExists(skillPath) {
		t.Fatalf("expected skill removed at %s", skillPath)
	}
}

func TestSkillInstallStatusUninstallCodex(t *testing.T) {
	installStatusUninstallFor(t, SkillAgentCodex, "codex")
}

func TestSkillInstallStatusUninstallClaudeCode(t *testing.T) {
	installStatusUninstallFor(t, SkillAgentClaudeCode, "claude-code")
}

func TestSkillInstallStatusUninstallOpencode(t *testing.T) {
	installStatusUninstallFor(t, SkillAgentOpencode, "opencode")
}

func TestSkillStatusReportsStaleInstallAsNotCurrent(t *testing.T) {
	dir := t.TempDir()
	opts := testOptions(SkillAgentOpencode, dir, false)
	skillDir := filepath.Join(dir, "agent-first-test")
	skillPath := filepath.Join(skillDir, skillFileName)
	if err := os.MkdirAll(skillDir, 0o755); err != nil {
		t.Fatal(err)
	}
	// A managed marker but stale body: valid + managed, but not current.
	stale := "---\nname: agent-first-test\ndescription: test skill\n---\n<!-- " +
		skillGeneratedBy(testSpec()) + " -->\n<!-- " + skillMarker(testSpec()) +
		" -->\n\n# Body\n\nOLD rules.\n"
	if err := os.WriteFile(skillPath, []byte(stale), 0o644); err != nil {
		t.Fatal(err)
	}

	report, err := RunSkillAdmin(testSpec(), SkillActionStatus, opts)
	if err != nil {
		t.Fatalf("status: %v", err)
	}
	status := report.(*SkillStatusReport)
	if status.CurrentAll {
		t.Fatalf("expected current_all=false, got %v", status.CurrentAll)
	}
	first := status.Targets[0]
	if !first.Installed || !first.Valid || !first.Managed {
		t.Fatalf("expected installed+valid+managed, got %+v", first)
	}
	if first.Current {
		t.Fatalf("expected current=false, got %v", first.Current)
	}

	// Reinstall makes it current again.
	if _, err := RunSkillAdmin(testSpec(), SkillActionInstall, opts); err != nil {
		t.Fatalf("reinstall: %v", err)
	}
	report, _ = RunSkillAdmin(testSpec(), SkillActionStatus, opts)
	if !report.(*SkillStatusReport).Targets[0].Current {
		t.Fatalf("expected current=true after reinstall")
	}
}

func TestSkillInstallAndUninstallRefuseUnmanaged(t *testing.T) {
	dir := t.TempDir()
	skillDir := filepath.Join(dir, "agent-first-test")
	skillPath := filepath.Join(skillDir, skillFileName)
	if err := os.MkdirAll(skillDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(skillPath, []byte("---\nname: custom\ndescription: custom\n---\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	opts := testOptions(SkillAgentCodex, dir, false)

	if _, err := RunSkillAdmin(testSpec(), SkillActionInstall, opts); err == nil {
		t.Fatal("expected install to refuse unmanaged skill")
	}
	if _, err := RunSkillAdmin(testSpec(), SkillActionUninstall, opts); err == nil {
		t.Fatal("expected uninstall to refuse unmanaged skill")
	}
	if !skillPathExists(skillPath) {
		t.Fatal("unmanaged skill should be left in place")
	}
}

func TestSkillAllPersonalResolvesThreeTargets(t *testing.T) {
	opts := SkillOptions{Agent: SkillAgentAll, Scope: SkillScopePersonal}
	targets, err := skillResolveTargets(testSpec(), opts)
	if err != nil {
		t.Fatalf("resolve: %v", err)
	}
	if len(targets) != 3 {
		t.Fatalf("len = %d, want 3", len(targets))
	}
	want := []skillAgent{agentCodex, agentClaudeCode, agentOpencode}
	for i, w := range want {
		if targets[i].agent != w {
			t.Fatalf("targets[%d].agent = %q, want %q", i, targets[i].agent, w)
		}
	}
}

func TestSkillAllProjectSkipsCodex(t *testing.T) {
	opts := SkillOptions{Agent: SkillAgentAll, Scope: SkillScopeProject}
	targets, err := skillResolveTargets(testSpec(), opts)
	if err != nil {
		t.Fatalf("resolve: %v", err)
	}
	if len(targets) != 2 {
		t.Fatalf("len = %d, want 2", len(targets))
	}
	if targets[0].agent != agentClaudeCode || targets[1].agent != agentOpencode {
		t.Fatalf("unexpected agents: %q, %q", targets[0].agent, targets[1].agent)
	}
}

func TestSkillCodexProjectScopeRejected(t *testing.T) {
	opts := SkillOptions{Agent: SkillAgentCodex, Scope: SkillScopeProject}
	if _, err := skillResolveTargets(testSpec(), opts); err == nil {
		t.Fatal("expected Codex project scope to be rejected")
	}
}

func TestSkillSkillsDirRequiresSingleAgent(t *testing.T) {
	opts := SkillOptions{Agent: SkillAgentAll, Scope: SkillScopePersonal, SkillsDir: "/tmp/x"}
	if _, err := skillResolveTargets(testSpec(), opts); err == nil {
		t.Fatal("expected --skills-dir with --agent all to be rejected")
	}
}
