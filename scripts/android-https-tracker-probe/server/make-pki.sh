#!/bin/bash
# usage: make-pki.sh <out-dir>          create everything (empty CRL)
#        make-pki.sh <out-dir> revoke   reissue the CRL listing the CRL-only leaf
#
# Test-only PKI for the probe: a root to install in the emulator's system store,
# a CRL-only leaf (CRL on a host the app's network config lists, mapped to the
# host machine through the emulator's /etc/hosts), a leaf without revocation
# info, and a leaf from a root the device does not trust.
set -e
out=${1:?usage: make-pki.sh <out-dir> [revoke]}; mkdir -p "$out/cadb"; cd "$out"
crl() { openssl ca -config ca.cnf -gencrl -keyfile ca.key -cert ca.crt -out probe.crl.pem 2>/dev/null
  openssl crl -in probe.crl.pem -outform DER -out probe.crl; }
if [ "$2" = revoke ]; then
  if ! grep -q "^R" cadb/index.txt; then
    openssl ca -config ca.cnf -keyfile ca.key -cert ca.crt -revoke crl.crt
  fi
  crl
  echo "probe.crl now revokes $(openssl x509 -in crl.crt -noout -serial)"; exit 0
fi
touch cadb/index.txt; echo 1000 > cadb/crlnumber
printf '[ca]\ndefault_ca = d\n[d]\ndatabase = cadb/index.txt\ncrlnumber = cadb/crlnumber\ndefault_md = sha256\ndefault_crl_days = 7\n' > ca.cnf
ca() { openssl req -x509 -newkey rsa:2048 -nodes -keyout "$1.key" -out "$1.crt" -days 30 -subj "/CN=$2" \
  -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null; }
leaf() { openssl req -newkey rsa:2048 -nodes -keyout "$1.key" -out "$1.csr" -subj "/CN=10.0.2.2" 2>/dev/null
  printf "subjectAltName=IP:10.0.2.2\nextendedKeyUsage=serverAuth\nbasicConstraints=CA:FALSE\n$3" > "$1.ext"
  openssl x509 -req -in "$1.csr" -CA "$2.crt" -CAkey "$2.key" -CAcreateserial -out "$1.crt" -days 20 -extfile "$1.ext" 2>/dev/null; }
ca ca "Kiwix Probe Test Root"; ca rogue "Kiwix Probe Untrusted Root"
leaf crl ca "crlDistributionPoints=URI:http://r10.c.lencr.org/probe.crl\n"
leaf norev ca ""
leaf rogue_leaf rogue ""
crl
touch serve-crl
hash=$(openssl x509 -in ca.crt -noout -subject_hash_old)
cp ca.crt "$hash.0"; echo "system CA file: $out/$hash.0"
