const { copyFileSync, existsSync, mkdirSync } = require("node:fs");
const { resolve } = require("node:path");

const { getRustHost, getRustSysroot } = require("./rust-environment");

const workspaceDir = resolve(__dirname, "..");
const profile = process.argv.includes("--release") ? "release" : "debug";
const libraryNames = {
  darwin: "libkbc_node.dylib",
  linux: "libkbc_node.so",
  win32: "kbc_node.dll",
};
const libraryName = libraryNames[process.platform];

if (!libraryName) {
  throw new Error(`Unsupported native platform: ${process.platform}`);
}

const sourcePath = resolve(workspaceDir, "target", profile, libraryName);
const outputDir = resolve(workspaceDir, "native");
const outputPath = resolve(outputDir, "kbc_node.node");

if (!existsSync(sourcePath)) {
  throw new Error(`Native library was not found: ${sourcePath}`);
}

mkdirSync(outputDir, { recursive: true });
copyFileSync(sourcePath, outputPath);

if (process.platform === "win32") {
  const host = getRustHost(workspaceDir);

  if (host.endsWith("-gnullvm")) {
    const unwindPath = resolve(
      getRustSysroot(workspaceDir),
      "lib",
      "rustlib",
      host,
      "bin",
      "libunwind.dll",
    );
    copyFileSync(unwindPath, resolve(outputDir, "libunwind.dll"));
  }
}

process.stdout.write(`Copied native module to ${outputPath}\n`);
