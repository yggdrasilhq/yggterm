#!/usr/bin/env node
/**
 * ynpm finalize — the yggterm-package postinstall.
 *
 * Copies this package's platform binary over the entry shim, marks it
 * executable, and verifies it RUNS (`--version`). Exits non-zero otherwise —
 * the ynpm install gate refuses a package whose binary cannot run, exactly
 * like the managed-CLI provisioner's publish gate.
 *
 * First-party script: this file ships in yggdrasilhq's own package, from the
 * same repo as the binary. The boundary is the vendor-script boundary: HOME
 * intact, no privilege escalation, stdin closed by the installer.
 */
const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");

const NAME = process.env.YNPM_BIN_NAME;
const PACKAGE = process.env.YNPM_PACKAGE_NAME; // e.g. @ygghq/ytop
const PLATFORM = process.env.YNPM_PLATFORM; // e.g. linux-x64

function fail(message) {
  console.error(`ynpm finalize: ${message}`);
  process.exit(1);
}

if (!NAME || !PACKAGE || !PLATFORM) {
  fail("YNPM_BIN_NAME / YNPM_PACKAGE_NAME / YNPM_PLATFORM must be set by the installer");
}

const shimPath = path.join(__dirname, "bin", NAME);
const platformPackage = path.join(
  __dirname, "..", "..", "..", `${PACKAGE}-${PLATFORM}`, "bin", NAME
);

if (!fs.existsSync(platformPackage)) {
  fail(`${PACKAGE}-${PLATFORM} is not installed next to this package — the platform binary is required on this machine`);
}

fs.mkdirSync(path.dirname(shimPath), { recursive: true });
fs.copyFileSync(platformPackage, shimPath);
fs.chmodSync(shimPath, 0o755);

try {
  execFileSync(shimPath, ["--version"], { stdio: "ignore", timeout: 30_000 });
} catch (error) {
  fail(`${NAME} does not run after finalize (${error.status ?? error.message})`);
}
