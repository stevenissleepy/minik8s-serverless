#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/../../../.."

controller_image_ref="${CONTROLLER_IMAGE_REF:-stevenissleepy/serverless-controller:latest}"
activator_image_ref="${ACTIVATOR_IMAGE_REF:-stevenissleepy/serverless-activator:latest}"

cargo build --release -p serverless-controller -p serverless-activator --bins
docker build -t "${controller_image_ref}" -f- . <<'DOCKERFILE'
FROM ubuntu:24.04
COPY target/release/serverless-controller /usr/local/bin/serverless-controller
DOCKERFILE
docker build -t "${activator_image_ref}" -f- . <<'DOCKERFILE'
FROM ubuntu:24.04
COPY target/release/serverless-activator /usr/local/bin/serverless-activator
DOCKERFILE
docker push "${controller_image_ref}"
docker push "${activator_image_ref}"

echo "built and pushed images ${controller_image_ref} ${activator_image_ref}"
