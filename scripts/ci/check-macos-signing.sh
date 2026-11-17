#!/bin/bash
# Diagnose only public certificate lookup and disposable signatures; never notarize.
set +x
set -euo pipefail
umask 077

fail() { echo "Solar signing check: $*" >&2; exit 1; }
[[ $(uname -s) == Darwin && $(uname -m) == arm64 ]] || fail 'Use the native macos-arm64 runner.'
echo 'Native signing environment:'
/usr/bin/sw_vers
echo "Effective user ID: $(id -u)"
for variable in APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY; do
  [[ -n ${!variable:-} ]] || fail "Missing signing secret: $variable"
done
[[ $APPLE_SIGNING_IDENTITY == 'Developer ID Application: '* ]] || fail 'Use a Developer ID Application identity.'

work=$(mktemp -d "$HOME/Library/Keychains/solar-signing-check.XXXXXX")
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
    bounded /usr/bin/security list-keychains -s "${original_default[@]}" >/dev/null 2>&1 || { echo 'Could not restore effective keychain list.' >&2; status=1; }
    bounded /usr/bin/security list-keychains -d user -s "${original_user[@]}" >/dev/null 2>&1 || { echo 'Could not restore user keychain list.' >&2; status=1; }
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

echo 'launchd manager context:'
bounded /bin/launchctl managername || true
bounded /bin/launchctl manageruid || true
python3 - <<'PY'
import os, subprocess
print('Parent process executable names:')
pid, seen = os.getppid(), set()
for _ in range(10):
    if pid <= 0 or pid in seen:
        break
    seen.add(pid)
    try:
        result = subprocess.run(['/bin/ps', '-p', str(pid), '-o', 'ppid=', '-o', 'comm='], capture_output=True, text=True, timeout=3)
        parent, name = result.stdout.strip().split(None, 1)
        print(name)
        pid = int(parent)
    except (OSError, ValueError, subprocess.TimeoutExpired):
        print('Remaining parent context unavailable.')
        break
PY

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
bounded /usr/bin/security list-keychains -s "$keychain" "${original_default[@]}"
/usr/bin/openssl x509 -in scripts/ci/certificates/DeveloperIDG2CA.pem -outform DER -out "$work/intermediate.cer"
bounded /usr/bin/security add-certificates -k "$keychain" "$work/intermediate.cer"
bounded /usr/bin/security import "$work/certificate.p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign -T /usr/bin/security >/dev/null
bounded /usr/bin/security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain" >/dev/null
bounded /usr/bin/security list-keychains -d user -s "$keychain" "${original_user[@]}"
bounded /usr/bin/security find-certificate -c "$APPLE_SIGNING_IDENTITY" -p "$keychain" > "$work/leaf.pem"
bounded /usr/bin/security find-certificate -a -p "$keychain" > "$work/keychain-certificates.pem"
python3 - "$work/keychain-certificates.pem" "$work" <<'PY'
import pathlib, re, subprocess, sys
blocks = re.findall(r'-----BEGIN CERTIFICATE-----.*?-----END CERTIFICATE-----', pathlib.Path(sys.argv[1]).read_text(), re.S)
print(f'Certificates actually stored in temporary keychain: {len(blocks)}')
for index, block in enumerate(blocks):
    path = pathlib.Path(sys.argv[2]) / f'inventory-{index}.pem'
    path.write_text(block)
    result = subprocess.run(['/usr/bin/openssl', 'x509', '-in', str(path), '-noout', '-subject', '-fingerprint', '-sha256'], capture_output=True, text=True, timeout=10)
    if result.returncode == 0:
        print(result.stdout.strip())
PY
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
  local label=$1 status=0 policy=codeSign local_only=true
  local -a verification_args
  shift
  if [[ ${1:-} == --basic ]]; then policy=basic; shift; fi
  if [[ ${1:-} == --network ]]; then local_only=false; shift; fi
  verification_args=(-p "$policy" -P -v -c "$work/leaf.pem")
  if [[ $local_only == true ]]; then verification_args+=(-L); fi
  bounded /usr/bin/security verify-cert "${verification_args[@]}" "$@" > "$work/$label.chain" 2> "$work/$label.error" || status=$?
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
  if [[ $status != 0 ]]; then sed -n '1,80p' "$work/$label.error"; fi
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
complete_chain_ok=false
if probe explicit-keychain --keychain "$keychain"; then explicit_ok=true; fi
if probe default-search; then default_ok=true; fi

echo 'Retry: temporary keychain added to the effective search list.'
bounded /usr/bin/security list-keychains -s "$keychain" "${original_default[@]}"
show_lists
verify_chain effective-search
if probe effective-search; then repaired_ok=true; fi

echo 'Retry: include the public Apple root certificate without changing trust settings.'
/usr/bin/openssl x509 -in scripts/ci/certificates/AppleRootCA.pem -outform DER -out "$work/apple-root.cer"
bounded /usr/bin/security add-certificates -k "$keychain" "$work/apple-root.cer"
verify_chain complete-chain
if probe complete-chain --keychain "$keychain"; then complete_chain_ok=true; fi

