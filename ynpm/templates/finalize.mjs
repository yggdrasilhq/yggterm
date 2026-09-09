#!/usr/bin/env node
/**
 * ynpm finalize — the yggdrasilhq-package postinstall.
 *
 * Best-effort fast path: copies the platform binary beside the JS entry
 * shim (bin/ytop.platform) so runs skip one exec hop, then verifies the
 * binary actually RUNS. The shim itself never depends on this succeeding —
 * it falls back to the platform sibling package, which npm places during
 * the same install.
 *
 * STRICT under the ynpm installer (YNPM_* env set): a fleet sync must fail
 * loudly when its binary cannot run. SOFT under a plain `npm i -g`: the
 * package tree is complete either way, and a postinstall failure would
 * roll back a working install over a mere optimization.
 */
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const pkgJson = JSON.parse(fs.readFileSync(path.join(__dirname, "package.json"), "utf8"));
const NAMES = Object.keys(pkgJson.bin || {});
const PACKAGE = process.env.YNPM_PACKAGE_NAME || pkgJson.name;
const PLATFORM = process.env.YNPM_PLATFORM || `${process.platform}-${process.arch}`;
const strict = Boolean(process.env.YNPM_PACKAGE_NAME && process.env.YNPM_PLATFORM);

function fail(message) {
  if (strict) {
    console.error(`ynpm finalize: ${message}`);
    process.exit(1);
  }
  console.warn(`ynpm finalize (non-fatal): ${message}`);
  process.exit(0);
}

if (!NAMES.length || !PACKAGE || !PLATFORM) {
  fail("could not derive YNPM_BIN_NAME / YNPM_PACKAGE_NAME / YNPM_PLATFORM");
}

const shortName = PACKAGE.includes("/") ? PACKAGE.split("/")[1] : PACKAGE;
// The platform package may sit NESTED under this package's own
// node_modules (npm 11 global layout) or as a flat sibling. Try both.
for (const name of NAMES) {
  const platformBinary = [
    path.join(__dirname, "node_modules", "@ygghq", `${shortName}-${PLATFORM}`, "bin", name),
    path.join(__dirname, "..", "node_modules", "@ygghq", `${shortName}-${PLATFORM}`, "bin", name),
  ].find((c) => fs.existsSync(c));
  const fastCopy = path.join(__dirname, "bin", name);
  if (!platformBinary) {
    fail(`${shortName}-${PLATFORM} is not installed beside this package - bin ${name} cannot be finalized`);
  }
  try {
    fs.mkdirSync(path.dirname(fastCopy), { recursive: true });
    fs.copyFileSync(platformBinary, fastCopy);
    fs.chmodSync(fastCopy, 0o755);
    execFileSync(fastCopy, ["--version"], { stdio: "ignore", timeout: 30000 });
  } catch (error) {
    // The package's platform dependency remains the fallback under npm; a
    // failed fast-path copy is not a reason to damage an otherwise complete
    // install.
    console.warn(`ynpm finalize (non-fatal): ${name} fast path unusable (${error.status ?? error.message ?? error})`);
  }
}
