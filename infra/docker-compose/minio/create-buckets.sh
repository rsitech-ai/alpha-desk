#!/bin/sh
set -eu

: "${MINIO_ROOT_USER:?MINIO_ROOT_USER is required}"
: "${MINIO_ROOT_PASSWORD:?MINIO_ROOT_PASSWORD is required}"
: "${MINIO_BUCKET:?MINIO_BUCKET is required}"
: "${MC_CONFIG_DIR:?MC_CONFIG_DIR is required}"

mc alias set local http://minio:9000 \
  "$MINIO_ROOT_USER" "$MINIO_ROOT_PASSWORD" >/dev/null
mc mb --ignore-existing "local/$MINIO_BUCKET" >/dev/null
mc stat "local/$MINIO_BUCKET" >/dev/null

printf 'minio-bucket:ok name=%s\n' "$MINIO_BUCKET"
