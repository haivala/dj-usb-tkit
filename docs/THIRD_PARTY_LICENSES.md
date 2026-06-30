# Third-Party Licenses

This document is a redistribution-oriented license summary for repository code and release artifacts. It is not legal advice.

## Current licensing posture

- Project code is licensed as `MIT`.
- Default release artifacts do not bundle a Node runtime.
- Essentia is optional and downloaded in-app when enabled by the user.

## What this covers

This summary tracks obligations visible from:

- Rust dependency metadata
- JavaScript package metadata in this repository
- release packaging behavior in `scripts/release.sh`

## Redistribution notes

### Project code

Redistribution follows the obligations of the MIT license.

### Optional Essentia runtime

Essentia is not part of default bundled artifacts. If distribution policy changes to include Essentia assets directly, review additional license obligations for that distribution model.

### Node runtime

Default artifacts do not ship a bundled Node binary. Any distribution that includes Node must also include the appropriate notices for the shipped Node version.

### Frontend font assets

The frontend bundles the following fonts from Google Fonts under the SIL Open Font License 1.1:

- Outfit (`vanilla-ui/assets/fonts/google/Outfit-400.ttf`, `Outfit-500.ttf`, `Outfit-600.ttf`, `Outfit-700.ttf`)
- JetBrains Mono (`vanilla-ui/assets/fonts/google/JetBrainsMono-400.ttf`, `JetBrainsMono-500.ttf`)

Included notice files:

- `vanilla-ui/assets/fonts/LICENSES/OFL-1.1.txt`
- `vanilla-ui/assets/fonts/LICENSES/ATTRIBUTIONS.md`

### Rust/Tauri dependency graph

The graph is primarily permissive licenses, with some weak-copyleft components in the ecosystem. Keep dependency audits current for every release.

### Linux AppImage shared libraries

AppImage bundling can vary by build host. Keep a per-release inventory of bundled shared libraries and build-host details.

## Release maintenance checklist

1. Keep top-level license files and package metadata aligned.
2. Include notices for bundled non-source third-party components where required.
3. Keep this document updated as packaging or dependency policy changes.
