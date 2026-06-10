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

func legacyManagedSkill(body string) string {
	return "---\nname: agent-first-test\ndescription: test skill\n---\n" +
		skillLegacyMarkerBlock(testSpec()) + "\n\n" + body
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
	if want := skillManagedMarkerBlock(testSpec()); !strings.Contains(string(data), want) {
		t.Fatalf("installed skill missing marker %q", want)
	}
	if !strings.Contains(string(data), "aftest-managed-skill-name: agent-first-test") {
		t.Fatalf("installed skill missing bound name marker: %s", string(data))
	}
	if !strings.Contains(string(data), "aftest-managed-skill-source-hash-fnv1a64:") {
		t.Fatalf("installed skill missing source hash marker: %s", string(data))
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
	stale := legacyManagedSkill("# Body\n\nOLD rules.\n")
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
	refreshed, _ := os.ReadFile(skillPath)
	if !strings.Contains(string(refreshed), skillManagedMarkerBlock(testSpec())) {
		t.Fatalf("expected refreshed new marker, got: %s", string(refreshed))
	}
	if strings.Contains(string(refreshed), "<!-- "+skillMarker(testSpec())+" -->") {
		t.Fatalf("legacy marker should be refreshed, got: %s", string(refreshed))
	}
	report, _ = RunSkillAdmin(testSpec(), SkillActionStatus, opts)
	if !report.(*SkillStatusReport).Targets[0].Current {
		t.Fatalf("expected current=true after reinstall")
	}
}

func TestSkillRandomTextWithMarkerWordsIsNotManaged(t *testing.T) {
	dir := t.TempDir()
	opts := testOptions(SkillAgentOpencode, dir, false)
	skillDir := filepath.Join(dir, "agent-first-test")
	skillPath := filepath.Join(skillDir, skillFileName)
	if err := os.MkdirAll(skillDir, 0o755); err != nil {
		t.Fatal(err)
	}
	random := "---\nname: agent-first-test\ndescription: test skill\n---\n\nThis mentions " +
		skillGeneratedBy(testSpec()) + " and " + skillMarker(testSpec()) +
		" but is not a generated block.\n"
	if err := os.WriteFile(skillPath, []byte(random), 0o644); err != nil {
		t.Fatal(err)
	}

	report, err := RunSkillAdmin(testSpec(), SkillActionStatus, opts)
	if err != nil {
		t.Fatalf("status: %v", err)
	}
	if report.(*SkillStatusReport).Targets[0].Managed {
		t.Fatalf("random marker words should not be managed")
	}
	if _, err := RunSkillAdmin(testSpec(), SkillActionInstall, opts); err == nil {
		t.Fatalf("expected random marker words to be refused as unmanaged")
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

func TestSkillInvalidSpecSlugsRejectedBeforePathResolution(t *testing.T) {
	for _, name := range []string{"", "../x", "x/y", ".hidden", "bad_name", "Bad"} {
		bad := SkillSpec{Name: name, Source: skillSource, Title: "Bad", MarkerSlug: "aftest"}
		opts := testOptions(SkillAgentCodex, filepath.Join(os.TempDir(), "afdata"), false)
		if _, err := RunSkillAdmin(bad, SkillActionStatus, opts); err == nil {
			t.Fatalf("expected invalid name %q to be rejected", name)
		}
	}

	badMarker := SkillSpec{Name: "agent-first-test", Source: skillSource, Title: "Bad", MarkerSlug: "../aftest"}
	opts := testOptions(SkillAgentCodex, filepath.Join(os.TempDir(), "afdata"), false)
	if _, err := RunSkillAdmin(badMarker, SkillActionStatus, opts); err == nil {
		t.Fatal("expected invalid marker slug to be rejected")
	}
}

func TestSkillFrontmatterNameMustMatchSpecName(t *testing.T) {
	bad := SkillSpec{
		Name:       "agent-first-test",
		Source:     "---\nname: other-skill\ndescription: test skill\n---\n",
		Title:      "Bad",
		MarkerSlug: "aftest",
	}
	if _, err := RunSkillAdmin(bad, SkillActionInstall, testOptions(SkillAgentCodex, t.TempDir(), false)); err == nil {
		t.Fatal("expected front matter name mismatch to be rejected")
	}
}

func TestSkillSymlinkTargetRejectedByDefaultAndForceDoesNotFollow(t *testing.T) {
	dir := t.TempDir()
	opts := testOptions(SkillAgentCodex, dir, false)
	forceOpts := testOptions(SkillAgentCodex, dir, true)
	skillDir := filepath.Join(dir, "agent-first-test")
	skillPath := filepath.Join(skillDir, skillFileName)
	external := filepath.Join(dir, "external.md")
	if err := os.MkdirAll(skillDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(external, []byte("external"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(external, skillPath); err != nil {
		t.Skipf("symlink unsupported: %v", err)
	}

	if _, err := RunSkillAdmin(testSpec(), SkillActionInstall, opts); err == nil {
		t.Fatal("expected install to reject symlink by default")
	}
	if got, _ := os.ReadFile(external); string(got) != "external" {
		t.Fatalf("external target was modified: %q", string(got))
	}
	if _, err := RunSkillAdmin(testSpec(), SkillActionUninstall, opts); err == nil {
		t.Fatal("expected uninstall to reject symlink by default")
	}
	if info, err := os.Lstat(skillPath); err != nil || info.Mode()&os.ModeSymlink == 0 {
		t.Fatalf("expected symlink to remain, info=%v err=%v", info, err)
	}

	if _, err := RunSkillAdmin(testSpec(), SkillActionInstall, forceOpts); err != nil {
		t.Fatalf("force install: %v", err)
	}
	if got, _ := os.ReadFile(external); string(got) != "external" {
		t.Fatalf("external target was modified by force install: %q", string(got))
	}
	if info, err := os.Lstat(skillPath); err != nil || info.Mode()&os.ModeSymlink != 0 {
		t.Fatalf("expected regular file after force install, info=%v err=%v", info, err)
	}
}

func TestSkillForceUninstallRemovesSymlinkWithoutFollowing(t *testing.T) {
	dir := t.TempDir()
	forceOpts := testOptions(SkillAgentCodex, dir, true)
	skillDir := filepath.Join(dir, "agent-first-test")
	skillPath := filepath.Join(skillDir, skillFileName)
	external := filepath.Join(dir, "external.md")
	if err := os.MkdirAll(skillDir, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(external, []byte("external"), 0o644); err != nil {
		t.Fatal(err)
	}
	if err := os.Symlink(external, skillPath); err != nil {
		t.Skipf("symlink unsupported: %v", err)
	}

	if _, err := RunSkillAdmin(testSpec(), SkillActionUninstall, forceOpts); err != nil {
		t.Fatalf("force uninstall: %v", err)
	}
	if _, err := os.Lstat(skillPath); !os.IsNotExist(err) {
		t.Fatalf("expected symlink removed, err=%v", err)
	}
	if got, _ := os.ReadFile(external); string(got) != "external" {
		t.Fatalf("external target was modified: %q", string(got))
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
