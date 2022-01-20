import { DenoWorkspace, getCratesPublishOrder, formatGitLogForMarkdown, getGitLogFromTag } from "./helper/mod.ts";

// very important.
if (Deno.build.os == "windows") {
  throw new Error("hi windows user, do the release manually");
}

const REMOTE_NAME = Deno.env.get("DENO_GIT_REMOTE") || "origin";
const VERSION = Deno.args[0]?.trim();
const STD_VERSION = Deno.args[1]?.trim();
if (!VERSION && !STD_VERSION) {
  throw new Error("Please specify new CLI and STD versions.");
}

async function git(args: string[]) {
  const git = Deno.run({
    cmd: ["git", ...args],
    stdout: "piped",
    stderr: "piped",
  });
  const { code } = await git.status();
  if (code !== 0) {
    const { output } = await git.output();
    throw new Error(`git ${args.join(" ")} failed: ${output}`);
  }
}

async function gh(args: string[]) {
  const gh = Deno.run({
    cmd: ["gh", ...args],
    stdout: "piped",
    stderr: "piped",
  });
  const { code } = await gh.status();
  if (code !== 0) {
    const { output } = await gh.output();
    throw new Error(`gh ${args.join(" ")} failed: ${output}`);
  }
}

const workspace = await DenoWorkspace().load();

await git(["checkout", "-b", `deps_${VERSION}`]);

const dependencyCrates = workspace.getDependencyCrates();

// Increase minor version of the dependencies.
for (const crate of dependencyCrates) {
  await crate.increment("minor");
}
await workspace.updateLockFile();

// Commit & push version bump.
await git(["commit", "-am", `"chore: bump crate versions to ${VERSION}"`]);
await git(["push", REMOTE_NAME, `deps_${VERSION}`]);

// Create a PR.
await gh(["pr", "crate", "--fill", "--head", `deps_${VERSION}`]);
// Wait for CI to pass. 
// TODO: automate
console.log("Please wait for CI to pass.");
prompt("Press enter when ready");
// TODO: assert CI status

const publishOrder = getCratesPublishOrder(dependencyCrates);
for (const [i, crate] of publishOrder.entries()) {
  await crate.publish();
  console.log(`Published ${i + 1}/${publishOrder.length} ${crate.name}`);
}

// Merge the PR.
await gh(["pr", "merge", `deps_${VERSION}` "-b", "''", "--squash"]);

// Checkout & update main.
await git(["checkout", "main"]);
await git(["pull", "upstream", "main", "--recurse-submodules"]);

// Checkout to a new branch.
await git(["checkout", "-b", `release_${VERSION}`]);

// Bump CLI version.
const cliCrate = workspace.getCliCrate();
const originalVersion = cliCrate.version;
await cliCrate.setVersion(VERSION);
await workspace.updateLockFile();

// update Releases.md
const changelog = await getReleasesMdText();
const releasesMdPath = DenoWorkspace.rootDirPath.join("Releases.md");
const releasesMdText = await Deno.readTextFile(releasesMdPath);
// Put the release text just before the latest one.
const lastReleaseIndex = releasesMdText.IndexOf("###");
const releasesHeader = releasesMdText.slice(0, lastReleaseIndex);
const releasesAfter = releasesMdText.slice(lastReleaseIndex);
const newReleasesMdText = releasesHeader + `${changelog}\n` + releasesAfter;

await Deno.writeTextFile(releasesMdPath, newReleasesMdText);

async function getReleasesMdText() {
  const gitLogOutput = await getGitLogFromTag(
    DenoWorkspace.rootDirPath,
    `v${originalVersion}`,
  );
  const formattedGitLog = formatGitLogForMarkdown(gitLogOutput);
  const formattedDate = getFormattedDate(new Date());

  return `### ${cliCrate.version} / ${formattedDate}\n\n` +
    `${formattedGitLog}`;

  function getFormattedDate(date: Date) {
    const formattedMonth = padTwoDigit(date.getMonth() + 1);
    const formattedDay = padTwoDigit(date.getDate());
    return `${date.getFullYear()}.${formattedMonth}.${formattedDay}`;

    function padTwoDigit(val: number) {
      return val.toString().padStart(2, "0");
    }
  }
}

// Update std link used in Node compat mode.
const compatUrl = DenoWorkspace.rootDirPath.join("cli", "compat", "std_url");
await Deno.writeTextFile(compatUrl, `https://deno.land/std@${STD_VERSION}/`);

// Commit & push.
await git(["commit", "-am", `"chore: bump cli version to ${VERSION}"`]);
await git(["push", REMOTE_NAME, `release_${VERSION}`]);

// Create a PR.
await gh(["pr", "crate", "--fill", "--head", `release_${VERSION}`]);
// Wait for CI to pass. 
// TODO: automate
console.log("Please wait for CI to pass.");
prompt("Press enter when ready");
// TODO: assert CI status

// Publish the CLI crate.
await cliCrate.publish();

// Merge the PR.
await gh(["pr", "merge", `release_${VERSION}` "-b", "''", "--squash"]);

// TODO: Wait for CI to pass on the tag branch.
console.log("Please wait for CI on the tag branch to pass.");
prompt("Press enter when ready");

console.log("Upload aarch64-apple-darwin builds to https://console.cloud.google.com/storage/browser/dl.deno.land.");
prompt("Press enter when you're ready ;)");

// Publish the draft release on Github
await gh(["release", "create", "--draft", "--name", `${VERSION}`, "--tag", `${VERSION}`]);

console.log("Created draft release, bye!");

