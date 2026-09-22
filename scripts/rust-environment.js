const { spawnSync } = require("node:child_process");

function runRustc(arguments, workspaceDir) {
  const rustc = spawnSync("rustc", arguments, {
    cwd: workspaceDir,
    encoding: "utf8",
  });

  if (rustc.status !== 0) {
    throw new Error(rustc.stderr || "rustc is unavailable");
  }

  return rustc.stdout.trim();
}

function getRustHost(workspaceDir) {
  return runRustc(["-vV"], workspaceDir).match(/^host: (.+)$/m)?.[1] ?? "";
}

function getRustSysroot(workspaceDir) {
  return runRustc(["--print", "sysroot"], workspaceDir);
}

module.exports = { getRustHost, getRustSysroot };
