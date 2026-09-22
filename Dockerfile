FROM rust:1.98-bookworm AS rust-builder
WORKDIR /workspace

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked -p kbc-node

FROM node:24-bookworm-slim AS typescript-builder
WORKDIR /workspace

COPY package.json package-lock.json tsconfig.json ./
COPY apps/discord/package.json ./apps/discord/package.json
RUN npm ci

COPY apps/discord ./apps/discord
RUN npm run compile:ts && npm prune --omit=dev

FROM node:24-bookworm-slim
WORKDIR /app
ENV NODE_ENV=production

COPY package.json ./
COPY apps/discord/package.json ./apps/discord/package.json
COPY content ./content
COPY --from=typescript-builder /workspace/node_modules ./node_modules
COPY --from=typescript-builder /workspace/apps/discord/dist ./apps/discord/dist
COPY --from=rust-builder /workspace/target/release/libkbc_node.so ./native/kbc_node.node

EXPOSE 3000
CMD ["node", "apps/discord/dist/index.js"]
