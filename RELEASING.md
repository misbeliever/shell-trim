# Releasing `trm`

## Steps

1. **Check build & tests**
   ```sh
   make fmt && make lint && make test
   ```
2. **Bump version & changelog**
   - Update `version` in `Cargo.toml` and run `cargo check` so `Cargo.lock` updates.
   - Add a top `## vX.Y.Z — Short Title` section to `CHANGELOG.md`.
3. **Commit, tag & push**
   ```sh
   git add Cargo.toml Cargo.lock CHANGELOG.md
   git commit -m "Release vX.Y.Z: <short summary>"
   git tag vX.Y.Z
   git push origin main vX.Y.Z
   ```
   `.github/workflows/release.yml` automatically:
   - builds `macos-arm64`, `macos-x86_64`, and `linux-x86_64` tarballs + `.sha256` files,
   - publishes the GitHub Release from the matching `CHANGELOG.md` section, and
   - updates `Formula/trm.rb` in [`misbeliever/homebrew-tap`](https://github.com/misbeliever/homebrew-tap).

4. *(Optional)* Reinstall locally: `make install`

## CI setup (already configured)

The `homebrew` job uses a read/write SSH deploy key (`HOMEBREW_TAP_DEPLOY_KEY` secret in this repo, public key on `misbeliever/homebrew-tap`). If it ever needs rotating:

```sh
gh-axi secret set HOMEBREW_TAP_DEPLOY_KEY < path/to/private_key
```
