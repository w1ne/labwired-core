FROM golang:1.24-alpine AS build
WORKDIR /src

COPY go.mod go.sum ./
RUN go mod download

COPY cmd ./cmd
COPY internal ./internal
COPY static ./static
COPY configs ./configs
COPY openapi.yaml ./openapi.yaml

RUN CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -o /out/foundry-server ./cmd/server

FROM alpine:3.20
RUN adduser -D -u 10001 appuser
WORKDIR /app

COPY --from=build /out/foundry-server /app/foundry-server
COPY --from=build /src/static /app/static
COPY --from=build /src/configs /app/configs

ENV PORT=8080 \
    APP_ENV=production \
    DB_PATH=/data/foundry.db \
    ARTIFACTS_DIR=/data/artifacts \
    HARDWARE_JSON_PATH=/app/configs/renode_hardware.json

RUN mkdir -p /data/artifacts && chown -R appuser:appuser /app /data
USER appuser

EXPOSE 8080
ENTRYPOINT ["/app/foundry-server"]
