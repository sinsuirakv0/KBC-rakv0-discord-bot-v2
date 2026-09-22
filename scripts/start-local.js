const { existsSync } = require("node:fs");
const { resolve } = require("node:path");
const { spawn } = require("node:child_process");

const repositoryRoot = resolve(__dirname, "..");
const configuredEnvFile = process.env.KBC_LOCAL_ENV_FILE?.trim();
const localEnvFile = resolve(repositoryRoot, ".env");
const sourceEnvFile = resolve(repositoryRoot, "../KBC-rakv0-discord-bot/.env");
const envFile = configuredEnvFile
  ? resolve(process.cwd(), configuredEnvFile)
  : existsSync(localEnvFile)
    ? localEnvFile
    : sourceEnvFile;

if (!process.env.DISCORD_TOKEN?.trim()) {
  if (!existsSync(envFile)) {
    throw new Error(
      `Local environment file was not found: ${envFile}. Set DISCORD_TOKEN or KBC_LOCAL_ENV_FILE.`,
    );
  }
  process.loadEnvFile(envFile);
}

if (!process.env.DISCORD_TOKEN?.trim()) {
  throw new Error("DISCORD_TOKEN is required for local startup.");
}

const child = spawn(
  process.execPath,
  [resolve(repositoryRoot, "apps/discord/dist/index.js")],
  {
    cwd: repositoryRoot,
    env: { ...process.env, NODE_ENV: "development" },
    stdio: "inherit",
  },
);

const waitForChildShutdown = () => {};
process.on("SIGINT", waitForChildShutdown);
process.on("SIGTERM", waitForChildShutdown);

child.once("error", (error) => {
  process.off("SIGINT", waitForChildShutdown);
  process.off("SIGTERM", waitForChildShutdown);
  throw error;
});

child.once("exit", (code, signal) => {
  process.off("SIGINT", waitForChildShutdown);
  process.off("SIGTERM", waitForChildShutdown);
  if (code !== null && code !== 0) {
    process.exitCode = code;
  } else if (signal && signal !== "SIGINT" && signal !== "SIGTERM") {
    process.exitCode = 1;
  }
});
