set positional-arguments := true

_list:
    @just --list

# Check project formatting.
check:
    just --unstable --fmt --check
    nixpkgs-fmt .
    fd --hidden --extension=md --extension=yml --exec-batch prettier --check
    fd --hidden --extension=toml --exec-batch taplo format --check
    fd --hidden --extension=toml --exec-batch taplo lint
    fd --hidden --extension=proto --exec-batch eclint
    cargo +nightly fmt -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo machete --with-metadata

# Format project.
fmt:
    just --unstable --fmt
    nixpkgs-fmt .
    fd --hidden --extension=md --extension=yml --exec-batch prettier --write
    fd --hidden --extension=toml --exec-batch taplo format
    fd --hidden --extension=proto --exec-batch eclint -fix
    cargo +nightly fmt

# Run protobug.
gen *args:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo run -p=protogen -- "$@"

# Run protobug.
run *args:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo run -p=protobug -- "$@"

# Print a protobuf payload as canonical JSON.
proto-json schema file message='' input_format='auto':
    #!/usr/bin/env bash
    set -euo pipefail
    schema={{ quote(schema) }}
    file={{ quote(file) }}
    message={{ quote(message) }}
    input_format={{ quote(input_format) }}
    args=(cargo run -p=protobug -- inspect --schema "$schema" --file "$file" --input-format "$input_format" --print-json)
    if [[ -n "$message" ]]; then
        args+=(--message "$message")
    fi
    "${args[@]}"

# Apply a jq filter to a protobuf payload and print the transformed JSON.
proto-jq schema file filter message='' input_format='auto':
    #!/usr/bin/env bash
    set -euo pipefail
    schema={{ quote(schema) }}
    file={{ quote(file) }}
    filter={{ quote(filter) }}
    message={{ quote(message) }}
    input_format={{ quote(input_format) }}
    args=(cargo run -p=protobug -- inspect --schema "$schema" --file "$file" --input-format "$input_format" --print-json)
    if [[ -n "$message" ]]; then
        args+=(--message "$message")
    fi
    "${args[@]}" | jq "$filter"

# Apply a jq filter to a protobuf payload and emit the encoded result.
proto-jq-encode schema file filter message='' input_format='auto' output_format='binary':
    #!/usr/bin/env bash
    set -euo pipefail
    schema={{ quote(schema) }}
    file={{ quote(file) }}
    filter={{ quote(filter) }}
    message={{ quote(message) }}
    input_format={{ quote(input_format) }}
    output_format={{ quote(output_format) }}
    inspect_args=(cargo run -p=protobug -- inspect --schema "$schema" --file "$file" --input-format "$input_format" --print-json)
    encode_args=(cargo run -p=protobug -- encode --schema "$schema" --file - --output-format "$output_format")
    if [[ -n "$message" ]]; then
        inspect_args+=(--message "$message")
        encode_args+=(--message "$message")
    fi
    "${inspect_args[@]}" | jq "$filter" | "${encode_args[@]}"

# Apply a jq filter and replace a protobuf payload in place.
proto-jq-rewrite schema file filter message='' input_format='auto' output_format='binary':
    #!/usr/bin/env bash
    set -euo pipefail
    schema={{ quote(schema) }}
    file={{ quote(file) }}
    filter={{ quote(filter) }}
    message={{ quote(message) }}
    input_format={{ quote(input_format) }}
    output_format={{ quote(output_format) }}
    dir="$(dirname "$file")"
    base="$(basename "$file")"
    tmp="$(mktemp "$dir/.${base}.XXXXXX")"
    trap 'rm -f "$tmp"' EXIT
    inspect_args=(cargo run -p=protobug -- inspect --schema "$schema" --file "$file" --input-format "$input_format" --print-json)
    encode_args=(cargo run -p=protobug -- encode --schema "$schema" --file - --output-format "$output_format")
    if [[ -n "$message" ]]; then
        inspect_args+=(--message "$message")
        encode_args+=(--message "$message")
    fi
    "${inspect_args[@]}" | jq "$filter" | "${encode_args[@]}" >"$tmp"
    mv "$tmp" "$file"
    trap - EXIT

# Lint workspace.
clippy:
    cargo clippy --workspace --no-default-features
    cargo clippy --workspace --all-features
    cargo hack --feature-powerset --depth=3 clippy --workspace

# Lint workspace and watch for changes.
clippy-watch:
    cargo watch -- cargo clippy --workspace --all-features

# Apply possible linting fixes in the workspace.
clippy-fix *args:
    cargo clippy --workspace --all-features --fix {{ args }}
    cargo +nightly fmt

# Test workspace.
test:
    cargo nextest run --workspace --all-features

# Document workspace.
doc *args:
    RUSTDOCFLAGS="--cfg=docsrs" cargo +nightly doc --no-deps --workspace --all-features {{ args }}

# Document workspace and watch for changes.
doc-watch: (doc "--open")
    cargo watch -- RUSTDOCFLAGS="--cfg=docsrs" cargo +nightly doc --no-deps --workspace --all-features
