#!/bin/sh
# Builds the booking API for Lambda: a static arm64 Linux binary, cross-compiled with zig from any
# host, zipped as `bootstrap` with a fixed timestamp, so the same binary always zips to the same
# bytes.
#   sh server/build-lambda.sh <out-dir>
set -eu
out=$1
target=aarch64-unknown-linux-musl
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
# The commit the binary reports at /healthz, which a deploy waits for before it moves the aliases.
BUILD_SHA=${BUILD_SHA:-$(git -C "${root}" rev-parse HEAD)}
export BUILD_SHA
# The paths are remapped, so the binary doesn't depend on where it was built. zig's linker notes
# that it ignores the -O1 rustc passes, which is harmless.
RUSTFLAGS="--remap-path-prefix=${root}=/build --remap-path-prefix=${CARGO_HOME:-${HOME}/.cargo}=/cargo -A linker_messages" \
	cargo zigbuild --release --locked --manifest-path "${root}/Cargo.toml" --target "${target}" --bin bootstrap
work=$(mktemp -d)
trap 'rm -rf "${work}"' EXIT
install -m 0755 "${root}/target/${target}/release/bootstrap" "${work}/bootstrap"
touch -t 198001010000.00 "${work}/bootstrap"
(cd "${work}" && TZ=UTC zip -q -X booking-lambda.zip bootstrap)
mkdir -p "${out}"
mv "${work}/booking-lambda.zip" "${out}/"