# CMS signing constructs its embedded chain using basic X.509 policy.
verify_chain basic-local-explicit --basic -k "$keychain"
verify_chain basic-local-default --basic
verify_chain basic-network-explicit --basic --network -k "$keychain"
verify_chain basic-network-default --basic --network

# This checks private-key use through Apple's CMS tool without changing the key ACL.
# Signature verification deliberately skips PKI trust, which is checked separately above.
printf 'Solar native signing diagnostic\n' > "$work/cms-input.txt"
cms_sign_status=0
cms_verify_status=1
cms_content_status=1
bounded /usr/bin/security cms -S -N "$APPLE_SIGNING_IDENTITY" -k "$keychain" -H SHA256 -u 6 -i "$work/cms-input.txt" -o "$work/probe.cms" > "$work/cms-sign.log" 2>&1 || cms_sign_status=$?
if [[ $cms_sign_status == 0 ]]; then
  cms_verify_status=0
  bounded /usr/bin/openssl smime -verify -binary -inform DER -in "$work/probe.cms" -noverify -nointern -certfile "$work/leaf.pem" -out "$work/cms-verified.txt" > "$work/cms-verify.log" 2>&1 || cms_verify_status=$?
  if [[ $cms_verify_status == 0 ]] && cmp -s "$work/cms-input.txt" "$work/cms-verified.txt"; then cms_content_status=0; fi
fi
echo "CMS probe: sign exit $cms_sign_status; signature verification exit $cms_verify_status; content comparison exit $cms_content_status"
if [[ $cms_sign_status != 0 ]]; then sed -n '1,12p' "$work/cms-sign.log"; fi
if [[ $cms_sign_status == 0 && $cms_verify_status != 0 ]]; then sed -n '1,12p' "$work/cms-verify.log"; fi

# Compare exact certificate fingerprints, rather than display names that users can change.
# Exported trust settings remain private temporary files; unrelated entries are never printed.
for domain in user admin; do
  trust_status=0
  if [[ $domain == admin ]]; then
    bounded /usr/bin/security trust-settings-export -d "$work/trust-$domain.plist" > "$work/trust-$domain.log" 2>&1 || trust_status=$?
  else
    bounded /usr/bin/security trust-settings-export "$work/trust-$domain.plist" > "$work/trust-$domain.log" 2>&1 || trust_status=$?
  fi
  python3 - "$domain" "$trust_status" "$work/trust-$domain.plist" "$work/leaf.pem" scripts/ci/certificates/DeveloperIDG2CA.pem scripts/ci/certificates/AppleRootCA.pem <<'PY'
import json, pathlib, plistlib, subprocess, sys
domain, status, filename, *certificates = sys.argv[1:]
wanted = {}
for label, certificate in zip(('leaf', 'intermediate', 'root'), certificates):
    result = subprocess.run(['/usr/bin/openssl', 'x509', '-in', certificate, '-noout', '-fingerprint', '-sha1'], capture_output=True, text=True, timeout=10)
    if result.returncode:
        raise SystemExit('Unable to calculate a public certificate fingerprint.')
    fingerprint = result.stdout.strip().split('=')[-1].replace(':', '').upper()
    if len(fingerprint) != 40 or any(c not in '0123456789ABCDEF' for c in fingerprint):
        raise SystemExit('Invalid public certificate fingerprint.')
    wanted[fingerprint] = label
try:
    trust = plistlib.loads(pathlib.Path(filename).read_bytes()).get('trustList', {})
except (OSError, ValueError, plistlib.InvalidFileException):
    print(f'Trust settings ({domain}): no readable export; command exit {status}')
    sys.exit(0)
matches = 0
for key, entry in trust.items():
    fingerprint = key.hex().upper() if isinstance(key, bytes) else str(key).replace(':', '').upper()
    if fingerprint not in wanted:
        continue
    matches += 1
    settings = entry.get('trustSettings', [])
    print(f'Trust settings ({domain}): {wanted[fingerprint]} SHA1={fingerprint}; entries={len(settings)}')
    for setting in settings:
        safe = {}
        for name, value in setting.items():
            if not any(part in name for part in ('Result', 'Policy', 'KeyUsage', 'AllowedError')):
                continue
            if isinstance(value, (bool, int, float)):
                safe[name] = value
            elif isinstance(value, str):
                safe[name] = ''.join(c for c in value[:160] if c.isprintable())
            elif isinstance(value, bytes):
                safe[name] = f'<{len(value)} bytes>'
        print(json.dumps(safe, sort_keys=True))
print(f'Trust settings ({domain}): {matches} exact certificate matches; command exit {status}')
PY
done

echo "Signing results: explicit=$explicit_ok default=$default_ok effective-list-retry=$repaired_ok complete-chain=$complete_chain_ok"
[[ $explicit_ok == true || $default_ok == true || $repaired_ok == true || $complete_chain_ok == true ]] || fail 'All disposable signing probes failed.'
