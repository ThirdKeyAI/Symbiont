# ── Stage 1: Build ────────────────────────────────────────────────────
FROM node:22-slim AS build

WORKDIR /app

COPY package.json ./
RUN npm install

COPY . .
RUN npm run build

# ── Stage 2: Serve ────────────────────────────────────────────────────
FROM caddy:2-alpine

COPY --from=build /app/dist /srv/www
COPY Caddyfile /etc/caddy/Caddyfile

EXPOSE 3001
