#!/usr/bin/env bash
set -euo pipefail

mode="release"

if [[ "${1:-}" == "--debug" ]]; then
  mode="debug"
  shift
fi

if [[ "$#" -gt 0 ]]; then
  echo "Usage: $0 [--debug]" >&2
  exit 1
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"

cargo_profile_flag="--release"
godot_export_flag="--export-release"
if [[ "$mode" == "debug" ]]; then
  cargo_profile_flag=""
  godot_export_flag="--export-debug"
fi

echo "Building gdext (${mode}) for wasm32-unknown-emscripten"
RUSTFLAGS="${RUSTFLAGS:-} -C panic=abort" cargo build \
  --manifest-path "${repo_root}/rust/Cargo.toml" \
  -p gdext \
  --target wasm32-unknown-emscripten \
  ${cargo_profile_flag}

tmp_export_dir="$(mktemp -d)"
tmp_export_file="${tmp_export_dir}/index.html"

echo "Exporting Godot project (${mode})"
godot4 --headless \
  --path "${repo_root}/godot" \
  ${godot_export_flag} "Web" "${tmp_export_file}"

tmp_index_html="${tmp_export_dir}/index.html"

requires_side_wasm=0
if [[ -f "${tmp_index_html}" ]]; then
  requires_side_wasm="$(python3 - "${tmp_index_html}" <<'PY'
from pathlib import Path
import re
import sys

text = Path(sys.argv[1]).read_text(encoding="utf-8")
match = re.search(r'"gdextensionLibs"\s*:\s*\[(.*?)\]', text, flags=re.S)
if not match:
    print("0")
    raise SystemExit(0)

items = [part.strip() for part in match.group(1).split(",") if part.strip()]
print("1" if len(items) > 0 else "0")
PY
)"
fi

if [[ "${requires_side_wasm}" == "1" ]] && [[ ! -f "${tmp_export_dir}/index.side.wasm" ]]; then
  echo "ERROR: Web export requires dynamic-link side module, but index.side.wasm is missing." >&2
  echo "Your template likely lacks dlink support or Godot did not use the custom dlink-enabled template." >&2
  echo "Exported files:" >&2
  ls -1 "${tmp_export_dir}" >&2
  exit 1
fi

server_public_dir="${repo_root}/rust/crates/server/public"
tmp_main_js="${tmp_export_dir}/_main.js"

if [[ -f "${server_public_dir}/main.js" ]]; then
  cp "${server_public_dir}/main.js" "${tmp_main_js}"
fi

mkdir -p "${server_public_dir}"
rm -rf "${server_public_dir}"/*
cp -R "${tmp_export_dir}/"* "${server_public_dir}/"

if [[ -f "${tmp_main_js}" ]]; then
  cp "${tmp_main_js}" "${server_public_dir}/main.js"
fi

if [[ "${requires_side_wasm}" == "1" ]] && [[ ! -f "${server_public_dir}/index.side.wasm" ]]; then
  echo "ERROR: index.side.wasm missing after copy to server public directory." >&2
  exit 1
fi

index_html="${server_public_dir}/index.html"
if [[ -f "${index_html}" ]] && ! grep -q 'src="main.js"' "${index_html}"; then
  python3 - "${index_html}" <<'PY'
from pathlib import Path
import sys

index_path = Path(sys.argv[1])
content = index_path.read_text(encoding="utf-8")
inject = '    <script src="main.js"></script>\n'

if "</head>" in content:
    content = content.replace("</head>", inject + "</head>", 1)
elif "</body>" in content:
    content = content.replace("</body>", inject + "</body>", 1)
else:
    content += "\n" + inject

index_path.write_text(content, encoding="utf-8")
PY
fi

echo "Web assets exported to ${server_public_dir}"
