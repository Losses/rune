#!/usr/bin/env bun
/**
 * Rune Release & Version Bump Automation Tool
 * 
 * Usage:
 *   bun scripts/release.ts [version] [options]
 * 
 * Examples:
 *   bun scripts/release.ts 2.0.0-alpha.16
 *   bun scripts/release.ts --dry-run
 *   bun scripts/release.ts 2.0.0 --file ./custom_notes.txt
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { execFileSync, execSync } from "node:child_process";

const ROOT_DIR = path.resolve(__dirname, "..");
const DEFAULT_NOTES_FILES = ["RELEASE_NOTES.txt", "release_notes.txt", "upgrade.txt"];

// Supported release categories and aliases
const CATEGORY_MAP: Record<string, string> = {
  features: "Features",
  feature: "Features",
  "new features": "Features",
  bugs: "Bugs",
  bug: "Bugs",
  "bug fixes": "Bugs",
  "bug fix": "Bugs",
  fixes: "Bugs",
  fix: "Bugs",
  performance: "Performance",
  perf: "Performance",
  improvements: "Performance",
  improvement: "Performance",
  security: "Security",
  documentation: "Documentation",
  docs: "Documentation",
  doc: "Documentation",
  refactor: "Refactor",
  refactoring: "Refactor",
  "technical changes": "Refactor",
  internal: "Refactor",
  chore: "Refactor",
};

interface ReleaseCategory {
  title: string;
  items: string[];
}

interface ParsedNotes {
  filePath: string;
  categories: ReleaseCategory[];
}

interface VersionInfo {
  tagVersion: string;       // e.g. "v2.0.0-alpha.16"
  rawVersion: string;       // e.g. "2.0.0-alpha.16"
  numericVersion: string;   // e.g. "2.0.1016" (for pubspec / nix / msix)
  buildNumber: number;      // e.g. 1016
  msixVersion: string;      // e.g. "2.0.1016.0"
  marketingVersion: string; // e.g. "2.0.0" (for Apple)
  isPrerelease: boolean;
  dateStr: string;          // e.g. "2026-11-01"
}

interface CliOptions {
  version?: string;
  notesFile?: string;
  dryRun: boolean;
  noPush: boolean;
  skipGit: boolean;
  help: boolean;
}

function printHelp() {
  console.log(`
Rune Release Automation Tool
=============================

Basic Usage:
  1. Write your release notes in RELEASE_NOTES.txt (enforcing categorized format).
  2. Run: bun scripts/release.ts [version]

Arguments:
  [version]                  Target version (e.g. 2.0.0-alpha.16, v2.0.0-alpha.16, 2.0.0)
                             If omitted, auto-increments based on current version.

Options:
  -f, --file <path>          Path to release notes file (default: RELEASE_NOTES.txt)
  -d, --dry-run              Preview changes without committing, pushing, or deleting files
  --no-push                  Commit and tag locally, but do not push or create GitHub release
  --skip-git                 Only update version files and changelogs, skip all git operations
  -h, --help                 Show this help message

Release Notes File Format (Mandatory):
  Categories must be indicated with brackets [Category] or markdown headers ## Category.
  Allowed categories:
    - [Features] (or: New Features)
    - [Bugs] (or: Bug Fixes)
    - [Performance] (or: Improvements)
    - [Security]
    - [Documentation] (or: Docs)
    - [Refactor] (or: Technical Changes)

  Example RELEASE_NOTES.txt:
  -------------------------------------------------------------
  [Features]
  - Add support for new visualizer presets
  - Support importing .runescene files

  [Bugs]
  - Fix crash on Linux when opening menu
  - Keep inherited font metrics in acrylic popups
  -------------------------------------------------------------
`);
}

function parseCliArgs(args: string[]): CliOptions {
  const options: CliOptions = {
    dryRun: false,
    noPush: false,
    skipGit: false,
    help: false,
  };

  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === "-h" || arg === "--help") {
      options.help = true;
    } else if (arg === "-d" || arg === "--dry-run") {
      options.dryRun = true;
    } else if (arg === "--no-push") {
      options.noPush = true;
    } else if (arg === "--skip-git") {
      options.skipGit = true;
    } else if (arg === "-f" || arg === "--file") {
      if (i + 1 < args.length) {
        options.notesFile = args[++i];
      } else {
        throw new Error("Missing value for --file option");
      }
    } else if (arg.startsWith("-")) {
      throw new Error(`Unknown option: ${arg}`);
    } else if (!options.version) {
      options.version = arg;
    } else {
      throw new Error(`Unexpected positional argument: ${arg}`);
    }
  }

  return options;
}

function runCommand(command: string, options: { silent?: boolean } = {}): string {
  try {
    return execSync(command, {
      cwd: ROOT_DIR,
      encoding: "utf-8",
      stdio: options.silent ? "pipe" : ["pipe", "pipe", "pipe"],
    }).trim();
  } catch (error: any) {
    const stderr = error.stderr ? error.stderr.toString() : error.message;
    throw new Error(`Command failed: ${command}\n${stderr}`);
  }
}

function runFile(file: string, args: string[], options: { silent?: boolean } = {}): string {
  try {
    return execFileSync(file, args, {
      cwd: ROOT_DIR,
      encoding: "utf-8",
      stdio: options.silent ? "pipe" : ["pipe", "pipe", "pipe"],
    }).trim();
  } catch (error: any) {
    const stderr = error.stderr ? error.stderr.toString() : error.message;
    throw new Error(`Command failed: ${file} ${args.join(" ")}\n${stderr}`);
  }
}

function ensureGitignoreHasNotesFiles() {
  const gitignorePath = path.join(ROOT_DIR, ".gitignore");
  let content = "";
  if (fs.existsSync(gitignorePath)) {
    content = fs.readFileSync(gitignorePath, "utf-8");
  }

  const entriesToIgnore = ["RELEASE_NOTES.txt", "release_notes.txt", "*.release.txt", "upgrade.txt"];
  const missing = entriesToIgnore.filter(e => !content.includes(e));

  if (missing.length > 0) {
    const addition = `\n# Release notes draft (auto-ignored)\n${missing.join("\n")}\n`;
    fs.appendFileSync(gitignorePath, addition, "utf-8");
    console.log(`[gitignore] Added ${missing.join(", ")} to .gitignore`);
  }
}

function findReleaseNotesFile(specifiedPath?: string): string | null {
  if (specifiedPath) {
    const resolved = path.isAbsolute(specifiedPath) ? specifiedPath : path.join(ROOT_DIR, specifiedPath);
    if (!fs.existsSync(resolved)) {
      throw new Error(`Specified release notes file not found: ${specifiedPath}`);
    }
    return resolved;
  }

  for (const filename of DEFAULT_NOTES_FILES) {
    const candidate = path.join(ROOT_DIR, filename);
    if (fs.existsSync(candidate)) {
      return candidate;
    }
  }

  return null;
}

function createTemplateNotesFile(): string {
  const templatePath = path.join(ROOT_DIR, "RELEASE_NOTES.txt");
  const templateContent = `# Rune Release Notes Draft
# ---------------------------------------------------------------------------
# Instructions:
# 1. Fill in the changes under the appropriate category headers below.
# 2. Categories must be enclosed in brackets (e.g. [Features], [Bugs]).
# 3. Each item must start with '-' or '*'.
# 4. Allowed categories:
#      [Features], [Bugs], [Performance], [Security], [Documentation], [Refactor]
# 5. This file is automatically ignored by Git and will be deleted once published.
# ---------------------------------------------------------------------------

[Features]
- 

[Bugs]
- 
`;
  fs.writeFileSync(templatePath, templateContent, "utf-8");
  return templatePath;
}

function parseAndValidateReleaseNotes(filePath: string): ParsedNotes {
  const content = fs.readFileSync(filePath, "utf-8");
  const lines = content.split(/\r?\n/);

  const categories: ReleaseCategory[] = [];
  let currentCategory: ReleaseCategory | null = null;
  let currentItem: string | null = null;

  for (let idx = 0; idx < lines.length; idx++) {
    const lineNum = idx + 1;
    const rawLine = lines[idx];
    const trimmed = rawLine.trim();

    // Skip empty lines and comment lines
    if (!trimmed || trimmed.startsWith("#")) {
      continue;
    }

    // Check for category header: [Category], ## Category, or Category:
    const categoryHeaderMatch = trimmed.match(/^(?:\[([^\]]+)\]|##+\s+(.+)|([A-Za-z\s]+):)$/);
    if (categoryHeaderMatch) {
      if (currentItem && currentCategory) {
        currentCategory.items.push(currentItem);
        currentItem = null;
      }

      const rawCategoryName = (categoryHeaderMatch[1] || categoryHeaderMatch[2] || categoryHeaderMatch[3]).trim().toLowerCase();
      const normalizedTitle = CATEGORY_MAP[rawCategoryName];
      if (!normalizedTitle) {
        const allowed = Object.values(CATEGORY_MAP)
          .filter((v, i, a) => a.indexOf(v) === i)
          .join(", ");
        throw new Error(
          `[Format Error] Line ${lineNum}: Unrecognized category "${rawCategoryName}".\n` +
          `Allowed categories are: ${allowed}`
        );
      }

      let existing = categories.find(c => c.title === normalizedTitle);
      if (!existing) {
        existing = { title: normalizedTitle, items: [] };
        categories.push(existing);
      }
      currentCategory = existing;
      continue;
    }

    // Check for bullet item: - ..., * ..., • ...
    const bulletMatch = rawLine.match(/^\s*[-*•]\s+(.*)$/);
    if (bulletMatch) {
      if (!currentCategory) {
        throw new Error(
          `[Format Error] Line ${lineNum}: Found item "${trimmed}" outside any category!\n` +
          `Please place all items under a category header (e.g. [Features] or [Bugs]).`
        );
      }
      if (currentItem) {
        currentCategory.items.push(currentItem);
      }
      const itemText = bulletMatch[1].trim();
      currentItem = itemText;
      continue;
    }

    // Continuation of multiline item
    if (currentItem && (rawLine.startsWith("  ") || rawLine.startsWith("\t"))) {
      currentItem += ` ${trimmed}`;
      continue;
    }

    // Unclassified / malformed text
    throw new Error(
      `[Format Error] Line ${lineNum}: Unrecognized line format:\n  "${rawLine}"\n` +
      `Items must start with '-' or '*' and be listed under a valid category header.`
    );
  }

  if (currentItem && currentCategory) {
    currentCategory.items.push(currentItem);
  }

  // Filter out empty items
  for (const cat of categories) {
    cat.items = cat.items.map(i => i.trim()).filter(i => i.length > 0);
  }

  const activeCategories = categories.filter(c => c.items.length > 0);
  if (activeCategories.length === 0) {
    throw new Error(
      `[Format Error] No release items found in ${filePath}.\n` +
      `Please add at least one bullet item under a category header (e.g. [Features] or [Bugs]).`
    );
  }

  return {
    filePath,
    categories: activeCategories,
  };
}

function getLatestTagOrVersion(): string {
  try {
    const tag = runCommand("git describe --tags --abbrev=0", { silent: true });
    if (tag) return tag;
  } catch {}

  try {
    const tags = runCommand("git tag --sort=-v:refname", { silent: true }).split("\n");
    if (tags[0]) return tags[0];
  } catch {}

  // Fallback to pubspec.yaml
  const pubspecPath = path.join(ROOT_DIR, "pubspec.yaml");
  if (fs.existsSync(pubspecPath)) {
    const content = fs.readFileSync(pubspecPath, "utf-8");
    const match = content.match(/^version:\s*([^\s]+)/m);
    if (match) return match[1];
  }

  return "v2.0.0-alpha.15";
}

function computeVersionInfo(targetVersionInput?: string): VersionInfo {
  let target = targetVersionInput;
  if (!target) {
    const latest = getLatestTagOrVersion();
    const cleanLatest = latest.startsWith("v") ? latest.slice(1) : latest;
    // Attempt auto-increment
    const alphaMatch = cleanLatest.match(/^(\d+\.\d+\.\d+)-alpha\.(\d+)$/);
    if (alphaMatch) {
      const nextNum = parseInt(alphaMatch[2], 10) + 1;
      target = `${alphaMatch[1]}-alpha.${nextNum}`;
    } else {
      const semverMatch = cleanLatest.match(/^(\d+)\.(\d+)\.(\d+)$/);
      if (semverMatch) {
        const nextPatch = parseInt(semverMatch[3], 10) + 1;
        target = `${semverMatch[1]}.${semverMatch[2]}.${nextPatch}`;
      } else {
        throw new Error(`Unable to determine next version from "${latest}". Please explicitly specify version argument.`);
      }
    }
    console.log(`[version] Auto-selected next version: ${target} (previous: ${latest})`);
  }

  const rawVersion = target.startsWith("v") ? target.slice(1) : target;
  const tagVersion = target.startsWith("v") ? target : `v${target}`;
  const isPrerelease = /(alpha|beta|rc|dev|pre)/i.test(rawVersion);

  let numericVersion = rawVersion;
  let buildNumber = 1;
  let msixVersion = `${rawVersion}.0`;
  let marketingVersion = rawVersion;

  // Pattern 1: X.Y.Z-alpha.N -> numeric = X.Y.(1000 + N), build = 1000 + N
  const alphaMatch = rawVersion.match(/^(\d+)\.(\d+)\.(\d+)-alpha\.(\d+)$/);
  if (alphaMatch) {
    const major = alphaMatch[1];
    const minor = alphaMatch[2];
    const patch = alphaMatch[3];
    const alphaNum = parseInt(alphaMatch[4], 10);
    buildNumber = 1000 + alphaNum;
    numericVersion = `${major}.${minor}.${buildNumber}`;
    msixVersion = `${numericVersion}.0`;
    marketingVersion = `${major}.${minor}.${patch}`;
  } else {
    // Pattern 2: X.Y.Z-beta.N -> numeric = X.Y.(2000 + N)
    const betaMatch = rawVersion.match(/^(\d+)\.(\d+)\.(\d+)-beta\.(\d+)$/);
    if (betaMatch) {
      const major = betaMatch[1];
      const minor = betaMatch[2];
      const patch = betaMatch[3];
      const betaNum = parseInt(betaMatch[4], 10);
      buildNumber = 2000 + betaNum;
      numericVersion = `${major}.${minor}.${buildNumber}`;
      msixVersion = `${numericVersion}.0`;
      marketingVersion = `${major}.${minor}.${patch}`;
    } else {
      // Pattern 3: X.Y.Z-rc.N -> numeric = X.Y.(4000 + N)
      const rcMatch = rawVersion.match(/^(\d+)\.(\d+)\.(\d+)-rc\.(\d+)$/);
      if (rcMatch) {
        const major = rcMatch[1];
        const minor = rcMatch[2];
        const patch = rcMatch[3];
        const rcNum = parseInt(rcMatch[4], 10);
        buildNumber = 4000 + rcNum;
        numericVersion = `${major}.${minor}.${buildNumber}`;
        msixVersion = `${numericVersion}.0`;
        marketingVersion = `${major}.${minor}.${patch}`;
      } else {
        // Pattern 4: Standard semver X.Y.Z
        const semverMatch = rawVersion.match(/^(\d+)\.(\d+)\.(\d+)$/);
        if (semverMatch) {
          const major = semverMatch[1];
          const minor = semverMatch[2];
          const patch = semverMatch[3];
          buildNumber = parseInt(patch, 10) || 1;
          numericVersion = `${major}.${minor}.${patch}`;
          msixVersion = `${major}.${minor}.${patch}.0`;
          marketingVersion = `${major}.${minor}.${patch}`;
        }
      }
    }
  }

  const now = new Date();
  const year = now.getFullYear();
  const month = String(now.getMonth() + 1).padStart(2, "0");
  const day = String(now.getDate()).padStart(2, "0");
  const dateStr = `${year}-${month}-${day}`;

  return {
    tagVersion,
    rawVersion,
    numericVersion,
    buildNumber,
    msixVersion,
    marketingVersion,
    isPrerelease,
    dateStr,
  };
}

function escapeXml(str: string): string {
  return str
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;");
}

function updateMetainfoXml(info: VersionInfo, categories: ReleaseCategory[], dryRun: boolean): string {
  const metainfoPath = path.join(ROOT_DIR, "assets/source/linux/rune.metainfo.xml");
  if (!fs.existsSync(metainfoPath)) {
    console.warn(`[warning] ${metainfoPath} not found, skipping.`);
    return metainfoPath;
  }

  const content = fs.readFileSync(metainfoPath, "utf-8");

  // Build the XML release block
  const lines: string[] = [];
  lines.push(`    <release version="${info.rawVersion}" date="${info.dateStr}">`);
  lines.push(`      <description>`);
  for (const cat of categories) {
    lines.push(`        <p>${escapeXml(cat.title)}</p>`);
    lines.push(`        <ul>`);
    for (const item of cat.items) {
      const formatted = item.endsWith(".") ? item : `${item}.`;
      lines.push(`          <li>${escapeXml(formatted)}</li>`);
    }
    lines.push(`        </ul>`);
  }
  lines.push(`      </description>`);
  lines.push(`    </release>`);
  const newReleaseBlock = lines.join("\n");

  const releasesTag = "<releases>";
  const insertIndex = content.indexOf(releasesTag);
  if (insertIndex === -1) {
    throw new Error(`Could not find <releases> tag in ${metainfoPath}`);
  }

  const updatedContent =
    content.slice(0, insertIndex + releasesTag.length) +
    "\n" +
    newReleaseBlock +
    content.slice(insertIndex + releasesTag.length);

  if (!dryRun) {
    fs.writeFileSync(metainfoPath, updatedContent, "utf-8");
  }
  console.log(`[metainfo] Updated ${metainfoPath} with release ${info.rawVersion}`);
  return metainfoPath;
}

function updatePubspecYaml(info: VersionInfo, dryRun: boolean): string {
  const pubspecPath = path.join(ROOT_DIR, "pubspec.yaml");
  let content = fs.readFileSync(pubspecPath, "utf-8");

  // Replace version: ...
  content = content.replace(/^version:\s*.*$/m, `version: ${info.numericVersion}`);

  // Replace msix_version: ...
  content = content.replace(/^(\s*msix_version:\s*).*$/m, `$1${info.msixVersion}`);

  if (!dryRun) {
    fs.writeFileSync(pubspecPath, content, "utf-8");
  }
  console.log(`[pubspec] Updated version: ${info.numericVersion}, msix_version: ${info.msixVersion}`);
  return pubspecPath;
}

function updateInnoSetupScript(info: VersionInfo, dryRun: boolean): string | null {
  const issPath = path.join(ROOT_DIR, "rune.iss");
  if (!fs.existsSync(issPath)) {
    return null;
  }

  let content = fs.readFileSync(issPath, "utf-8");
  content = content.replace(/^#define MyAppVersion "[^"]+"/m, `#define MyAppVersion "${info.rawVersion}"`);

  if (!dryRun) {
    fs.writeFileSync(issPath, content, "utf-8");
  }
  console.log(`[windows] Updated ${issPath} (#define MyAppVersion "${info.rawVersion}")`);
  return issPath;
}

function updateNixPackages(info: VersionInfo, dryRun: boolean): string[] {
  const files = [
    path.join(ROOT_DIR, "nix/packages/rune.nix"),
    path.join(ROOT_DIR, "nix/platforms/windows/package.nix"),
  ];

  for (const file of files) {
    if (fs.existsSync(file)) {
      let content = fs.readFileSync(file, "utf-8");
      content = content.replace(/^(\s*version\s*=\s*)"[^"]+";/m, `$1"${info.numericVersion}";`);
      if (!dryRun) {
        fs.writeFileSync(file, content, "utf-8");
      }
      console.log(`[nix] Updated ${file} -> version = "${info.numericVersion}"`);
    }
  }
  return files;
}

function updateAppleProjectConfigs(info: VersionInfo, dryRun: boolean): string[] {
  const pbxFiles = [
    path.join(ROOT_DIR, "macos/Runner.xcodeproj/project.pbxproj"),
    path.join(ROOT_DIR, "ios/Runner.xcodeproj/project.pbxproj"),
  ];

  const updatedFiles: string[] = [];

  for (const file of pbxFiles) {
    if (fs.existsSync(file)) {
      let content = fs.readFileSync(file, "utf-8");
      // Replace MARKETING_VERSION = ...;
      content = content.replace(/MARKETING_VERSION = [^;]+;/g, `MARKETING_VERSION = ${info.marketingVersion};`);
      // Replace CURRENT_PROJECT_VERSION = <numbers>;
      content = content.replace(/CURRENT_PROJECT_VERSION = \d+;/g, `CURRENT_PROJECT_VERSION = ${info.buildNumber};`);

      if (!dryRun) {
        fs.writeFileSync(file, content, "utf-8");
      }
      console.log(`[apple] Updated ${file} (MARKETING_VERSION=${info.marketingVersion}, CURRENT_PROJECT_VERSION=${info.buildNumber})`);
      updatedFiles.push(file);
    }
  }
  return updatedFiles;
}

function updateAndroidLocalProperties(info: VersionInfo, dryRun: boolean): string | null {
  const localPropsPath = path.join(ROOT_DIR, "android/local.properties");
  if (!fs.existsSync(localPropsPath)) {
    return null;
  }

  let content = fs.readFileSync(localPropsPath, "utf-8");
  if (content.includes("flutter.versionName=")) {
    content = content.replace(/^flutter\.versionName=.*$/m, `flutter.versionName=${info.numericVersion}`);
  } else {
    content += `\nflutter.versionName=${info.numericVersion}`;
  }

  if (content.includes("flutter.versionCode=")) {
    content = content.replace(/^flutter\.versionCode=.*$/m, `flutter.versionCode=${info.buildNumber}`);
  } else {
    content += `\nflutter.versionCode=${info.buildNumber}`;
  }

  if (!dryRun) {
    fs.writeFileSync(localPropsPath, content, "utf-8");
  }
  console.log(`[android] Updated ${localPropsPath} (versionName=${info.numericVersion}, versionCode=${info.buildNumber})`);
  return localPropsPath;
}

function updateChangelogMd(info: VersionInfo, categories: ReleaseCategory[], dryRun: boolean): string {
  const changelogPath = path.join(ROOT_DIR, "CHANGELOG.md");
  let existingContent = "";
  if (fs.existsSync(changelogPath)) {
    existingContent = fs.readFileSync(changelogPath, "utf-8");
  }

  const lines: string[] = [];
  lines.push(`## [${info.rawVersion}] - ${info.dateStr}\n`);
  for (const cat of categories) {
    lines.push(`### ${cat.title}`);
    for (const item of cat.items) {
      lines.push(`- ${item}`);
    }
    lines.push("");
  }

  const newSection = lines.join("\n");
  let finalContent = "";
  if (existingContent.startsWith("# Changelog")) {
    finalContent = existingContent.replace(/^# Changelog\s*\n+/, `# Changelog\n\n${newSection}\n`);
  } else {
    finalContent = `# Changelog\n\n${newSection}\n${existingContent}`.trim() + "\n";
  }

  if (!dryRun) {
    fs.writeFileSync(changelogPath, finalContent, "utf-8");
  }
  console.log(`[changelog] Updated ${changelogPath}`);
  return changelogPath;
}

function buildGitHubReleaseNotes(info: VersionInfo, categories: ReleaseCategory[], prevTag: string): string {
  const parts: string[] = [];

  if (info.isPrerelease) {
    parts.push(`> [!CAUTION]`);
    parts.push(`> Purchased users should not mix test versions and stable versions on their devices. All test versions may automatically upgrade databases and configuration files, potentially causing incompatibility with older versions. This could result in you being unable to use the stable version again.\n`);
  }

  for (const cat of categories) {
    parts.push(`## ${cat.title}`);
    for (const item of cat.items) {
      parts.push(`• ${item}`);
    }
    parts.push("");
  }

  if (prevTag) {
    parts.push(`Full Changelog: https://github.com/Losses/rune/compare/${prevTag}...${info.tagVersion}\n`);
  }

  return parts.join("\n");
}

async function main() {
  const args = process.argv.slice(2);
  const options = parseCliArgs(args);

  if (options.help) {
    printHelp();
    process.exit(0);
  }

  console.log("==> Rune Release Automation Tool");

  // 1. Ensure .gitignore has notes files
  ensureGitignoreHasNotesFiles();

  // 2. Locate or create release notes text file
  const notesPath = findReleaseNotesFile(options.notesFile);
  if (!notesPath) {
    const createdPath = createTemplateNotesFile();
    console.log(`\n[notice] Release notes file not found.`);
    console.log(`Created template: ${createdPath}`);
    console.log(`Please edit this file with your categorized changes, then re-run:`);
    console.log(`  bun scripts/release.ts ${options.version || ""}\n`);
    process.exit(0);
  }

  // 3. Parse and strictly validate release notes
  console.log(`[notes] Reading and validating: ${notesPath}`);
  const parsed = parseAndValidateReleaseNotes(notesPath);
  console.log(`[notes] Validation passed. Categories detected: ${parsed.categories.map(c => `${c.title} (${c.items.length})`).join(", ")}`);

  // 4. Determine version information
  const prevTag = getLatestTagOrVersion();
  const info = computeVersionInfo(options.version);
  console.log(`--------------------------------------------------`);
  console.log(`Tag Version:       ${info.tagVersion}`);
  console.log(`Raw Version:       ${info.rawVersion}`);
  console.log(`Numeric Version:   ${info.numericVersion}`);
  console.log(`Build Number:      ${info.buildNumber}`);
  console.log(`MSIX Version:      ${info.msixVersion}`);
  console.log(`Marketing Version: ${info.marketingVersion}`);
  console.log(`Is Prerelease:     ${info.isPrerelease ? "YES" : "NO"}`);
  console.log(`Release Date:      ${info.dateStr}`);
  console.log(`--------------------------------------------------`);

  if (options.dryRun) {
    console.log("\n[DRY RUN MODE] Simulating all changes...\n");
  }

  // 5. Update version files
  const changedFiles: string[] = [];
  changedFiles.push(updatePubspecYaml(info, options.dryRun));
  changedFiles.push(...updateNixPackages(info, options.dryRun));
  const innoSetupFile = updateInnoSetupScript(info, options.dryRun);
  if (innoSetupFile) changedFiles.push(innoSetupFile);
  changedFiles.push(updateMetainfoXml(info, parsed.categories, options.dryRun));
  changedFiles.push(...updateAppleProjectConfigs(info, options.dryRun));
  // Update android local.properties if it exists (for local gradle builds, ignored by git)
  updateAndroidLocalProperties(info, options.dryRun);
  changedFiles.push(updateChangelogMd(info, parsed.categories, options.dryRun));

  // 6. Build GitHub release body
  const releaseBody = buildGitHubReleaseNotes(info, parsed.categories, prevTag);

  if (options.dryRun || options.skipGit) {
    console.log("\n=== Generated GitHub Release Notes Preview ===");
    console.log(releaseBody);
    console.log("==============================================\n");
    if (options.dryRun) {
      console.log(`[dry-run] Would execute:`);
      console.log(`  git add ${changedFiles.join(" ")}`);
      console.log(`  git commit -m "chore: Bump to \\\`${info.tagVersion}\\\`"`);
      console.log(`  git tag ${info.tagVersion}`);
      if (!options.noPush) {
        console.log(`  git push origin HEAD`);
        console.log(`  git push origin ${info.tagVersion}`);
        console.log(`  gh release create ${info.tagVersion} --title "${info.tagVersion}" ${info.isPrerelease ? "--prerelease" : "--latest"} ...`);
        console.log(`  rm ${notesPath}`);
      }
      console.log(`\n[dry-run] Dry run completed successfully.`);
      return;
    }
  }

  // 7. Git commit and tag
  console.log("\n[git] Staging modified version files...");
  for (const file of changedFiles) {
    if (fs.existsSync(file)) {
      runFile("git", ["add", path.relative(ROOT_DIR, file)]);
    }
  }

  const commitMsg = `chore: Bump to \`${info.tagVersion}\``;
  console.log(`[git] Committing: ${commitMsg}`);
  runFile("git", ["commit", "-m", commitMsg]);

  console.log(`[git] Tagging: ${info.tagVersion}`);
  runFile("git", ["tag", info.tagVersion]);

  if (options.noPush) {
    console.log(`\n[done] Local version bump and tag created successfully. (--no-push active)`);
    return;
  }

  // 8. Git push and GitHub release
  console.log("[git] Pushing commit to remote...");
  runFile("git", ["push", "origin", "HEAD"]);

  console.log(`[git] Pushing tag ${info.tagVersion} to remote...`);
  runFile("git", ["push", "origin", info.tagVersion]);

  console.log("[gh] Creating GitHub release...");
  const tempNotesPath = path.join(ROOT_DIR, `.release_notes_tmp_${Date.now()}.md`);
  fs.writeFileSync(tempNotesPath, releaseBody, "utf-8");

  try {
    const prereleaseFlag = info.isPrerelease ? "--prerelease" : "--latest";
    runFile("gh", [
      "release",
      "create",
      info.tagVersion,
      "--title",
      info.tagVersion,
      "--notes-file",
      tempNotesPath,
      prereleaseFlag,
    ]);
    console.log(`[gh] Successfully published GitHub release: ${info.tagVersion}`);
  } finally {
    if (fs.existsSync(tempNotesPath)) {
      fs.unlinkSync(tempNotesPath);
    }
  }

  // 9. Clean up user notes file after successful release
  if (fs.existsSync(notesPath)) {
    fs.unlinkSync(notesPath);
    console.log(`[clean] Successfully removed release notes file: ${notesPath}`);
  }

  console.log(`\n🎉 Release ${info.tagVersion} finished successfully!`);
}

main().catch((err) => {
  console.error(`\n❌ Error: ${err.message}`);
  process.exit(1);
});
