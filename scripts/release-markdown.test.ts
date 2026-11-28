import { test, expect } from "bun:test";
import { execFileSync, spawnSync } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

const initialVersion = "name: rune\nversion: 1.0.0\n";
const initialMetainfo = "<component><releases></releases></component>\n";

// Run with: bun test scripts/release-markdown.test.ts
// Exercise the real CLI; --no-push keeps commits and tags in disposable repos.
function release(notes: string) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "rune-markdown-"));
  const env = {
    ...process.env,
    GIT_CONFIG_GLOBAL: path.join(root, "gitconfig"),
    GIT_CONFIG_NOSYSTEM: "1",
    GIT_AUTHOR_NAME: "Release Test",
    GIT_AUTHOR_EMAIL: "release@example.invalid",
    GIT_COMMITTER_NAME: "Release Test",
    GIT_COMMITTER_EMAIL: "release@example.invalid",
  };
  try {
    fs.mkdirSync(path.join(root, "scripts"));
    fs.copyFileSync(path.join(import.meta.dir, "release.ts"), path.join(root, "scripts/release.ts"));
    fs.writeFileSync(path.join(root, "pubspec.yaml"), initialVersion);
    fs.mkdirSync(path.join(root, "assets/source/linux"), { recursive: true });
    fs.writeFileSync(path.join(root, "assets/source/linux/rune.metainfo.xml"), initialMetainfo);
    fs.writeFileSync(path.join(root, ".gitignore"), "RELEASE_NOTES.txt\nrelease_notes.txt\n*.release.txt\nupgrade.txt\n");
    const git = (...args: string[]) => execFileSync("git", args, { cwd: root, env, encoding: "utf8" });
    git("init", "-q");
    git("add", ".");
    git("commit", "-qm", "Initial fixture");
    fs.writeFileSync(path.join(root, "RELEASE_NOTES.txt"), notes);
    const result = spawnSync(process.execPath, [path.join(root, "scripts/release.ts"), "2.0.0", "--no-push"], {
      cwd: root, env, encoding: "utf8", timeout: 30_000,
    });
    expect(result.error).toBeUndefined();
    const changelog = path.join(root, "CHANGELOG.md");
    return {
      status: result.status,
      output: result.stdout + result.stderr,
      changelogExists: fs.existsSync(changelog),
      version: fs.readFileSync(path.join(root, "pubspec.yaml"), "utf8"),
      metainfo: fs.readFileSync(path.join(root, "assets/source/linux/rune.metainfo.xml"), "utf8"),
    };
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
}

test("Markdown category can be the first release category", () => {
  const result = release("## Features\n- Add a scene\n");
  expect(result.status, result.output).toBe(0);
  expect(result.changelogExists).toBe(false);
  expect(result.metainfo).toContain("<p>Features</p>\n        <ul>\n          <li>Add a scene.</li>");
});

test("Markdown heading changes category after a bracketed heading", () => {
  const result = release("[Features]\n- Add a scene\n## Bugs\n- Fix a crash\n");
  expect(result.status, result.output).toBe(0);
  expect(result.changelogExists).toBe(false);
  expect(result.metainfo).toContain("<p>Features</p>\n        <ul>\n          <li>Add a scene.</li>\n        </ul>\n        <p>Bugs</p>\n        <ul>\n          <li>Fix a crash.</li>");
});

test("aliases, repeated categories, colon headings and comments retain their behavior", () => {
  const result = release("# Release notes\n\n## New Features\n- First feature\n  continued description\n# An ordinary comment\n[Features]\n- Second feature\n### Bug Fixes\n- Fix a crash\nDocs:\n- Explain setup\n");
  expect(result.status, result.output).toBe(0);
  expect(result.changelogExists).toBe(false);
  expect(result.metainfo).toContain("<p>Features</p>\n        <ul>\n          <li>First feature continued description.</li>\n          <li>Second feature.</li>");
  expect(result.metainfo).toContain("<p>Bugs</p>\n        <ul>\n          <li>Fix a crash.</li>");
  expect(result.metainfo).toContain("<p>Documentation</p>\n        <ul>\n          <li>Explain setup.</li>");
  expect(result.metainfo).not.toContain("ordinary comment");
  expect(result.metainfo.match(/<p>Features<\/p>/g)).toHaveLength(1);
});

test("Markdown headings accept optional closing hashes and whitespace", () => {
  const result = release("## Features ##\n- Add a scene\n### Bug Fixes ###   \n- Fix a crash\n## Docs\t##\n- Explain setup\n");
  expect(result.status, result.output).toBe(0);
  expect(result.changelogExists).toBe(false);
  expect(result.metainfo).toContain("<p>Features</p>\n        <ul>\n          <li>Add a scene.</li>");
  expect(result.metainfo).toContain("<p>Bugs</p>\n        <ul>\n          <li>Fix a crash.</li>");
  expect(result.metainfo).toContain("<p>Documentation</p>\n        <ul>\n          <li>Explain setup.</li>");
});

for (const heading of ["## Unknown", "[Unknown]", "Unknown:", "## constructor", "[constructor]", "constructor:", "## __proto__", "## Features##", "[Features ##]"]) {
  test(`reject unknown category ${heading}`, () => {
    const result = release(`[Features]\n- Valid feature\n${heading}\n- Invalid category item\n`);
    expect(result.status).toBe(1);
    expect(result.output).toContain("Unrecognized category");
    expect(result.changelogExists).toBe(false);
    expect(result.version).toBe(initialVersion);
    expect(result.metainfo).toBe(initialMetainfo);
  });
}

test("a comment does not establish a category for a bullet", () => {
  const result = release("# A comment\n- Orphan item\n");
  expect(result.status).toBe(1);
  expect(result.output).toContain("outside any category");
});
