#!/bin/bash
# Diagnose only public certificate lookup and disposable signatures; never notarize.
set +x
set -euo pipefail
umask 077

fail() { echo "Solar signing check: $*" >&2; exit 1; }
[[ $(uname -s) == Darwin && $(uname -m) == arm64 ]] || fail 'Use the native macos-arm64 runner.'
for variable in APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY; do
  [[ -n ${!variable:-} ]] || fail "Missing signing secret: $variable"
done
[[ $APPLE_SIGNING_IDENTITY == 'Developer ID Application: '* ]] || fail 'Use a Developer ID Application identity.'

work=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/solar-signing-check.XXXXXX")
keychain="$work/signing.keychain-db"
keychain_password=$(/usr/bin/openssl rand -hex 32)
original_default=()
original_user=()
search_lists_touched=false

# A stuck native command must not leave a CI job waiting for a GUI prompt.
cat > "$work/bounded.py" <<'PY'
import os, signal, subprocess, sys
process = subprocess.Popen(sys.argv[1:], start_new_session=True)
try:
    result = process.wait(timeout=40)
except subprocess.TimeoutExpired:
    os.killpg(process.pid, signal.SIGKILL)
    process.wait()
    print('Native signing check timed out.', file=sys.stderr)
    result = 124
sys.exit(result if result >= 0 else 1)
PY
bounded() { python3 "$work/bounded.py" "$@"; }

