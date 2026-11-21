#!/usr/bin/env bash
#
# Prove that each vendored crate is its upstream from crates.io plus exactly
# the patch we meant to carry, and nothing else.
#
# A vendored dependency rots in two ways, and this guards both. Someone edits
# the tree and it quietly stops being reviewable as "upstream plus a diff"; or
# someone bumps the version without re-applying the patch, and a 43.7 GB memory
# leak comes back with no compile error to announce it. Neither shows up in a
# review of the change that caused it, which is why this is a script and not a
# paragraph in a README.
#
#   scripts/verify-vendor.sh            check; exit non-zero on any drift
#   scripts/verify-vendor.sh --update   rewrite the patch of each crate whose
#                                       tree its patch no longer reproduces
#
# `--update` is for when you deliberately change a patch. Run it, read the
# diff it writes, and commit that alongside the source change.

set -euo pipefail

cd "$(dirname "$0")/.."

# --- The table ---------------------------------------------------------------
#
# One entry per vendored crate, and an entry is two functions: `crate_<key>`
# says what the crate is, and `check_<key>` holds the assertions that each of
# its fixes is still there. To vendor another crate, add its key to CRATES and
# write the two functions; nothing below the table changes.
#
# Functions rather than an associative array because the macOS runner's bash
# is 3.2, which has none.
#
# A crate with no fixes is allowed: PATCHED=() and no patch file (or an empty
# one), and a `check_<key>` that does nothing. That is how a crate is first
# brought in, pristine, so that the commit which patches it shows the patch
# and nothing else.

CRATES=(gpui_pre gpui_pre_windows gpui_pre_macos)

# gpui-pre is a snapshot of Zed's gpui, published weekly to crates.io by
# gpui-component's maintainer, with each platform's backend in a crate of its
# own. gpui-component pins one snapshot exactly, so these move together and
# with it.
crate_gpui_pre() {
  CRATE=gpui-pre
  VENDOR=vendor/gpui-pre
  PATCH=vendor/gpui-pre.patch

  # The crates.io sha256 of the .crate tarball, pinned to the version beside
  # it. Both are facts about one immutable published artifact, so they live
  # together: bump the version in $VENDOR/Cargo.toml without bumping these and
  # the check fails loudly, which is the entire point.
  EXPECT_VERSION=0.3.12
  EXPECT_SHA256=29db89613527079737f53752eab27fb65c9a7dd94979256c65a0e590c9fd34d8

  # Large example assets deleted from the vendored copy. They are runtime
  # inputs for gpui's own examples, which we never build; every .rs example
  # target is still present, so the manifest needs no edit. Dropping them takes
  # the tree from 8.6 MB to 4.1 MB.
  REMOVED=(
    examples/image/black-cat-typing.gif
    examples/image/app-icon.png
  )

  # Files we expect to differ from upstream. Anything else differing is drift.
  PATCHED=(
    src/app.rs
    src/platform.rs
    src/window.rs
  )
}

