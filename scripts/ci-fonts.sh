#!/usr/bin/env bash
# Pin generic Latin faces in the test runner. The application's own font
# discovery still sees every installed family; only the CI defaults are fixed.
set -euo pipefail
config="${RUNNER_TEMP:?}/bubbleTranslate-fontconfig.conf"
cat > "$config" <<'XML'
<?xml version="1.0"?>
<!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
<fontconfig>
  <include>/etc/fonts/fonts.conf</include>
  <alias><family>sans-serif</family><prefer><family>Liberation Sans</family></prefer></alias>
  <alias><family>serif</family><prefer><family>Liberation Serif</family></prefer></alias>
  <alias><family>monospace</family><prefer><family>Liberation Mono</family></prefer></alias>
</fontconfig>
XML
printf 'FONTCONFIG_FILE=%s\n' "$config" >> "${GITHUB_ENV:?}"
for family in sans-serif serif monospace; do
    FONTCONFIG_FILE="$config" fc-match "$family"
done
