const { existsSync, readFileSync, writeFileSync } = require("node:fs");
const { resolve } = require("node:path");
const { parseEnv } = require("node:util");

const repositoryRoot = resolve(__dirname, "..");
const sourceEnvFile = resolve(
  repositoryRoot,
  process.env.KBC_LOCAL_SOURCE_ENV_FILE?.trim() || "../KBC-rakv0-discord-bot/.env",
);
const targetEnvFile = resolve(repositoryRoot, ".env");

if (!existsSync(sourceEnvFile)) {
  throw new Error(`Source environment file was not found: ${sourceEnvFile}`);
}

const source = parseEnv(readFileSync(sourceEnvFile, "utf8"));
const current = existsSync(targetEnvFile)
  ? parseEnv(readFileSync(targetEnvFile, "utf8"))
  : {};
const discordToken = current.DISCORD_TOKEN?.trim() || source.DISCORD_TOKEN?.trim();
if (!discordToken) {
  throw new Error("DISCORD_TOKEN was not found in the source environment file.");
}
const githubToken =
  current.GITHUB_DATA_TOKEN?.trim() ||
  source.GITHUB_DATA_TOKEN?.trim() ||
  source.WORKFLOW_GITHUB_TOKEN?.trim();
const eventUpdateSecret =
  current.EVENT_UPDATE_SECRET?.trim() || source.EVENT_UPDATE_SECRET?.trim();

const values = {
  DISCORD_TOKEN: discordToken,
  ...(githubToken
    ? {
        GITHUB_DATA_OWNER:
          current.GITHUB_DATA_OWNER?.trim() || "sinsuirakv0",
        GITHUB_DATA_REPO:
          current.GITHUB_DATA_REPO?.trim() || "KBC-rakv0-discord-bot-data",
        GITHUB_DATA_BRANCH: current.GITHUB_DATA_BRANCH?.trim() || "main",
        GITHUB_DATA_TOKEN: githubToken,
      }
    : {}),
  ...(eventUpdateSecret
    ? {
        EVENT_UPDATE_SECRET: eventUpdateSecret,
        EVENT_UPDATE_PORT:
          current.EVENT_UPDATE_PORT?.trim() ||
          source.EVENT_UPDATE_PORT?.trim() ||
          source.PORT?.trim() ||
          "3000",
        EVENT_UPDATE_HOST:
          current.EVENT_UPDATE_HOST?.trim() ||
          source.EVENT_UPDATE_HOST?.trim() ||
          "0.0.0.0",
      }
    : {}),
};

writeFileSync(
  targetEnvFile,
  `${Object.entries(values)
    .map(([name, value]) => `${name}=${JSON.stringify(value)}`)
    .join("\n")}\n`,
  { encoding: "utf8", mode: 0o600 },
);
process.stdout.write(`Updated ${targetEnvFile} without displaying secret values.\n`);
