#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/../../../.."

image_ref="${IMAGE_REF:-stevenissleepy/serverless-controller:latest}"

cargo build --release -p serverless-controller
docker build -t "${image_ref}" -f- . <<'DOCKERFILE'
FROM ubuntu:24.04
COPY target/release/serverless-controller /usr/local/bin/serverless-controller
DOCKERFILE
docker push "${image_ref}"

echo "built and pushed image ${image_ref}"