# Two assertions per fix wherever the state is set in one place and acted on in
# another, because one is not enough: `filter(|texture| !texture.dedicated)`
# still matches a file whose `let dedicated = ...` has been replaced by
# `let dedicated = false;`, and every texture is shared again with the 43.7 GB
# leak back and the grep still green. The setter and the consumer both have to
# be named. This matters most right after someone runs `--update`, which
# rewrites the patch from whatever the tree is and never reaches this block.
check_gpui_pre() {
  # The in-place tile update: offered by the atlas trait, carried by the
  # shared atlas state only while the tile is still the image's size, and
  # reached from `Window::update_image`.
  needs src/platform.rs 'fn update(&self, _key: &AtlasKey' \
    "offers atlases an in-place tile update"
  needs src/platform.rs 'fn update(&mut self, _tile: AtlasTile, _bytes: &[u8]) -> bool' \
    "lets an atlas backend overwrite a tile"
  needs src/platform.rs 'if tile.bounds.size != size {' \
    "refuses an in-place update at another size"
  needs src/platform.rs 'self.backend.update(tile, bytes)' \
    "hands an in-place update to the backend"
  needs src/window.rs 'pub fn update_image(' \
    "exposes the in-place image update to callers"
  needs src/window.rs '.update(&params.clone().into(), data.size(frame_index), bytes)' \
    "asks the atlas to overwrite the image's tile"

  needs src/window.rs 'let pointer_left = self.tracks_window_hover() && !self.hovered.get();' \
    "reads the hovered flag to know the pointer has left the window"
  needs src/window.rs 'self.mouse_hit_test = if pointer_left {' \
    "hit-tests nothing under a pointer that has left the window"

  needs src/window.rs 'cfg!(target_os = "macos") && kind == crate::WindowKind::Floating;' \
    "tracks macOS floating windows"
  needs src/window.rs 'if self.tracks_mac_panel_hover {' \
    "updates the floating window hover flag from pointer events"
  needs src/window.rs 'PlatformInput::MouseExited(_) => false,' \
    "clears floating window hover on pointer exit"

  needs src/app.rs 'pub fn hide_cursor_until_mouse_moves(&self)' \
    "lets an app hide the cursor until the mouse moves"
  needs src/app.rs 'self.platform.hide_cursor_until_mouse_moves();' \
    "passes the cursor hide to the platform"
}

crate_gpui_pre_windows() {
  CRATE=gpui-pre-windows
  VENDOR=vendor/gpui-pre-windows
  PATCH=vendor/gpui-pre-windows.patch
  EXPECT_VERSION=0.3.12
  EXPECT_SHA256=7201762420222d5b5ce5bdf0056996199212a3349bfb2457a3a0238c688681f3
  REMOVED=()
  PATCHED=(
    src/directx_atlas.rs
  )
}

check_gpui_pre_windows() {
  needs src/directx_atlas.rs 'size.width > DEFAULT_ATLAS_SIZE.width || size.height > DEFAULT_ATLAS_SIZE.height' \
    "decides which textures are dedicated to one image"
  needs src/directx_atlas.rs 'filter(|texture| !texture.dedicated)' \
    "skips dedicated textures when scanning for room"
  needs src/directx_atlas.rs 'self.0.lock().update(key, size, bytes)' \
    "passes an in-place tile update to the atlas state"
  needs src/directx_atlas.rs 'fn update(&mut self, tile: AtlasTile, bytes: &[u8]) -> bool' \
    "writes over a tile in place"
}

crate_gpui_pre_macos() {
  CRATE=gpui-pre-macos
  VENDOR=vendor/gpui-pre-macos
  PATCH=vendor/gpui-pre-macos.patch
  EXPECT_VERSION=0.3.12
  EXPECT_SHA256=2de7d3f4ad51ac387e822495db363fcc1188d349793365731d054444c4cb06ee
  REMOVED=()
  PATCHED=(Cargo.toml src/platform.rs src/window.rs)
}

check_gpui_pre_macos() {
  needs src/window.rs 'WindowKind::PopUp | WindowKind::AnchoredPopup(_) | WindowKind::Floating => {' \
    "creates floating windows with the nonactivating panel style"
  needs src/window.rs 'setHidesOnDeactivate: Bool::NO' \
    "keeps the floating player visible after deactivation"
  needs src/window.rs 'setBecomesKeyOnlyIfNeeded: Bool::YES' \
    "keeps the floating player from taking keyboard focus"
  needs Cargo.toml 'features = ["font-kit", "runtime_shaders"]' \
    "builds the Apple backend with fonts and runtime shaders"
  needs src/platform.rs 'let text_system = Arc::new(gpui_apple::AppleTextSystem::new());' \
    "uses Core Text to render Mac labels"
}

# --- The machinery -----------------------------------------------------------

fail() { echo "verify-vendor: $*" >&2; exit 1; }

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# Every regular file under $1, as repo-relative paths, sorted.
list_files() { ( cd "$1" && find . -type f | sed 's#^\./##' | LC_ALL=C sort ); }

