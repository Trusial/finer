import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const [destination, target] = process.argv.slice(2);
const lockfile = JSON.parse(fs.readFileSync("package-lock.json", "utf8"));
const notices = [];
function findNotices(directory) {
  return fs.readdirSync(directory, { withFileTypes: true })
    .filter((file) => file.isFile() && /^(licen[cs]e|copying|notice|ofl)([-_.]|$)/i.test(file.name))
    .map((file) => fs.readFileSync(path.join(directory, file.name), "utf8"));
}

for (const [location, entry] of Object.entries(lockfile.packages)) {
  const cssAndRuntime = ["node_modules/tailwindcss", "node_modules/vite"].includes(location);
  const shouldSkip = !location || (entry.dev && !cssAndRuntime) || !fs.existsSync(location);
  if (shouldSkip) {
    continue;
  }

  const manifest = JSON.parse(fs.readFileSync(path.join(location, "package.json"), "utf8"));
  notices.push(`${manifest.name} ${manifest.version} · ${manifest.license ?? "See project license"}\n${findNotices(location).join("\n\n")}`);
}

const metadata = JSON.parse(execFileSync("cargo", [
  "metadata",
  "--locked",
  "--format-version",
  "1",
  "--filter-platform",
  target,
], { encoding: "utf8", maxBuffer: 16 * 1024 * 1024 }));

for (const crate of metadata.packages) {
  if (metadata.workspace_members.includes(crate.id)) {
    continue;
  }

  const directory = path.dirname(crate.manifest_path);
  const texts = crate.license_file
    ? [fs.readFileSync(path.resolve(directory, crate.license_file), "utf8")]
    : findNotices(directory);
  notices.push(`${crate.name} ${crate.version} · ${crate.license ?? "See project license"}\n${texts.join("\n\n")}`);
}

fs.writeFileSync(destination, notices.join("\n\n========================================\n\n") + "\n");
