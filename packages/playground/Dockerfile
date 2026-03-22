FROM node:20-alpine AS build

WORKDIR /packages

# Build @labwired/ui first (playground depends on it via file: reference)
COPY ui/package.json ui/package-lock.json* ui/
WORKDIR /packages/ui
RUN npm ci --ignore-scripts
COPY ui/ .
RUN npm run build

# Build the playground
WORKDIR /packages/playground
COPY playground/package.json playground/package-lock.json* ./
RUN npm ci --ignore-scripts
COPY playground/ .
RUN npm run build

FROM nginx:1.27-alpine
COPY --from=build /packages/playground/dist /usr/share/nginx/html/playground

EXPOSE 80
CMD ["nginx", "-g", "daemon off;"]
