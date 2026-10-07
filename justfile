# ------------------------------------------
# SPDX-License-Identifier: MIT OR Apache-2.0
# -------------------------------- 𝒒𝒑𝒓𝒐𝒋 --

QPROJ_REF := env("QPROJ_SCRIPTS_REF", "main")
QPROJ_GIT_URL := "git+https://github.com/cubething-qproj/infra.git@" + QPROJ_REF + "#subdirectory=scripts"
SCRIPTS_SRC := env("QPROJ_SCRIPTS_SRC", QPROJ_GIT_URL)
qproj := "qproj-scripts"

_default:
    just --list

# Install the shared CLI.
sync-scripts:
    uv tool install --force --python /usr/bin/python3 qproj-scripts --from {{ SCRIPTS_SRC }}


# Build the workspace.
[working-directory: '.']
build *args:
    {{ qproj }} build {{ args }}

# Run the application.
[working-directory: '.']
play *args:
    {{ qproj }} play {{ args }}

# Lint with Clippy.
[working-directory: '.']
check *args:
    cargo clippy {{ args }}

# Run clippy.
[working-directory: '.']
clippy *args:
    cargo clippy {{ args }}

# Check dependencies with cargo-deny.
[working-directory: '.']
deny:
    {{ qproj }} deny

# Run tests via cargo-nextest.
[working-directory: '.']
test *args:
    {{ qproj }} test {{ args }}

# Generate test coverage report.
[working-directory: '.']
coverage *args:
    {{ qproj }} coverage {{ args }}

# Fix all fixable issues.
[working-directory: '.']
fix *args:
    cargo clippy --fix {{ args }}

# Test CI locally with act.
[working-directory: '.']
ci *args:
    {{ qproj }} ci {{ args }}
