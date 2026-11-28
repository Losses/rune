import { beforeAll, afterAll, test, expect } from "bun:test";
import { execFileSync, spawnSync } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

// Run with: bun test scripts/release.test.ts
// Native executables intercept commands on Windows as well as Unix.
const tools = fs.mkdtempSync(path.join(os.tmpdir(), "rune-release-tools-"));
const suffix = process.platform === "win32" ? ".exe" : "";
beforeAll(() => {
  const source = path.join(tools, "recorder.ts");
  fs.writeFileSync(source, `import * as fs from "node:fs";
import * as path from "node:path";
import { spawnSync } from "node:child_process";
const name = path.basename(process.execPath).replace(/\\.exe$/i, "");
const args = process.argv.slice(2);
fs.appendFileSync(process.env.COMMAND_LOG!, name + " " + args.join(" ") + "\\n");
if (name === "git" && args[0] !== "push") {
  const result = spawnSync(process.env.REAL_GIT!, args, { stdio: "inherit" });
  if (result.error) throw result.error;
  process.exit(result.status ?? 1);
}
`);
  const result = spawnSync(process.execPath, ["build", "--compile", source, "--outfile", path.join(tools, `git${suffix}`)], { encoding: "utf8" });
  expect(result.status, result.stdout + result.stderr).toBe(0);
  fs.copyFileSync(path.join(tools, `git${suffix}`), path.join(tools, `gh${suffix}`));
}, 30_000);
afterAll(() => fs.rmSync(tools, { recursive: true, force: true }));

function fixture() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "rune-release-test-"));
  const repo = path.join(root, "repo");
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
    PATH: `${tools}${path.delimiter}${process.env.PATH}`,
  };
  fs.mkdirSync(path.join(repo, "scripts"), { recursive: true });
  fs.mkdirSync(path.join(repo, "assets/source/linux"), { recursive: true });
  const git = (...args: string[]) => execFileSync(realGit, args, { cwd: repo, env, encoding: "utf8" }).trim();
  const read = (name: string) => fs.readFileSync(path.join(repo, name), "utf8");
  const write = (name: string, value: string) => fs.writeFileSync(path.join(repo, name), value);
  fs.copyFileSync(path.join(import.meta.dir, "release.ts"), path.join(repo, "scripts/release.ts"));
  write("pubspec.yaml", "name: rune\nversion: 1.0.0\nmsix_config:\n  msix_version: 1.0.0.0\n");
  write("assets/source/linux/rune.metainfo.xml", "<component><releases></releases></component>\n");
  write(".gitignore", "RELEASE_NOTES.txt\nrelease_notes.txt\n*.release.txt\nupgrade.txt\n");
  git("init", "-q");
  git("add", ".");
  git("commit", "-qm", "Initial fixture");
  git("tag", "v1.0.0");
  const notes = "[Bugs]\n- Preserve maintainer control over publication\n";
  write("RELEASE_NOTES.txt", notes);
  fs.writeFileSync(commandLog, "");
  const run = (version: string, ...flags: string[]) => {
    const result = spawnSync(process.execPath, [path.join(repo, "scripts/release.ts"), version, ...flags], { cwd: root, env, encoding: "utf8", timeout: 30_000 });
    expect(result.error).toBeUndefined();
    return result;
  };
  return { repo, git, read, write, notes, run,
    commands: () => fs.readFileSync(commandLog, "utf8").trim().split("\n"),
    clean: () => fs.rmSync(root, { recursive: true, force: true }),
  };
}