# Whether the patched file `$1` of the current crate still contains the literal
# `$2`, which is what makes it do `$3`.
#
# Comments are stripped before the search. A hunk commented out with `//` still
# contains its literal, so a plain grep would have called it present - which is
# the opposite of what "present rather than merely commented" means. Only
# whole-line and trailing `//` comments are removed; a `//` inside a string
# literal would be a false negative here, and none of the strings in the table
# has one.
#
# `grep` without `-q`, deliberately: `-q` exits on the first match, and on a
# file the size of window.rs that closes the pipe while sed is still writing,
# so under `pipefail` the pipeline reports sed's SIGPIPE as a failure of a
# check that had in fact passed.
needs() { # file, literal string, what that string makes the file do
  sed 's|//.*$||' "$VENDOR/$1" | grep -F -- "$2" > /dev/null     || fail "$VENDOR/$1 no longer $3"
}

mode="${1:-}"

# `${var:?}` on everything this script removes: an empty variable stops the
# shell there rather than turning the path into the root of something.
root=$(mktemp -d)
trap 'rm -rf "${root:?}"' EXIT

# Whether the patch `$2`, applied to a copy of the upstream tree `$1`, gives
# the current crate's vendored tree byte for byte. Quiet, and it leaves `$1`
# alone: the check proper applies the patch in place and says what went wrong.
reproduces() { # upstream dir, absolute patch path, work dir
  local applied="$3/applied" f
  cp -R "$1" "$applied"
  (cd "$applied" && git -c core.autocrlf=false -c core.eol=lf apply -p2 "$2") 2> /dev/null ||
    return 1
  while IFS= read -r f; do
    cmp -s "$applied/$f" "$VENDOR/$f" || return 1
  done < "$3/up.files"
}

