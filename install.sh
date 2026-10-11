#!/usr/bin/env bash
set -euo pipefail

readonly APP_ID="md-view"
readonly APP_NAME="md-view"
readonly APP_COMMENT="Markdown file viewer"
readonly MIME_TYPES=("text/markdown" "text/x-markdown")

build_release() {
    local repo_dir="${1}"
    ( cd -- "${repo_dir}" && cargo build --release )
}

deploy_binary() {
    local src="${1}"
    local dest="${2}"
    if [[ ! -f "${src}" ]]; then
        printf 'error: built binary not found at %s\n' "${src}" >&2
        return 1
    fi
    install -D -m 0755 -- "${src}" "${dest}"
}

join_by() {
    local sep="${1}"
    shift
    local out=""
    local item
    for item in "$@"; do
        out+="${item}${sep}"
    done
    printf '%s' "${out}"
}

write_desktop_file() {
    local desktop_file="${1}"
    local exec_path="${2}"
    local app_name="${3}"
    local app_comment="${4}"
    local mime_list="${5}"
    mkdir -p -- "${desktop_file%/*}"
    cat > "${desktop_file}" <<EOF
[Desktop Entry]
Type=Application
Version=1.0
Name=${app_name}
Comment=${app_comment}
Exec=${exec_path} %f
Terminal=false
NoDisplay=false
Categories=Utility;
MimeType=${mime_list}
StartupWMClass=${APP_ID}
EOF
}

register_default() {
    local apps_dir="${1}"
    local desktop_name="${2}"
    shift 2
    local mime_types=("$@")
    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database -- "${apps_dir}" || true
    fi
    if ! command -v xdg-mime >/dev/null 2>&1; then
        printf 'error: xdg-mime not found; install xdg-utils to set the default handler\n' >&2
        return 1
    fi
    local mime
    for mime in "${mime_types[@]}"; do
        xdg-mime default "${desktop_name}" "${mime}"
    done
}

warn_if_not_on_path() {
    local bin_dir="${1}"
    case ":${PATH}:" in
        *":${bin_dir}:"*) ;;
        *) printf 'note: %s is not on your PATH; add it to run %s from a terminal\n' \
            "${bin_dir}" "${APP_ID}" >&2 ;;
    esac
}

main() {
    local script_dir
    script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

    local repo_dir="${script_dir}"
    local bin_dir="${BIN_DIR:-${HOME}/.local/bin}"
    local data_home="${XDG_DATA_HOME:-${HOME}/.local/share}"
    local apps_dir="${APPS_DIR:-${data_home}/applications}"

    local built_binary="${repo_dir}/target/release/${APP_ID}"
    local installed_binary="${bin_dir}/${APP_ID}"
    local desktop_name="${APP_ID}.desktop"
    local desktop_file="${apps_dir}/${desktop_name}"

    if ! command -v cargo >/dev/null 2>&1; then
        printf 'error: cargo not found; install the Rust toolchain first\n' >&2
        return 1
    fi

    local mime_list
    mime_list="$(join_by ';' "${MIME_TYPES[@]}")"

    build_release "${repo_dir}"
    deploy_binary "${built_binary}" "${installed_binary}"
    write_desktop_file "${desktop_file}" "${installed_binary}" "${APP_NAME}" "${APP_COMMENT}" "${mime_list}"
    register_default "${apps_dir}" "${desktop_name}" "${MIME_TYPES[@]}"

    printf 'Installed binary: %s\n' "${installed_binary}"
    printf 'Desktop entry:    %s\n' "${desktop_file}"
    printf 'Default handler:  %s for %s\n' "${desktop_name}" "${mime_list%;}"
    warn_if_not_on_path "${bin_dir}"
}

main "$@"