for (const flags of [["--skip-git"], ["--skip-git", "--no-push"], ["--dry-run"], ["--no-push"]]) {
  test(`release ${flags.join(" ")}`, () => {
    const f = fixture();
    try {
      const head = f.git("rev-parse", "HEAD");
      const index = f.git("write-tree");
      const result = f.run("2.0.0", ...flags);
      expect(result.status, result.stdout + result.stderr).toBe(0);
      const writes = f.commands().filter(line => /^git (add|commit|tag|push)( |$)|^gh /.test(line));
      expect(f.read("RELEASE_NOTES.txt")).toBe(f.notes);
      expect(fs.existsSync(path.join(f.repo, "CHANGELOG.md"))).toBe(false);
      expect(f.commands().some(line => /^git push |^gh /.test(line))).toBe(false);
      if (flags.includes("--dry-run")) {
        expect(f.read("pubspec.yaml")).toContain("version: 1.0.0\n");
        expect(f.read("assets/source/linux/rune.metainfo.xml")).toBe("<component><releases></releases></component>\n");
        expect(result.stdout).toContain("## Bugs\n\n• Preserve maintainer control over publication.\n\nFull Changelog: https://github.com/Losses/rune/compare/v1.0.0...v2.0.0");
      } else {
        expect(f.read("pubspec.yaml")).toContain("version: 2.0.0\n");
        expect(f.read("pubspec.yaml")).toContain("msix_version: 2.0.0.0\n");
        expect(f.read("assets/source/linux/rune.metainfo.xml")).toContain('<release version="2.0.0"');
        expect(f.read("assets/source/linux/rune.metainfo.xml")).toContain("<li>Preserve maintainer control over publication.</li>");
      }
      if (flags.includes("--skip-git") || flags.includes("--dry-run")) {
        expect(writes, "No Git writes or publication commands should be attempted").toEqual([]);
        expect(f.git("rev-parse", "HEAD")).toBe(head);
        expect(f.git("write-tree")).toBe(index);
        expect(f.git("tag", "--list")).toBe("v1.0.0");
      } else {
        expect(writes.some(line => line.startsWith("git add "))).toBe(true);
        expect(writes.some(line => line.startsWith("git commit "))).toBe(true);
        expect(writes).toContain("git tag v2.0.0");
        expect(f.git("rev-parse", "HEAD")).not.toBe(head);
        expect(f.git("rev-parse", "v2.0.0")).toBe(f.git("rev-parse", "HEAD"));
        expect(f.git("status", "--porcelain")).toBe("");
      }
    } finally { f.clean(); }
  });
}

test("skip-git preserves incomplete and absent gitignore", () => {
  const f = fixture();
  try {
    f.write(".gitignore", "build/\n");
    expect(f.run("2.0.0", "--skip-git").status).toBe(0);
    expect(f.read(".gitignore")).toBe("build/\n");
    fs.unlinkSync(path.join(f.repo, ".gitignore"));
    expect(f.run("2.0.1", "--skip-git").status).toBe(0);
    expect(fs.existsSync(path.join(f.repo, ".gitignore"))).toBe(false);
  } finally { f.clean(); }
});

test("skip-git template creation preserves gitignore", () => {
  const f = fixture();
  try {
    fs.unlinkSync(path.join(f.repo, "RELEASE_NOTES.txt"));
    f.write(".gitignore", "build/\n");
    expect(f.run("2.0.0", "--skip-git").status).toBe(0);
    expect(f.read(".gitignore")).toBe("build/\n");
    expect(f.read("pubspec.yaml")).toContain("version: 1.0.0\n");
  } finally { f.clean(); }
});

test("full release removes temporary notes and never creates CHANGELOG.md", () => {
  const f = fixture();
  try {
    const result = f.run("2.0.0-alpha.17");
    expect(result.status, result.stdout + result.stderr).toBe(0);
    expect(fs.existsSync(path.join(f.repo, "RELEASE_NOTES.txt"))).toBe(false);
    expect(fs.existsSync(path.join(f.repo, "CHANGELOG.md"))).toBe(false);
    expect(f.read("assets/source/linux/rune.metainfo.xml")).toContain('<release version="2.0.0-alpha.17"');
    expect(f.commands().some(line => line.startsWith("gh release create v2.0.0-alpha.17 "))).toBe(true);
    expect(f.git("status", "--porcelain")).toBe("");
  } finally { f.clean(); }
});