# Check, or with --update rewrite the patch of, the crate whose `crate_<key>`
# has just run.
verify_crate() {
  [ -d "$VENDOR" ] || fail "$VENDOR does not exist"

  # Derive the vendored version from the tree rather than trusting the
  # constant, so a bump is caught here rather than as a confusing checksum
  # failure.
  local version
  version=$(grep -m1 '^version = ' "$VENDOR/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')
  [ "$version" = "$EXPECT_VERSION" ] || fail \
    "$VENDOR is version $version but this script pins $EXPECT_VERSION.
     If the bump is deliberate: re-apply the patch to the new source, update
     EXPECT_VERSION and EXPECT_SHA256 in crate_$key, then run with --update."

  # Absolute, because the apply below runs with the extracted upstream as cwd.
  local patch_abs="$PWD/$PATCH"

  local work="$root/$key"
  mkdir -p "$work"

  local url="https://static.crates.io/crates/$CRATE/$CRATE-$version.crate"
  echo "verify-vendor: fetching $url"
  curl -fsSL "$url" -o "$work/crate.tar.gz" || fail "download failed"

  local got
  got=$(sha256 "$work/crate.tar.gz")
  [ "$got" = "$EXPECT_SHA256" ] || fail \
    "checksum mismatch for $CRATE-$version.crate
     expected $EXPECT_SHA256
     got      $got"

  tar -xzf "$work/crate.tar.gz" -C "$work"
  local upstream="$work/$CRATE-$version"
  [ -d "$upstream" ] || fail "unexpected archive layout"

  # .cargo-ok is a registry extraction marker, not part of the published crate.
  rm -f "${upstream:?}/.cargo-ok"
  # `${A[@]+"${A[@]}"}` throughout: an empty array is "unbound" to `set -u` in
  # bash before 4.4, and a crate with nothing removed or patched has two.
  local f
  for f in ${REMOVED[@]+"${REMOVED[@]}"}; do
    [ -e "$upstream/$f" ] || fail "REMOVED lists $f but upstream $CRATE does not have it"
    rm -f "${upstream:?}/${f:?}"
  done

  # Compare the two trees by hand rather than parsing `diff -rq` output: its
  # wording is not something to depend on, and `diff -ruN` would bake mtimes
  # into the patch - one side comes from the tarball, the other from the
  # checkout, so the patch would differ on every machine.
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

  for f in ${PATCHED[@]+"${PATCHED[@]}"}; do printf '%s\n' "$f"; done \
    | LC_ALL=C sort > "$work/expected"
  if ! diff -u "$work/expected" "$work/differing" > "$work/contentdrift" 2>&1; then
    echo "verify-vendor: the set of files in $VENDOR differing from upstream is not the expected set." >&2
    echo "               (-) expected to differ, (+) actually differs:" >&2
    sed 's/^/    /' "$work/contentdrift" >&2
    fail "unexpected edits to the vendored tree"
  fi

  if [ "$mode" = "--update" ]; then
    if [ "${#PATCHED[@]}" -eq 0 ]; then
      rm -f "${PATCH:?}"
      echo "verify-vendor: $CRATE is pristine; no patch to write"
      return
    fi
    # A patch that still reproduces the tree is left as it is. BSD and GNU
    # diff draw hunk boundaries differently, so rewriting a patch on the other
    # kind of machine would change its hunk headers and nothing it means, and
    # `--update` visits every crate, not only the one just edited.
    if [ -s "$PATCH" ] && reproduces "$upstream" "$patch_abs" "$work"; then
      echo "verify-vendor: $PATCH still reproduces $VENDOR; left as it is"
      return
    fi
    : > "$PATCH"
    for f in "${PATCHED[@]}"; do
      diff -u --label "a/$CRATE/$f" --label "b/$CRATE/$f" \
        "$upstream/$f" "$VENDOR/$f" >> "$PATCH" || true
    done
    echo "verify-vendor: wrote $PATCH ($(wc -l < "$PATCH" | tr -d ' ') lines)"
    return
  fi

  if [ "${#PATCHED[@]}" -eq 0 ]; then
    # Pristine: the content comparison above already proved the tree is
    # upstream's, byte for byte. A patch file with anything in it would be a
    # patch nobody applied.
    [ ! -s "$PATCH" ] || fail "$CRATE has no patched files but $PATCH is not empty"
    echo "verify-vendor: OK - $CRATE $version, pristine, ${#REMOVED[@]} assets removed"
    return
  fi

  [ -f "$PATCH" ] || fail "$PATCH is missing; run: scripts/verify-vendor.sh --update"

  # Apply the committed patch to the pristine upstream and require the result
  # to be the vendored tree, byte for byte.
  #
  # Deliberately not a text comparison of a freshly generated diff against the
  # committed one, which is what this did first and what CI killed: BSD diff on
  # macOS numbers its hunks differently from GNU diff, so a patch generated on
  # one could never match one regenerated on the other, and the check was
  # structurally incapable of passing on both legs. Applying asks the better
  # question anyway - whether the patch really does reconstruct the tree,
  # rather than whether some rendering of a diff of it looks familiar.
  # `-c core.autocrlf=false -c core.eol=lf`: `git apply` honours those settings
  # when it writes, and the Windows runner sets autocrlf=true globally. Without
  # them it applies the patch perfectly and then writes CRLF into files the
  # vendored tree stores as LF, so every patched file "differs" for a reason
  # that has nothing to do with the patch.
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

  # The patch is byte-for-byte what it should be; now check it still does its
  # job: every patched file still says it is patched, and every fix the table
  # names is still in it.
  for f in "${PATCHED[@]}"; do
    grep -q 'PERCH PATCH' "$VENDOR/$f" || fail "$VENDOR/$f has lost its PERCH PATCH marker"
  done
  "check_$key"

  echo "verify-vendor: OK - $CRATE $version + $PATCH, ${#PATCHED[@]} files patched, ${#REMOVED[@]} assets removed"
}

for key in "${CRATES[@]}"; do
  "crate_$key"
  verify_crate
done
