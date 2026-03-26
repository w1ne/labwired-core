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
RUN npm run build

FROM nginx:1.27-alpine
COPY --from=build /repo/packages/playground/dist /usr/share/nginx/html/playground

EXPOSE 80
CMD ["nginx", "-g", "daemon off;"]
