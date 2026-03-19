#!/usr/bin/env node

const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawn } = require("node:child_process");

const pkg = require("../package.json");

function targetLabel() {
  if (process.platform === "linux" && process.arch === "x64") {
    return "linux-x86_64";
  }

  if (process.platform === "darwin" && process.arch === "x64") {
    return "macos-x86_64";
  }

  if (process.platform === "darwin" && process.arch === "arm64") {
    return "macos-aarch64";
  }

  if (process.platform === "win32" && process.arch === "x64") {
    return "windows-x86_64";
  }

  throw new Error(
    `unsupported platform for npm launcher: ${process.platform}-${process.arch}`
  );
}

function cacheRoot(version) {
  const base =
    process.env.XDG_CACHE_HOME || path.join(os.homedir(), ".cache");
  return path.join(base, "yggterm", "npm", version);
}

async function downloadBinary(version, destination) {
  const repo = process.env.YGGTERM_REPO || "yggdrasilhq/yggterm";
  const label = targetLabel();
  const suffix = label.startsWith("windows-") ? ".exe" : "";
  const url = `https://github.com/${repo}/releases/download/v${version}/yggterm-${label}${suffix}`;
  const response = await fetch(url, { redirect: "follow" });

  if (!response.ok) {
    throw new Error(`failed to download ${url}: ${response.status} ${response.statusText}`);
  }

  fs.mkdirSync(path.dirname(destination), { recursive: true });
  const tmp = `${destination}.tmp`;
  const data = Buffer.from(await response.arrayBuffer());
  fs.writeFileSync(tmp, data, { mode: 0o755 });
  fs.renameSync(tmp, destination);
  fs.chmodSync(destination, 0o755);
}

async function ensureBinary() {
  const version = pkg.version;
  const label = targetLabel();
  const suffix = label.startsWith("windows-") ? ".exe" : "";
  const location = path.join(cacheRoot(version), `yggterm-${label}${suffix}`);

  if (!fs.existsSync(location)) {
    await downloadBinary(version, location);
  }

  return location;
}

async function main() {
  const binary = await ensureBinary();
  const child = spawn(binary, process.argv.slice(2), {
    stdio: "inherit",
    env: process.env,
  });

  child.on("exit", (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
      return;
    }

    process.exit(code ?? 1);
  });
}

main().catch((error) => {
  console.error(`yggterm npm launcher error: ${error.message}`);
  process.exit(1);
});
