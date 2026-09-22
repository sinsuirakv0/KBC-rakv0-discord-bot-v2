const { mkdirSync, writeFileSync } = require("node:fs");
const { resolve } = require("node:path");
const { spawnSync } = require("node:child_process");

const { getRustHost } = require("./rust-environment");

const workspaceDir = resolve(__dirname, "..");
const cargoArguments = process.argv.slice(2);

if (cargoArguments.length === 0) {
  throw new Error("Cargo arguments are required");
}

const environment = { ...process.env };

if (process.platform === "win32") {
  const host = getRustHost(workspaceDir);

  if (host.endsWith("-gnullvm")) {
    const libraryDir = resolve(workspaceDir, "target", "libnode");
    mkdirSync(libraryDir, { recursive: true });
    writeFileSync(resolve(libraryDir, "libnode.dll"), "");
    writeFileSync(resolve(libraryDir, "libnode.dll.a"), "!<arch>\n", "ascii");
    environment.LIBNODE_PATH = libraryDir;
  }
}

const cargo = spawnSync("cargo", cargoArguments, {
  cwd: workspaceDir,
  env: environment,
  stdio: "inherit",
});

process.exit(cargo.status ?? 1);
