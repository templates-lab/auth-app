# Frontend image (bead authapp-3cb595): build the SPA with Vite, then serve the
# static output from an unprivileged nginx.
#
# Build context is the repo root (see infra/traefik/docker-compose.yml):
#   docker build -f infra/docker/web.Dockerfile -t auth-app-web .
#
# The runtime is `nginxinc/nginx-unprivileged`: nginx running as a non-root user
# (uid 101), which listens on 8080 rather than 80 — a non-root process cannot
# bind a privileged port. The edge (Traefik) forwards to that port; the SPA
# fallback lives in infra/web/nginx.conf.

FROM node:20-slim AS build
RUN corepack enable
WORKDIR /app
# Copy the whole workspace and install from the frozen lockfile, then build
# every package (the web app and the feature/library packages it depends on).
COPY . .
RUN pnpm install --frozen-lockfile
RUN pnpm run build

FROM nginxinc/nginx-unprivileged:1.27-alpine AS runtime
# Our server block (SPA fallback, asset caching) replaces the image default.
# `--chown=nginx` so the non-root nginx user (uid 101) can read the files it is
# copied — a root-owned copy trips a "Permission denied" at startup.
COPY --chown=nginx:nginx infra/web/nginx.conf /etc/nginx/conf.d/default.conf
COPY --chown=nginx:nginx --from=build /app/apps/web/dist /usr/share/nginx/html
EXPOSE 8080
