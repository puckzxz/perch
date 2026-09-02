#!/usr/bin/env bash
#
# Prove that vendor/gpui is upstream gpui from crates.io plus exactly the patch
# we meant to carry, and nothing else.
#
# A vendored dependency rots in two ways, and this guards both. Someone edits
# the tree and it quietly stops being reviewable as "upstream plus a diff"; or
# someone bumps the version without re-applying the patch, and a 43.7 GB memory
# leak comes back with no compile error to announce it. Neither shows up in a
# review of the change that caused it, which is why this is a script and not a
# paragraph in a README.
#
#   scripts/verify-vendor.sh            check; exit non-zero on any drift
#   scripts/verify-vendor.sh --update   rewrite vendor/gpui.patch from the tree
#
# `--update` is for when you deliberately change the patch. Run it, read the
# diff it writes, and commit that alongside the source change.

set -euo pipefail

cd "$(dirname "$0")/.."

CRATE=gpui
VENDOR=vendor/gpui
PATCH=vendor/gpui.patch

# The crates.io sha256 of the .crate tarball, pinned to the version below. Both
# are facts about one immutable published artifact, so they live together: bump
# the version in $VENDOR/Cargo.toml without bumping this and the download check
# fails loudly, which is the entire point.
EXPECT_VERSION=0.2.2
EXPECT_SHA256=979b45cfa6ec723b6f42330915a1b3769b930d02b2d505f9697f8ca602bee707

# Large example assets deleted from the vendored copy. They are runtime inputs
# for gpui's own examples, which we never build; every .rs example target is
# still present, so the manifest needs no edit. Dropping them takes the tree
# from 8.1 MB to 3.6 MB.
REMOVED=(
  examples/image/black-cat-typing.gif
  examples/image/app-icon.png
)

# Files we expect to differ from upstream. Anything else differing is drift.
PATCHED=(
  src/platform.rs
  src/platform/mac/metal_atlas.rs
  src/platform/windows/directx_atlas.rs
  src/platform/windows/directx_renderer.rs
  src/platform/windows/events.rs
  src/platform/windows/window.rs
  src/window.rs
)

fail() { echo "verify-vendor: $*" >&2; exit 1; }

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# Every regular file under $1, as repo-relative paths, sorted.
list_files() { ( cd "$1" && find . -type f | sed 's#^\./##' | LC_ALL=C sort ); }

[ -d "$VENDOR" ] || fail "$VENDOR does not exist"

