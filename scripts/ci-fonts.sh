#!/usr/bin/env bash
# Pin generic Latin faces in the test runner. The application's own font
# discovery still sees every installed family; only the CI defaults are fixed.
set -euo pipefail
config="${RUNNER_TEMP:?}/bubbleTranslate-fontconfig.conf"
cat > "$config" <<'XML'
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <match target="pattern">
    <test name="family" qual="any"><string>sans-serif</string></test>
    <edit name="family" mode="assign" binding="strong"><string>Liberation Sans</string></edit>
  </match>
  <match target="pattern">
    <test name="family" qual="any"><string>serif</string></test>
    <edit name="family" mode="assign" binding="strong"><string>Liberation Serif</string></edit>
  </match>
  <match target="pattern">
    <test name="family" qual="any"><string>monospace</string></test>
    <edit name="family" mode="assign" binding="strong"><string>Liberation Mono</string></edit>
  </match>
  <include>/etc/fonts/fonts.conf</include>
</fontconfig>
XML
printf 'FONTCONFIG_FILE=%s\n' "$config" >> "${GITHUB_ENV:?}"
for family in sans-serif serif monospace; do
    case "$family" in
        sans-serif) expected="Liberation Sans" ;;
        serif) expected="Liberation Serif" ;;
        monospace) expected="Liberation Mono" ;;
    esac
    found="$(FONTCONFIG_FILE="$config" fc-match -f '%{family[0]}' "$family")"
    if [ "$found" != "$expected" ]; then
        echo "error: $family resolved to $found, expected $expected" >&2
        exit 1
    fi
    echo "$family: $found"
done
