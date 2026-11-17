import { test, expect } from "bun:test";
import { execFileSync, spawnSync } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

// Run with: bun test scripts/release.test.ts
// All release commands run in disposable repositories; publication is intercepted.
for (const flags of [["--skip-git"], ["--skip-git", "--no-push"], ["--dry-run"], ["--no-push"]]) {
  test(`release ${flags.join(" ")}`, () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "rune-release-test-"));
    const repo = path.join(root, "repo");
    const bin = path.join(root, "bin");
    const commandLog = path.join(root, "commands.txt");
    const realGit = Bun.which("git");
    if (!realGit) throw new Error("git is required to run release tests");
    const env = {
      ...process.env,
      GIT_CONFIG_GLOBAL: path.join(root, "gitconfig"),
      GIT_CONFIG_NOSYSTEM: "1",
      GIT_AUTHOR_NAME: "Release Test",
      GIT_AUTHOR_EMAIL: "release@example.invalid",
      GIT_COMMITTER_NAME: "Release Test",
      GIT_COMMITTER_EMAIL: "release@example.invalid",
      COMMAND_LOG: commandLog,
      REAL_GIT: realGit,
      PATH: `${bin}${path.delimiter}${process.env.PATH}`,
    };
    fs.mkdirSync(path.join(repo, "scripts"), { recursive: true });
    fs.mkdirSync(bin);
    const git = (...args: string[]) => execFileSync(realGit, args, { cwd: repo, env, encoding: "utf8" }).trim();
    const read = (name: string) => fs.readFileSync(path.join(repo, name), "utf8");
    try {
      fs.copyFileSync(path.join(import.meta.dir, "release.ts"), path.join(repo, "scripts/release.ts"));
      fs.writeFileSync(path.join(repo, "pubspec.yaml"), "name: rune\nversion: 1.0.0\nmsix_config:\n  msix_version: 1.0.0.0\n");
      fs.writeFileSync(path.join(repo, "CHANGELOG.md"), "# Changelog\n\nExisting release\n");
      fs.writeFileSync(path.join(repo, ".gitignore"), "RELEASE_NOTES.txt\nrelease_notes.txt\n*.release.txt\nupgrade.txt\n");
      git("init", "-q");
      git("add", ".");
      git("commit", "-qm", "Initial fixture");
      git("tag", "v1.0.0");
      const head = git("rev-parse", "HEAD");
      const index = git("write-tree");
      const notes = "[Bugs]\n- Preserve maintainer control over publication\n";
      fs.writeFileSync(path.join(repo, "RELEASE_NOTES.txt"), notes);
      fs.writeFileSync(commandLog, "");
      fs.writeFileSync(path.join(bin, "git"), `#!/bin/sh
printf 'git %s\\n' "$*" >> "$COMMAND_LOG"
if [ "$1" = push ]; then exit 0; fi
exec "$REAL_GIT" "$@"
`, { mode: 0o755 });
      fs.writeFileSync(path.join(bin, "gh"), `#!/bin/sh
printf 'gh %s\\n' "$*" >> "$COMMAND_LOG"
`, { mode: 0o755 });
      const result = spawnSync(process.execPath, [path.join(repo, "scripts/release.ts"), "2.0.0", ...flags], {
        cwd: root,
        env,
        encoding: "utf8",
        timeout: 30_000,
      });
      expect(result.error).toBeUndefined();
      expect(result.status, result.stdout + result.stderr).toBe(0);
      const commands = fs.readFileSync(commandLog, "utf8").trim().split("\n");
      const writes = commands.filter(line => /^git (add|commit|tag|push)( |$)|^gh /.test(line));
      if (flags.includes("--skip-git") || flags.includes("--dry-run")) {
        expect(writes, "No Git writes or publication commands should be attempted").toEqual([]);
      }
      expect(read("RELEASE_NOTES.txt")).toBe(notes);
      expect(commands.some(line => /^git push |^gh /.test(line))).toBe(false);
      if (flags.includes("--dry-run")) {
        expect(read("pubspec.yaml")).toContain("version: 1.0.0\n");
        expect(read("CHANGELOG.md")).toBe("# Changelog\n\nExisting release\n");
      } else {
        expect(read("pubspec.yaml")).toContain("version: 2.0.0\n");
        expect(read("pubspec.yaml")).toContain("msix_version: 2.0.0.0\n");
        expect(read("CHANGELOG.md")).toContain("## [2.0.0]");
        expect(read("CHANGELOG.md")).toContain("Preserve maintainer control over publication");
        expect(read("CHANGELOG.md")).toContain("Existing release");
      }
      if (flags.includes("--skip-git") || flags.includes("--dry-run")) {
        expect(git("rev-parse", "HEAD")).toBe(head);
        expect(git("write-tree")).toBe(index);
        expect(git("tag", "--list")).toBe("v1.0.0");
      } else {
        expect(writes.some(line => line.startsWith("git add "))).toBe(true);
        expect(writes.some(line => line.startsWith("git commit "))).toBe(true);
        expect(writes).toContain("git tag v2.0.0");
        expect(git("rev-parse", "HEAD")).not.toBe(head);
        expect(git("rev-parse", "v2.0.0")).toBe(git("rev-parse", "HEAD"));
        expect(git("status", "--porcelain")).toBe("");
      }
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });
}
