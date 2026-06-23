#!/usr/bin/env node

// ctl - AI Dev Control Plane CLI
// Platform-native binary wrapper

const path = require("path");
const spawn = require("child_process").spawnSync;

const platform = process.platform;
const arch = process.arch;

let binaryName = "ctl";
if (platform === "win32") binaryName = "ctl.exe";

const platformDir = getPlatformDir();
const binaryPath = path.join(__dirname, "..", "platforms", platformDir, binaryName);

const fs = require("fs");
if (!fs.existsSync(binaryPath)) {
  console.error(
    `ctl: unsupported platform '${platform}-${arch}' or binary not installed.\n` +
    `Run 'npm rebuild @velo-ai/ctl' or install the correct platform package.`
  );
  process.exit(1);
}

const result = spawn(binaryPath, process.argv.slice(2), {
  stdio: "inherit",
  env: process.env,
  windowsHide: true,
});

process.exit(result.status ?? 1);

function getPlatformDir() {
  const tuples = {
    "win32-x64": "win32-x64-msvc",
    "darwin-x64": "darwin-x64",
    "darwin-arm64": "darwin-arm64",
    "linux-x64": "linux-x64-gnu",
    "linux-arm64": "linux-arm64-gnu",
  };
  return tuples[`${platform}-${arch}`] || `${platform}-${arch}`;
}
