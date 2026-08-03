#!/usr/bin/env bash
# Self-signed TLS material for development, in ./certs (gitignored).
# Requires openssl. For production use real certificates.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p certs

HOST="${1:-localhost}"
openssl req -x509 -newkey rsa:2048 -sha256 -days 365 -nodes \
    -keyout certs/key.pem -out certs/cert.pem \
    -subj "/CN=$HOST" \
    -addext "subjectAltName=DNS:$HOST,IP:127.0.0.1"

echo "wrote certs/cert.pem + certs/key.pem for $HOST"
echo "server.toml:  [tls]  cert = \"certs/cert.pem\"  key = \"certs/key.pem\""
echo "clients must trust certs/cert.pem in their OS trust store"
