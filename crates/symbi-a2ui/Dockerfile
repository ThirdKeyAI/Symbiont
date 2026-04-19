# ── Stage 1: Build ────────────────────────────────────────────────────
FROM node:22-slim AS build

WORKDIR /app

COPY package.json package-lock.json* ./
RUN --mount=type=cache,target=/root/.npm \
    npm install --prefer-offline

COPY . .
RUN npm run build

# ── Stage 2: Serve ────────────────────────────────────────────────────
FROM caddy:2-alpine

# Create non-root user and group for Caddy to drop to.
# 101:101 matches the unprivileged id range used by other Caddy images.
RUN addgroup -S -g 101 caddy \
    && adduser -S -D -H -u 101 -h /var/lib/caddy -s /sbin/nologin -G caddy -g caddy caddy \
    && mkdir -p /var/lib/caddy /srv/www /etc/caddy \
    && chown -R caddy:caddy /var/lib/caddy /srv/www /etc/caddy

COPY --from=build --chown=caddy:caddy /app/dist /srv/www
COPY --chown=caddy:caddy Caddyfile /etc/caddy/Caddyfile

# Drop privileges before running Caddy.
USER caddy:caddy

EXPOSE 3001
