#!/usr/bin/env sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$repo_root/Cargo.toml" | head -n 1)
target=${1:-}

case "$(uname -s)" in
  Darwin) platform=macos ;;
  Linux) platform=linux ;;
  *) echo "unsupported host" >&2; exit 1 ;;
esac

if [ -n "$target" ]; then
  cargo build --release --locked --target "$target" --manifest-path "$repo_root/Cargo.toml"
  binary="$repo_root/target/$target/release/contextpack"
else
  cargo build --release --locked --manifest-path "$repo_root/Cargo.toml"
  binary="$repo_root/target/release/contextpack"
fi

stage="$repo_root/target/package/contextpack"
archive="$repo_root/dist/contextpack-v$version-$platform-x86_64.tar.gz"
rm -rf "$stage"
mkdir -p "$stage" "$repo_root/dist"
cp "$binary" "$stage/"
cp "$repo_root/README.md" "$repo_root/COMMANDS.md" "$repo_root/examples/context-plan.yaml" "$stage/"
tar -C "$(dirname "$stage")" -czf "$archive" "$(basename "$stage")"
printf '%s\n' "$archive"