cleanup() {
  local status=$?
  trap - EXIT
  if [[ $search_lists_touched == true ]]; then
    bounded /usr/bin/security list-keychains -d user -s "${original_user[@]}" >/dev/null 2>&1 || { echo 'Could not restore user keychain list.' >&2; status=1; }
    bounded /usr/bin/security list-keychains -s "${original_default[@]}" >/dev/null 2>&1 || { echo 'Could not restore effective keychain list.' >&2; status=1; }
  fi
  if [[ -e $keychain ]]; then
    bounded /usr/bin/security delete-keychain "$keychain" >/dev/null 2>&1 || { echo 'Could not remove diagnostic keychain.' >&2; status=1; }
  fi
  rm -rf -- "$work"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

bounded /usr/bin/security list-keychains > "$work/original-default.txt"
bounded /usr/bin/security list-keychains -d user > "$work/original-user.txt"
python3 - "$work" <<'PY'
import base64, os, pathlib, shlex, sys
root = pathlib.Path(sys.argv[1])
for label in ('default', 'user'):
    paths = shlex.split((root / f'original-{label}.txt').read_text())
    if not paths:
        raise SystemExit('Refusing to replace an empty original keychain list.')
    (root / f'original-{label}.paths').write_bytes(b''.join(os.fsencode(p) + b'\0' for p in paths))
try:
    certificate = base64.b64decode(''.join(os.environ['APPLE_CERTIFICATE'].split()), validate=True)
except Exception:
    raise SystemExit('APPLE_CERTIFICATE must contain base64 file data.') from None
if not certificate:
    raise SystemExit('APPLE_CERTIFICATE decoded to an empty file.')
(root / 'certificate.p12').write_bytes(certificate)
PY
while IFS= read -r -d '' entry; do original_default+=("$entry"); done < "$work/original-default.paths"
while IFS= read -r -d '' entry; do original_user+=("$entry"); done < "$work/original-user.paths"

search_lists_touched=true
bounded /usr/bin/security create-keychain -p "$keychain_password" "$keychain" >/dev/null
bounded /usr/bin/security set-keychain-settings -lut 600 "$keychain"
bounded /usr/bin/security unlock-keychain -p "$keychain_password" "$keychain"
/usr/bin/openssl x509 -in scripts/ci/certificates/DeveloperIDG2CA.pem -outform DER -out "$work/intermediate.cer"
bounded /usr/bin/security add-certificates -k "$keychain" "$work/intermediate.cer"
bounded /usr/bin/security import "$work/certificate.p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign >/dev/null
bounded /usr/bin/security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain" >/dev/null
bounded /usr/bin/security list-keychains -d user -s "$keychain" "${original_user[@]}"
bounded /usr/bin/security find-certificate -c "$APPLE_SIGNING_IDENTITY" -p "$keychain" > "$work/leaf.pem"
identity_sha1=$(/usr/bin/openssl x509 -in "$work/leaf.pem" -noout -fingerprint -sha1 | sed 's/.*=//' | tr -d ':')
[[ $identity_sha1 =~ ^[A-Fa-f0-9]{40}$ ]] || fail 'Could not select the exact signing certificate.'
rm -f "$work/certificate.p12"
unset APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD

show_lists() {
  echo 'Effective keychain search list:'
  bounded /usr/bin/security list-keychains
  echo 'User preference keychain search list:'
  bounded /usr/bin/security list-keychains -d user
}

verify_chain() {
  local label=$1 status=0
  shift
  bounded /usr/bin/security verify-cert -p codeSign -L -P -c "$work/leaf.pem" "$@" > "$work/$label.chain" 2> "$work/$label.error" || status=$?
  echo "Certificate verification ($label): exit $status"
  python3 - "$work/$label.chain" "$work" <<'PY'
import pathlib, re, subprocess, sys
blocks = re.findall(r'-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----', pathlib.Path(sys.argv[1]).read_text(), re.S)
print(f'Public certificates in constructed chain: {len(blocks)}')
for index, block in enumerate(blocks):
    path = pathlib.Path(sys.argv[2]) / f'chain-{index}.pem'
    path.write_text(block)
    result = subprocess.run(['/usr/bin/openssl', 'x509', '-in', str(path), '-noout', '-subject', '-issuer'], capture_output=True, text=True, timeout=10)
    if result.returncode == 0:
        print(result.stdout.strip())
PY
  if [[ $status != 0 ]]; then sed -n '1,12p' "$work/$label.error"; fi
}

probe() {
  local label=$1 status=0
  shift
  /bin/cp /usr/bin/true "$work/$label"
  bounded /usr/bin/codesign --force --timestamp=none --options runtime --sign "$identity_sha1" "$@" "$work/$label" > "$work/$label.log" 2>&1 || status=$?
  if [[ $status == 0 ]]; then
    bounded /usr/bin/codesign --verify --strict "$work/$label" >> "$work/$label.log" 2>&1 || status=$?
  fi
  echo "Disposable signing probe ($label): exit $status"
  sed -n '1,12p' "$work/$label.log"
  return "$status"
}

echo 'Baseline: temporary keychain added to user preferences.'
show_lists
verify_chain explicit-keychain -k "$keychain"
verify_chain default-search
explicit_ok=false
default_ok=false
repaired_ok=false
if probe explicit-keychain --keychain "$keychain"; then explicit_ok=true; fi
if probe default-search; then default_ok=true; fi

echo 'Retry: temporary keychain added to the effective search list.'
bounded /usr/bin/security list-keychains -s "$keychain" "${original_default[@]}"
show_lists
verify_chain effective-search
if probe effective-search; then repaired_ok=true; fi

# Trust dumps contain public certificate metadata; exclude unrelated certificates.
for domain in user admin; do
  trust_status=0
  if [[ $domain == admin ]]; then
    bounded /usr/bin/security dump-trust-settings -d > "$work/trust-$domain.txt" 2>&1 || trust_status=$?
  else
    bounded /usr/bin/security dump-trust-settings > "$work/trust-$domain.txt" 2>&1 || trust_status=$?
  fi
  python3 - "$domain" "$trust_status" "$work/trust-$domain.txt" <<'PY'
import pathlib, re, sys
domain, status, filename = sys.argv[1:]
text = pathlib.Path(filename).read_text()
blocks = re.split(r'(?m)(?=^\s*Cert \d+:)', text)
relevant = [block for block in blocks if re.match(r'\s*Cert \d+:.*(?:Developer ID|Apple Root)', block)]
print(f'Trust overrides ({domain}): {len(relevant)} relevant certificate sections; command exit {status}')
for block in relevant:
    print('\n'.join(block.splitlines()[:30]))
if not relevant and 'No Trust Settings were found' in text:
    print('No trust settings were found in this domain.')
PY
done

echo "Signing results: explicit=$explicit_ok default=$default_ok effective-list-retry=$repaired_ok"
[[ $explicit_ok == true || $default_ok == true || $repaired_ok == true ]] || fail 'All disposable signing probes failed.'
