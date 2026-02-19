#!/usr/bin/env bash
set -euo pipefail

OUT_DIR="certs"
mkdir -p "${OUT_DIR}"

DNS_NAMES="${MIQBOT_TLS_DNS:-localhost}"
IP_NAMES="${MIQBOT_TLS_IPS:-127.0.0.1}"

SAN_EXT="${OUT_DIR}/server_san.ext"
{
  echo "subjectAltName=@alt_names"
  echo "extendedKeyUsage=serverAuth"
  echo "[alt_names]"
  idx=1
  IFS=',' read -ra dns_arr <<< "${DNS_NAMES}"
  for dns in "${dns_arr[@]}"; do
    dns_trimmed="$(echo "$dns" | xargs)"
    if [[ -n "${dns_trimmed}" ]]; then
      echo "DNS.${idx}=${dns_trimmed}"
      idx=$((idx + 1))
    fi
  done
  idx=1
  IFS=',' read -ra ip_arr <<< "${IP_NAMES}"
  for ip in "${ip_arr[@]}"; do
    ip_trimmed="$(echo "$ip" | xargs)"
    if [[ -n "${ip_trimmed}" ]]; then
      echo "IP.${idx}=${ip_trimmed}"
      idx=$((idx + 1))
    fi
  done
} > "${SAN_EXT}"

CLIENT_EXT="${OUT_DIR}/client.ext"
{
  echo "extendedKeyUsage=clientAuth"
} > "${CLIENT_EXT}"

openssl genrsa -out "${OUT_DIR}/ca.key.pem" 4096
openssl req -x509 -new -nodes -key "${OUT_DIR}/ca.key.pem" -sha256 -days 3650 \
  -subj "/CN=MiqBOT-ca" \
  -out "${OUT_DIR}/ca.crt.pem"

openssl genrsa -out "${OUT_DIR}/server.key.pem" 4096
openssl req -new -key "${OUT_DIR}/server.key.pem" -subj "/CN=MiqBOT-bridge" -out "${OUT_DIR}/server.csr.pem"
openssl x509 -req -in "${OUT_DIR}/server.csr.pem" -CA "${OUT_DIR}/ca.crt.pem" -CAkey "${OUT_DIR}/ca.key.pem" -CAcreateserial \
  -out "${OUT_DIR}/server.crt.pem" -days 3650 -sha256 -extfile "${SAN_EXT}"

openssl genrsa -out "${OUT_DIR}/client.key.pem" 4096
openssl req -new -key "${OUT_DIR}/client.key.pem" -subj "/CN=MiqBOT-game-client" -out "${OUT_DIR}/client.csr.pem"
openssl x509 -req -in "${OUT_DIR}/client.csr.pem" -CA "${OUT_DIR}/ca.crt.pem" -CAkey "${OUT_DIR}/ca.key.pem" -CAcreateserial \
  -out "${OUT_DIR}/client.crt.pem" -days 3650 -sha256 -extfile "${CLIENT_EXT}"

echo "OK: generated CA/server/client certs into ${OUT_DIR}/"
echo "DNS SANs: ${DNS_NAMES}"
echo "IP SANs:  ${IP_NAMES}"