# Derive the vendored version from the tree rather than trusting the constant,
# so a bump is caught here rather than as a confusing checksum failure.
version=$(grep -m1 '^version = ' "$VENDOR/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')
[ "$version" = "$EXPECT_VERSION" ] || fail \
  "$VENDOR is version $version but this script pins $EXPECT_VERSION.
   If the bump is deliberate: re-apply the patch to the new source, update
   EXPECT_VERSION and EXPECT_SHA256 here, then run with --update."

# Absolute, because the apply below runs with the extracted upstream as cwd.
patch_abs="$PWD/$PATCH"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

url="https://static.crates.io/crates/$CRATE/$CRATE-$version.crate"
echo "verify-vendor: fetching $url"
curl -fsSL "$url" -o "$work/crate.tar.gz" || fail "download failed"

got=$(sha256 "$work/crate.tar.gz")
[ "$got" = "$EXPECT_SHA256" ] || fail \
  "checksum mismatch for $CRATE-$version.crate
   expected $EXPECT_SHA256
   got      $got"

tar -xzf "$work/crate.tar.gz" -C "$work"
upstream="$work/$CRATE-$version"
[ -d "$upstream" ] || fail "unexpected archive layout"

# .cargo-ok is a registry extraction marker, not part of the published crate.
rm -f "$upstream/.cargo-ok"
for f in "${REMOVED[@]}"; do
  [ -e "$upstream/$f" ] || fail "REMOVED lists $f but upstream does not have it"
  rm -f "$upstream/$f"
done

# Compare the two trees by hand rather than parsing `diff -rq` output: its
# wording is not something to depend on, and `diff -ruN` would bake mtimes into
# the patch - one side comes from the tarball, the other from the checkout, so
# the patch would differ on every machine.
list_files "$upstream" > "$work/up.files"
list_files "$VENDOR"   > "$work/ven.files"
if ! diff -u "$work/up.files" "$work/ven.files" > "$work/setdrift" 2>&1; then
  echo "verify-vendor: $VENDOR does not hold the same set of files as upstream." >&2
  echo "               (-) only upstream has it, (+) only the vendored tree has it:" >&2
  sed 's/^/    /' "$work/setdrift" >&2
  fail "file set differs from upstream (add to REMOVED, or delete the stray file)"
fi

: > "$work/differing"
while IFS= read -r f; do
  cmp -s "$upstream/$f" "$VENDOR/$f" || printf '%s\n' "$f" >> "$work/differing"
done < "$work/up.files"

printf '%s\n' "${PATCHED[@]}" | LC_ALL=C sort > "$work/expected"
if ! diff -u "$work/expected" "$work/differing" > "$work/contentdrift" 2>&1; then
  echo "verify-vendor: the set of files differing from upstream is not the expected set." >&2
  echo "               (-) expected to differ, (+) actually differs:" >&2
  sed 's/^/    /' "$work/contentdrift" >&2
  fail "unexpected edits to the vendored tree"
fi

if [ "${1:-}" = "--update" ]; then
  : > "$PATCH"
  for f in "${PATCHED[@]}"; do
    diff -u --label "a/$CRATE/$f" --label "b/$CRATE/$f" \
      "$upstream/$f" "$VENDOR/$f" >> "$PATCH" || true
  done
  echo "verify-vendor: wrote $PATCH ($(wc -l < "$PATCH" | tr -d ' ') lines)"
  exit 0
fi

[ -f "$PATCH" ] || fail "$PATCH is missing; run: scripts/verify-vendor.sh --update"

# Apply the committed patch to the pristine upstream and require the result to
# be the vendored tree, byte for byte.
#
# Deliberately not a text comparison of a freshly generated diff against the
# committed one, which is what this did first and what CI killed: BSD diff on
# macOS numbers its hunks differently from GNU diff, so a patch generated on one
# could never match one regenerated on the other, and the check was structurally
# incapable of passing on both legs. Applying asks the better question anyway -
# whether the patch really does reconstruct the tree, rather than whether some
# rendering of a diff of it looks familiar.
# `-c core.autocrlf=false -c core.eol=lf`: `git apply` honours those settings
# when it writes, and the Windows runner sets autocrlf=true globally. Without
# them it applies the patch perfectly and then writes CRLF into files the
# vendored tree stores as LF, so every patched file "differs" for a reason that
# has nothing to do with the patch.
if ! (cd "$upstream" && git -c core.autocrlf=false -c core.eol=lf apply -p2 "$patch_abs")   2> "$work/applyerr"; then
  echo "verify-vendor: $PATCH does not apply to upstream $CRATE $version." >&2
  sed 's/^/    /' "$work/applyerr" >&2
  fail "the committed patch and the vendored tree have diverged"
fi

: > "$work/mismatch"
while IFS= read -r f; do
  cmp -s "$upstream/$f" "$VENDOR/$f" || printf '%s\n' "$f" >> "$work/mismatch"
done < "$work/up.files"
if [ -s "$work/mismatch" ]; then
  echo "verify-vendor: applying $PATCH to upstream does not reproduce $VENDOR." >&2
  echo "               These files differ from what the patch says they are:" >&2
  sed 's/^/    /' "$work/mismatch" >&2
  fail "patch drift"
fi

# The patch is byte-for-byte what it should be; now check it still does its job.
#
# Two assertions per fix wherever the state is set in one place and acted on in
# another, because one is not enough: `filter(|texture| !texture.dedicated)`
# still matches a file whose `let dedicated = ...` has been replaced by
# `let dedicated = false;`, and every texture is shared again with the 43.7 GB
# leak back and the grep still green. The setter and the consumer both have to
# be named. This matters most right after someone runs `--update`, which
# rewrites the patch from whatever the tree is and never reaches this block.
needs() { # file, literal string, what that string makes the file do
  grep -qF "$2" "$VENDOR/$1" || fail "$VENDOR/$1 no longer $3"
}

for f in "${PATCHED[@]}"; do
  grep -q 'PERCH PATCH' "$VENDOR/$f" || fail "$VENDOR/$f has lost its PERCH PATCH marker"
done

for f in src/platform/mac/metal_atlas.rs src/platform/windows/directx_atlas.rs; do
  needs "$f" 'size.width > DEFAULT_ATLAS_SIZE.width || size.height > DEFAULT_ATLAS_SIZE.height' \
    "decides which textures are dedicated to one image"
  needs "$f" 'filter(|texture| !texture.dedicated)' \
    "skips dedicated textures when scanning for room"
  needs "$f" 'allocator.deallocate(tile.tile_id.into())' \
    "returns removed tile space to the allocator"
done

needs src/platform.rs 'fn update(&self, _key: &AtlasKey' \
  "offers atlases an in-place tile update"
needs src/platform/windows/directx_atlas.rs 'fn update(&self, key: &AtlasKey' \
  "implements the in-place tile update"
needs src/platform/windows/directx_atlas.rs 'skipping upload to avoid a driver over-read' \
  "refuses an upload the source slice cannot cover"
needs src/window.rs 'pub fn update_image(' \
  "exposes the in-place image update to callers"

needs src/platform/windows/directx_renderer.rs 'self.skip_draws = true;' \
  "stops drawing when the device is replaced"
needs src/platform/windows/directx_renderer.rs 'if self.skip_draws {' \
  "acts on the post-device-lost draw block"
needs src/platform/windows/directx_renderer.rs 'render_target: Option<ID3D11Texture2D>' \
  "holds the render target in something that releases exactly once"
needs src/platform/windows/directx_renderer.rs 'self.resources.render_target.take()' \
  "releases the render target without leaving a dropped value behind"
needs src/platform/windows/events.rs 'lock.force_render_pending = true;' \
  "asks for a forced render after a device loss"
needs src/platform/windows/window.rs 'pub force_render_pending: bool,' \
  "carries the post-device-lost forced-render flag"

echo "verify-vendor: OK - $CRATE $version + $PATCH, ${#PATCHED[@]} files patched, ${#REMOVED[@]} assets removed"
