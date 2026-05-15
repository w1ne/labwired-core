FROM node:20-alpine AS build

WORKDIR /repo

# Build @labwired/ui first (playground depends on it via file: reference)
COPY packages/ui/package.json packages/ui/package-lock.json* packages/ui/
WORKDIR /repo/packages/ui
RUN npm ci --ignore-scripts
COPY packages/ui/ .
RUN npm run build

# Build the playground
WORKDIR /repo/packages/playground
COPY packages/playground/package.json packages/playground/package-lock.json* ./
RUN npm ci --ignore-scripts
COPY packages/playground/ .
COPY core/ /repo/core/

# Vite build-time env vars. Provide these via `docker build --build-arg ...`
# or the workflow's docker/build-push-action `build-args:` field.
ARG VITE_CLERK_PUBLISHABLE_KEY
ARG VITE_LABWIRED_API_BASE
ENV VITE_CLERK_PUBLISHABLE_KEY=${VITE_CLERK_PUBLISHABLE_KEY}
ENV VITE_LABWIRED_API_BASE=${VITE_LABWIRED_API_BASE}
RUN npm run build

FROM nginx:1.27-alpine
COPY --from=build /repo/packages/playground/dist /usr/share/nginx/html/playground

EXPOSE 80
CMD ["nginx", "-g", "daemon off;"]
