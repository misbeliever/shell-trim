# Releasing `trm`

## 1. Verify tests, formatting & lints

```sh
make fmt
make lint
make test
```

## 2. Bump version & update changelog

1. Bump `version` in `Cargo.toml`.
2. Run `cargo check` (or `make build`) so `Cargo.lock` picks up the new version.
3. Add a top section to `CHANGELOG.md` matching the exact tag name:

   ```markdown
   ## vX.Y.Z — Short Release Title
   - ...
   ```

   `.github/workflows/release.yml` extracts the matching `## vX.Y.Z` heading as the GitHub Release title and its body as the release notes (with GitHub's `Full Changelog` compare link appended underneath).

## 3. Commit, tag & push

```sh
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "Release vX.Y.Z: <short summary>"
git tag vX.Y.Z
git push origin main vX.Y.Z
```

Pushing the `v*` tag triggers `.github/workflows/release.yml`, which builds `macos-arm64`, `macos-x86_64`, and `linux-x86_64` tarballs + `.sha256` files and publishes the GitHub Release once all three builds succeed.

## 4. Update the Homebrew formula

After the release workflow finishes, fetch the published checksums and update `version` + the three `sha256` fields in `Formula/trm.rb`:

```sh
for f in trm-macos-arm64.tar.gz trm-macos-x86_64.tar.gz trm-linux-x86_64.tar.gz; do
  curl -fsSL "https://github.com/misbeliever/shell-trim/releases/download/vX.Y.Z/${f}.sha256"
done
```

Then commit and push `Formula/trm.rb` in this repo and mirror it to [`misbeliever/homebrew-tap`](https://github.com/misbeliever/homebrew-tap):

```sh
git add Formula/trm.rb
git commit -m "chore: update Homebrew formula for vX.Y.Z"
git push origin main

# Sync to misbeliever/homebrew-tap
git clone --depth 1 https://github.com/misbeliever/homebrew-tap.git /tmp/homebrew-tap
cp Formula/trm.rb /tmp/homebrew-tap/Formula/trm.rb
git -C /tmp/homebrew-tap add Formula/trm.rb
git -C /tmp/homebrew-tap commit -m "chore: update trm formula to vX.Y.Z"
git -C /tmp/homebrew-tap push origin main
rm -rf /tmp/homebrew-tap
```

## 5. (Optional) Reinstall locally

```sh
make install
```
